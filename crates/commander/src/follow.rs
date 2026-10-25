//! One-radio P25 Phase 1 control follower.
//!
//! Watch parks on a memory marked control. It measures every control in the
//! zone and keeps one only when that carrier is stronger than the others.
//! A sync blip on a dead channel does not count. After that choice it stays,
//! including through a sync loss. A clear voice grant is the tune. The voice
//! call ends on a terminator or a loss of Phase 1 sync, and the tune goes
//! back to the control frequency that was left. Encrypted grants are recorded
//! and are not tuned. No key is read.
//!
//! Every [`SCAN_PERIOD`] the selected radio's S-meter is read again. A louder
//! control replaces the one being held.
//!
//! The two roles are separate so a later radio can watch while another
//! follows voice. This type runs them one after the other on one radio.

/// Clear-mode algorithm id printed by the Phase 1 voice header.
pub const CLEAR_ALG: u16 = 0x80;

/// A grant is heard only when the air says it is clear and neither the
/// talkgroup nor the channel is marked encrypted. No key is stored.
pub fn hear_clear(alg: u16, talkgroup_encrypted: bool, channel_encrypted: bool) -> bool {
    alg == CLEAR_ALG && !talkgroup_encrypted && !channel_encrypted
}

/// How long a control that has signal is kept before the others are measured.
/// The decoder has no "scan now" event. This interval is the scan.
pub const SCAN_PERIOD: std::time::Duration = std::time::Duration::from_secs(3 * 60);

/// A carrier has to stand this far above the weakest control in the pass.
/// Equal readings are the noise floor. Those do not count as a signal.
pub const SIGNAL_MARGIN_DB: f64 = 6.0;

/// Wait after the radio is on a candidate before reading its S-meter.
/// The first report after a tune can still be the channel just left, and
/// the rig does not say when the new reading has settled.
pub const SCAN_DWELL: std::time::Duration = std::time::Duration::from_millis(1200);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Watch,
    Voice,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CcEvent {
    /// An identifier update was decoded, so a later channel number can
    /// become a frequency.
    Iden,
    Grant { hz: f64, tg: u32, alg: u16 },
    /// Voice-channel algorithm id. A value other than [`CLEAR_ALG`] ends
    /// the hop. Abbreviated unit-to-unit grants do not carry this bit.
    Alg(u16),
    /// P25 terminator.
    End,
    /// A decoded voice frame: Phase 1 `LDU1`/`LDU2`, Phase 2 `4V`/`2V`.
    /// This is the evidence that audio is on the air, not just a grant.
    Voice,
    /// Phase 1 frame sync is present.
    Sync,
    /// The decoder reported that sync is gone.
    SyncLost,
    /// An adjacent site's outbound control channel. `rfss` and `site` are the
    /// numbers printed on the advertisement. The follower does not tune this.
    Adjacent { hz: f64, rfss: u32, site: u32 },
    /// A radio named on this control channel. The follower does not tune it.
    Unit(UnitSight),
    /// A radio-to-radio (unit-to-unit) call from `src` to `tgt`. `hz` is the
    /// voice channel when the grant printed one. A grant with a frequency is
    /// also sent as [`CcEvent::Grant`] with the target as `tg`, so following
    /// works as before. Voice-channel link control lines carry no frequency.
    Direct {
        src: u32,
        tgt: u32,
        hz: Option<f64>,
        encrypted: bool,
    },
    /// One decoder search finished without a lock.
    ///
    /// `dsd-fme` prints `Sync: no sync` only after it has already seen a
    /// carrier. A control channel that never locks prints nothing, so the
    /// receive script counts one search of audio and sends this instead.
    Search,
}

/// One radio named by a grant, a registration, or a deregistration.
///
/// Absent fields were not on that line. A later line can fill them. Unit id
/// 0 is not a radio. No encryption key is stored.
#[derive(Clone, Debug, PartialEq)]
pub struct UnitSight {
    pub id: u32,
    pub last_tg: Option<u32>,
    pub peer: Option<u32>,
    pub emergency: bool,
    pub encrypted: bool,
    pub site: Option<u32>,
    pub wacn: Option<String>,
    pub sys: Option<String>,
    pub reg: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Heard {
    pub tg: u32,
    pub hz: f64,
    pub alg: u16,
}

#[derive(Clone, Debug)]
struct Pending {
    hz: f64,
    tg: u32,
}

/// What the radio should do with the control memories it was given.
#[derive(Clone, Debug)]
pub struct Follower {
    controls: Vec<f64>,
    index: usize,
    role: Role,
    iden: bool,
    synced: bool,
    /// Control frequency to restore when the voice call ends.
    return_hz: f64,
    pending: Option<Pending>,
    heard: Option<Heard>,
    status: String,
    /// Last frequency handed to the radio. Cleared when the target changes
    /// so the next event can try a tune that did not land.
    sent: Option<f64>,
    /// The search line after a tune can still be audio from the previous
    /// frequency. The first line only arms the hunt. The next one moves on.
    hunt_armed: bool,
    /// Set by the first control-channel sync. While set, sync loss and a
    /// quiet search do not walk to another memory.
    held: bool,
    /// When the next strength scan may start. `None` until the first lock.
    next_scan: Option<std::time::Instant>,
    /// Strength pass in progress. The home index stays put until it finishes.
    scan: Option<StrengthScan>,
}

/// One pass across the control list, in list order.
#[derive(Clone, Debug)]
struct StrengthScan {
    order: Vec<f64>,
    at: usize,
    /// Control index that was held when the pass started. A tie keeps it.
    home: usize,
    best_hz: f64,
    best_db: f64,
    worst_db: f64,
    have_best: bool,
    /// First instant this candidate was seen on the radio.
    on_freq: Option<std::time::Instant>,
}

impl Follower {
    pub fn new(controls: Vec<f64>) -> Self {
        let mut follower = Self {
            controls: Vec::new(),
            index: 0,
            role: Role::Watch,
            iden: false,
            synced: false,
            return_hz: 0.0,
            pending: None,
            heard: None,
            status: String::new(),
            sent: None,
            hunt_armed: false,
            held: false,
            next_scan: None,
            scan: None,
        };
        follower.set_controls(controls);
        follower
    }

    pub fn set_controls(&mut self, controls: Vec<f64>) {
        let current = self.control_hz();
        self.controls = controls.into_iter().filter(|hz| hz.is_finite() && *hz > 0.0).collect();
        if let Some(hz) = current {
            if let Some(index) = self.controls.iter().position(|item| (*item - hz).abs() < 1.0) {
                self.index = index;
                self.refresh_status();
                return;
            }
        }
        self.index = 0;
        self.role = Role::Watch;
        self.pending = None;
        self.iden = false;
        self.synced = false;
        self.sent = None;
        self.hunt_armed = false;
        self.held = false;
        self.next_scan = None;
        self.scan = None;
        self.return_hz = self.control_hz().unwrap_or(0.0);
        self.refresh_status();
    }

    /// True when `controls` is the same list this follower is already using.
    pub fn controls_match(&self, controls: &[f64]) -> bool {
        self.controls.len() == controls.len()
            && self
                .controls
                .iter()
                .zip(controls)
                .all(|(left, right)| (left - right).abs() < 1.0)
    }

    pub fn role(&self) -> Role {
        self.role
    }

    pub fn control_hz(&self) -> Option<f64> {
        self.controls.get(self.index).copied()
    }

    /// Control frequency to show. During a pass this is the candidate under
    /// the dial, not the memory the pass started on.
    pub fn shown_control_hz(&self) -> Option<f64> {
        if self.scan.is_some() {
            return self.target_hz();
        }
        self.control_hz()
    }

    /// Voice frequency of the grant being followed.
    pub fn voice_hz(&self) -> Option<f64> {
        self.pending.as_ref().map(|grant| grant.hz)
    }

    pub fn active_tg(&self) -> Option<u32> {
        self.pending.as_ref().map(|grant| grant.tg)
    }

    pub fn heard(&self) -> Option<&Heard> {
        self.heard.as_ref()
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn sent(&self) -> Option<f64> {
        self.sent
    }

    pub fn note_sent(&mut self, hz: f64) {
        self.sent = Some(hz);
        // The next search line may already have been counted on the
        // frequency this tune is leaving. Arm on that line. Hop on the next.
        self.hunt_armed = false;
        if self.pending.is_some() {
            self.role = Role::Voice;
            self.refresh_status();
        }
    }

    /// Where the radio should be. `None` when no control memory is marked.
    pub fn target_hz(&self) -> Option<f64> {
        if let Some(scan) = &self.scan {
            return scan.order.get(scan.at).copied();
        }
        if self.controls.is_empty() {
            return None;
        }
        if let Some(grant) = &self.pending {
            Some(grant.hz)
        } else {
            self.control_hz()
        }
    }

    /// Sample the S-meter while a scan is running, and start one when it is due.
    /// `on_frequency` is true when the radio is already on [`Self::target_hz`].
    /// Higher `strength_db` is a stronger signal.
    pub fn observe(&mut self, now: std::time::Instant, strength_db: f64, on_frequency: bool) {
        if self.scan.is_none() {
            self.maybe_start_scan(now);
            return;
        }
        if !on_frequency {
            if let Some(scan) = self.scan.as_mut() {
                scan.on_freq = None;
            }
            return;
        }
        let started = {
            let Some(scan) = self.scan.as_mut() else {
                return;
            };
            match scan.on_freq {
                Some(started) => started,
                None => {
                    scan.on_freq = Some(now);
                    return;
                }
            }
        };
        if now.saturating_duration_since(started) < SCAN_DWELL {
            return;
        }
        let prefer_home = self.held;
        let step = self.scan.as_mut().map(|scan| {
            let at = scan.at;
            if let Some(hz) = scan.order.get(at).copied() {
                if !scan.have_best {
                    scan.best_hz = hz;
                    scan.best_db = strength_db;
                    scan.worst_db = strength_db;
                    scan.have_best = true;
                } else {
                    let tie_home = prefer_home
                        && (strength_db - scan.best_db).abs() < 0.05
                        && at == scan.home;
                    if strength_db > scan.best_db || tie_home {
                        scan.best_hz = hz;
                        scan.best_db = strength_db;
                    }
                    if strength_db < scan.worst_db {
                        scan.worst_db = strength_db;
                    }
                }
            }
            (at + 1, scan.order.len())
        });
        let Some((next, order_len)) = step else {
            return;
        };
        if next >= order_len {
            self.finish_scan(now);
            return;
        }
        if let Some(scan) = self.scan.as_mut() {
            scan.at = next;
            scan.on_freq = None;
        }
        self.arm_candidate();
        self.status = format!("scanning {}/{}", next + 1, order_len);
    }

    fn maybe_start_scan(&mut self, now: std::time::Instant) {
        if self.role != Role::Watch || self.pending.is_some() || self.controls.len() <= 1 {
            return;
        }
        if self.held {
            if self.synced && self.next_scan.is_none() {
                self.next_scan = Some(now + SCAN_PERIOD);
            }
            let due = self.next_scan.is_some_and(|at| now >= at);
            if !due {
                return;
            }
        }
        // No confirmed carrier yet. Measure the list now instead of sitting
        // on the first memory.
        self.begin_scan();
    }

    fn begin_scan(&mut self) {
        let order = self.controls.clone();
        let first = order.first().copied().unwrap_or(0.0);
        let n = order.len();
        self.scan = Some(StrengthScan {
            order,
            at: 0,
            home: self.index,
            best_hz: 0.0,
            best_db: f64::MIN,
            worst_db: f64::MAX,
            have_best: false,
            on_freq: None,
        });
        if self.sent.map(|hz| (hz - first).abs() >= 1.0).unwrap_or(true) {
            self.sent = None;
        }
        self.status = format!("scanning 1/{n}");
    }

    /// The operator picked this control memory. Follow it instead of the one
    /// the scan started on. The next pass still has to find signal there.
    pub fn hold(&mut self, hz: f64, now: std::time::Instant) -> bool {
        if !hz.is_finite() || hz <= 0.0 {
            return false;
        }
        let Some(index) = self.controls.iter().position(|item| (*item - hz).abs() < 1.0) else {
            return false;
        };
        self.scan = None;
        self.index = index;
        self.return_hz = self.control_hz().unwrap_or(hz);
        self.pending = None;
        self.role = Role::Watch;
        self.iden = false;
        self.synced = false;
        self.sent = None;
        self.hunt_armed = false;
        self.held = false;
        self.next_scan = Some(now);
        self.status = "scanning".into();
        true
    }

    fn arm_candidate(&mut self) {
        let Some(hz) = self.scan.as_ref().and_then(|scan| scan.order.get(scan.at).copied()) else {
            return;
        };
        if self.sent.map(|sent| (sent - hz).abs() >= 1.0).unwrap_or(true) {
            self.sent = None;
        }
    }

    fn finish_scan(&mut self, now: std::time::Instant) {
        let Some(scan) = self.scan.take() else {
            return;
        };
        let choice = if scan.have_best && scan.best_db - scan.worst_db >= SIGNAL_MARGIN_DB {
            Some(scan.best_hz)
        } else {
            None
        };
        let Some(best_hz) = choice else {
            if self.held {
                // This pass did not find a stronger carrier. Keep the one
                // that already qualified.
                self.next_scan = Some(now + SCAN_PERIOD);
                self.status = "locked".into();
                return;
            }
            self.held = false;
            self.begin_scan();
            return;
        };
        let Some(index) = self
            .controls
            .iter()
            .position(|hz| (*hz - best_hz).abs() < 1.0)
        else {
            if !self.held {
                self.begin_scan();
            }
            return;
        };
        let same = index == self.index && self.sent.is_some_and(|hz| (hz - best_hz).abs() < 1.0);
        self.index = index;
        self.return_hz = self.control_hz().unwrap_or(best_hz);
        self.held = true;
        self.pending = None;
        self.role = Role::Watch;
        self.hunt_armed = false;
        self.next_scan = Some(now + SCAN_PERIOD);
        if !same {
            self.iden = false;
            self.synced = false;
            self.sent = None;
        }
        self.status = "locked".into();
    }

    /// Speaker audio belongs on a voice channel. The control channel stays quiet.
    pub fn play_voice(&self) -> bool {
        self.role == Role::Voice && self.pending.is_some()
    }

    pub fn push(&mut self, event: CcEvent) {
        match event {
            CcEvent::Iden => {
                self.iden = true;
                if self.scan.is_none()
                    && !self.held
                    && self.role == Role::Watch
                    && self.pending.is_none()
                {
                    self.status = "watching".into();
                }
            }
            CcEvent::Grant { hz, tg, alg } => self.on_grant(hz, tg, alg),
            CcEvent::Alg(alg) => {
                if self.role == Role::Voice && alg != CLEAR_ALG {
                    self.heard = self.heard.as_ref().map(|heard| Heard {
                        alg,
                        ..heard.clone()
                    });
                    self.status = format!("encrypted talkgroup {}", self.active_tg().unwrap_or(0));
                    self.back_to_control();
                }
            }
            CcEvent::End => {
                // A control channel sends terminators between trunking blocks.
                // Only a terminator after the radio has been sent to the voice
                // frequency ends the hop.
                if self.role == Role::Voice {
                    self.back_to_control();
                }
            }
            CcEvent::Sync => {
                // Sync on a dead carrier is not a signal. Only a pass that
                // found a stronger channel, or one already chosen, may hold.
                if self.scan.is_none() && self.held && self.role == Role::Watch {
                    self.synced = true;
                    self.status = "locked".into();
                }
            }
            CcEvent::SyncLost => self.on_sync_lost(),
            CcEvent::Search => self.on_search(),
            CcEvent::Adjacent { .. }
            | CcEvent::Unit(_)
            | CcEvent::Direct { .. }
            | CcEvent::Voice => {}
        }
    }

    fn on_grant(&mut self, hz: f64, tg: u32, alg: u16) {
        if !hz.is_finite() || hz <= 0.0 {
            return;
        }
        // A strength pass is tuning the list on purpose. A grant in that
        // window is from whichever candidate is under the dial.
        if self.scan.is_some() {
            return;
        }
        // A grant heard while the radio is already on a voice channel is
        // from that channel's link control, not a new control-channel hop.
        if self.role == Role::Voice {
            return;
        }
        self.heard = Some(Heard { tg, hz, alg });
        if alg != CLEAR_ALG {
            self.status = format!("encrypted talkgroup {tg}");
            return;
        }
        if !self.iden {
            self.status = "waiting for a channel map".into();
            return;
        }
        if self.pending.is_none() {
            self.return_hz = self.control_hz().unwrap_or(0.0);
        }
        let replace = self.pending.as_ref().is_some_and(|grant| (grant.hz - hz).abs() >= 1.0);
        self.pending = Some(Pending { hz, tg });
        if replace {
            self.sent = None;
        }
        self.status = format!("voice talkgroup {tg}");
    }

    fn on_sync_lost(&mut self) {
        if self.role == Role::Voice {
            self.back_to_control();
            return;
        }
        // A grant is already chosen. Losing the control channel does not
        // drop that frequency, and it does not walk to the next site.
        if self.pending.is_some() || self.scan.is_some() {
            return;
        }
        // The first lock is the channel to keep. A later dropout stays put
        // until the strength scan chooses a different memory.
        if self.held {
            self.synced = false;
            return;
        }
        // A miss before the first lock is still a miss. Staying there left
        // the radio on a control channel that never decoded.
        if self.role == Role::Watch {
            self.advance_control();
        }
    }

    fn on_search(&mut self) {
        if self.role != Role::Watch
            || self.pending.is_some()
            || self.synced
            || self.held
            || self.scan.is_some()
        {
            return;
        }
        // The tune has not landed. A search from the old frequency is not
        // evidence about this one.
        if self.sent.is_none() {
            return;
        }
        if !self.hunt_armed {
            self.hunt_armed = true;
            return;
        }
        self.advance_control();
    }

    fn back_to_control(&mut self) {
        self.pending = None;
        self.role = Role::Watch;
        self.synced = false;
        self.hunt_armed = false;
        if let Some(index) = self
            .controls
            .iter()
            .position(|hz| (*hz - self.return_hz).abs() < 1.0)
        {
            self.index = index;
        }
        self.sent = None;
        self.refresh_status();
    }

    fn advance_control(&mut self) {
        self.hunt_armed = false;
        if self.controls.len() <= 1 {
            // The only flagged control did not lock. Leave the tune where
            // it is. Clearing `sent` would retune the same frequency.
            self.synced = false;
            self.status = "no sync".into();
            return;
        }
        self.index = (self.index + 1) % self.controls.len();
        self.return_hz = self.control_hz().unwrap_or(0.0);
        self.iden = false;
        self.synced = false;
        self.pending = None;
        self.role = Role::Watch;
        self.sent = None;
        self.status = "next control channel".into();
    }

    fn refresh_status(&mut self) {
        if self.controls.is_empty() {
            self.status = "no control channel".into();
            return;
        }
        if let Some(scan) = &self.scan {
            let n = scan.order.len().max(1);
            self.status = format!("scanning {}/{}", scan.at + 1, n);
            return;
        }
        if self.role == Role::Voice {
            if let Some(tg) = self.active_tg() {
                self.status = format!("voice talkgroup {tg}");
                return;
            }
        }
        if self.status.is_empty() || self.status == "no control channel" {
            self.status = "watching".into();
        }
        if self.held && self.role == Role::Watch && self.pending.is_none() {
            if !self.status.starts_with("encrypted") {
                self.status = "locked".into();
            }
            return;
        }
        if self.pending.is_none() && self.role == Role::Watch && !self.status.starts_with("encrypted")
        {
            self.status = "watching".into();
        }
    }
}

/// Turns one `dsd-fme` stderr stream into [`CcEvent`]s.
///
/// The lines are the ones printed by the installed decoder
/// (`p25p2_vpdu.c`, `p25_frequency.c`, `dsd_frame.c`, `dsd_frame_sync.c`).
/// A line that is not one of those is ignored. Color codes are stripped.
#[derive(Clone, Debug, Default)]
pub struct DsdFeed {
    open: Option<OpenGrant>,
    /// An adjacent-site advertisement whose control frequency has not been
    /// read yet. The next `Frequency` line is that site's outbound channel.
    adjacent: Option<OpenAdjacent>,
    /// The direct call last named on a voice channel. Link control repeats
    /// it every superframe, so only a change is reported.
    voice_direct: Option<(u32, u32)>,
}

#[derive(Clone, Debug)]
struct OpenAdjacent {
    rfss: u32,
    site: u32,
    extended: bool,
    skip: bool,
    took: bool,
}

#[derive(Clone, Debug)]
struct OpenGrant {
    tg: Option<u32>,
    source: Option<u32>,
    target: Option<u32>,
    encrypted: bool,
    emergency: bool,
    unit: bool,
    /// The radio ids on this grant were already emitted.
    reported: bool,
}

impl DsdFeed {
    pub fn push_line(&mut self, raw: &str) -> Vec<CcEvent> {
        let line = strip_ansi(raw);
        let line = line.trim().to_string();
        if line.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        if line.contains("hamspot: search") {
            out.push(CcEvent::Search);
        }
        if line.contains("Identifier Update") {
            out.push(CcEvent::Iden);
        }
        if line.contains("Sync: no sync") {
            out.push(CcEvent::SyncLost);
        } else if line.contains("Sync: +P25p1") || line.contains("Sync: -P25p1") {
            out.push(CcEvent::Sync);
        }
        if is_tdu(&line) {
            out.push(CcEvent::End);
            self.voice_direct = None;
        }
        if is_voice_frame(&line) {
            out.push(CcEvent::Voice);
        }
        if let Some(alg) = alg_id(&line) {
            out.push(CcEvent::Alg(alg));
        }
        out.extend(unit_admin_events(&line));
        if let Some((src, tgt)) = direct_voice(&line) {
            if self.voice_direct != Some((src, tgt)) {
                self.voice_direct = Some((src, tgt));
                push_unit(&mut out, src, None, Some(tgt), false, false);
                if tgt != src {
                    push_unit(&mut out, tgt, None, Some(src), false, false);
                }
                out.push(CcEvent::Direct {
                    src,
                    tgt,
                    hz: None,
                    encrypted: line.contains("Encrypted"),
                });
            }
        }
        if is_grant_header(&line) {
            // A grant starts a different message. An advertisement that never
            // printed its frequency must not take this grant's frequency.
            if self.adjacent.as_ref().is_some_and(|adj| !adj.took) {
                self.adjacent = None;
            }
            self.open = Some(OpenGrant {
                tg: talkgroup_on(&line),
                source: None,
                target: None,
                encrypted: line.contains("Encrypted"),
                emergency: line.contains("Emergency"),
                unit: line.contains("Unit to Unit"),
                reported: false,
            });
        }
        if line.contains("Adjacent Status Broadcast") {
            self.adjacent = Some(OpenAdjacent {
                rfss: 0,
                site: 0,
                extended: line.contains("Extended"),
                skip: false,
                took: false,
            });
        }
        if let Some(adj) = self.adjacent.as_mut() {
            if let Some(rfss) = labeled_number(&line, "RFSS") {
                adj.rfss = rfss;
            }
            if let Some(site) = labeled_number(&line, "SITE").or_else(|| labeled_number(&line, "Site"))
            {
                adj.site = site;
            }
            if line.contains("Failure Condition") || line.contains("Conventional") {
                adj.skip = true;
            }
        }
        let adjacent_frequency = self.take_adjacent_frequency(&line, &mut out);
        if !adjacent_frequency && !is_unit_admin(&line) {
            if let Some(open) = self.open.as_mut() {
                if open.tg.is_none() {
                    open.tg = talkgroup_on(&line);
                }
                if open.unit {
                    if open.source.is_none() {
                        open.source = dec_after(&line, "SRC: ");
                    }
                    if open.target.is_none() {
                        open.target = dec_after(&line, "TGT: ");
                    }
                } else if open.source.is_none() {
                    open.source = source_on(&line);
                }
                if svc_encrypted(&line) {
                    open.encrypted = true;
                }
                if svc_emergency(&line) {
                    open.emergency = true;
                }
            }
            self.emit_open_units(&mut out);
            if self.open.is_some() && line.contains("Base Frequency Not Found") {
                // The source was already recorded. There is no voice frequency.
                if let Some(grant) = self.open.take() {
                    push_direct(&mut out, &grant, None);
                }
            } else if let Some(hz) = frequency_hz(&line) {
                if let Some(grant) = self.open.take() {
                    if hz > 0.0 {
                        push_direct(&mut out, &grant, Some(hz));
                    }
                    if let Some(tg) = grant.tg {
                        if hz > 0.0 {
                            out.push(CcEvent::Grant {
                                hz,
                                tg,
                                alg: if grant.encrypted { 0 } else { CLEAR_ALG },
                            });
                        }
                    }
                }
            }
        }
        out
    }

    /// Emit each radio on the open grant once. A missing base frequency still
    /// counts: the source was printed before that line.
    fn emit_open_units(&mut self, out: &mut Vec<CcEvent>) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if open.reported {
            return;
        }
        let emergency = open.emergency;
        let encrypted = open.encrypted;
        if open.unit {
            let (Some(src), Some(tgt)) = (open.source, open.target) else {
                return;
            };
            open.reported = true;
            push_unit(out, src, None, Some(tgt), emergency, encrypted);
            if tgt != src {
                push_unit(out, tgt, None, Some(src), emergency, encrypted);
            }
            return;
        }
        let Some(src) = open.source else {
            return;
        };
        open.reported = true;
        push_unit(out, src, open.tg, None, emergency, encrypted);
    }

    /// The first frequency after an adjacent-site header is that site's
    /// outbound control channel. An extended advertisement prints the uplink
    /// next, and that frequency is not stored. `true` means this line was
    /// the advertisement's frequency, so a voice grant must not take it.
    fn take_adjacent_frequency(&mut self, line: &str, out: &mut Vec<CcEvent>) -> bool {
        let Some(adj) = self.adjacent.clone() else {
            return false;
        };
        if line.contains("Base Frequency Not Found") {
            // An extended advertisement prints the uplink next. Keep the
            // record so that frequency is discarded with the failed one.
            if !adj.took && adj.extended {
                self.adjacent = Some(OpenAdjacent {
                    took: true,
                    skip: true,
                    ..adj
                });
            } else {
                self.adjacent = None;
            }
            return true;
        }
        let Some(hz) = frequency_hz(line) else {
            return false;
        };
        if !adj.took {
            if !adj.skip && hz > 0.0 {
                out.push(CcEvent::Adjacent {
                    hz,
                    rfss: adj.rfss,
                    site: adj.site,
                });
            }
            if adj.extended {
                self.adjacent = Some(OpenAdjacent { took: true, ..adj });
            } else {
                self.adjacent = None;
            }
        } else {
            self.adjacent = None;
        }
        true
    }
}

/// Turns DMR Tier III stderr into the same events the P25 follower already
/// applies. Connect Plus, Capacity Plus, and Hytera lines are ignored.
/// The logical-channel map stays inside `dsd-fme`; this feed only believes a
/// `Frequency:` line, and only after a `Chan_Freq` announcement has armed it.
#[derive(Clone, Debug, Default)]
pub struct DmrFeed {
    open: Option<DmrOpen>,
    /// A data grant's frequency line is not a voice channel.
    ignore_freq: bool,
}

#[derive(Clone, Debug)]
struct DmrOpen {
    tg: Option<u32>,
    source: Option<u32>,
    emergency: bool,
    reported: bool,
}

impl DmrFeed {
    pub fn push_line(&mut self, raw: &str) -> Vec<CcEvent> {
        let line = strip_ansi(raw);
        let line = line.trim().to_string();
        if line.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        if line.contains("hamspot: search") {
            out.push(CcEvent::Search);
        }
        if line.contains("Sync: no sync") {
            out.push(CcEvent::SyncLost);
        } else if line.contains("Sync: +DMR") || line.contains("Sync: -DMR") {
            out.push(CcEvent::Sync);
        }
        if line.contains("(Chan_Freq)") {
            out.push(CcEvent::Iden);
            self.open = None;
        }
        if is_dmr_data_grant(&line) {
            self.open = None;
            self.ignore_freq = true;
            return out;
        }
        if is_dmr_voice_grant(&line) {
            self.ignore_freq = false;
            self.open = Some(DmrOpen {
                tg: None,
                source: None,
                emergency: line.contains("Emergency"),
                reported: false,
            });
        }
        if line.contains("Invalid Channel") {
            self.open = None;
            return out;
        }
        if let Some(open) = self.open.as_mut() {
            if open.tg.is_none() {
                open.tg = dec_after(&line, "Target: ");
            }
            if open.source.is_none() {
                open.source = dec_after(&line, "Source: ");
            }
            if line.contains("Emergency") {
                open.emergency = true;
            }
            if let (Some(tg), Some(src)) = (open.tg, open.source) {
                if !open.reported && tg != 0 {
                    open.reported = true;
                    push_unit(&mut out, src, Some(tg), None, open.emergency, false);
                }
            }
        }
        if line.contains("Frequency Not Found") {
            self.open = None;
            self.ignore_freq = false;
            return out;
        }
        if let Some(hz) = dmr_frequency_hz(&line) {
            if self.ignore_freq {
                self.ignore_freq = false;
                return out;
            }
            if let Some(open) = self.open.take() {
                if let Some(tg) = open.tg {
                    if tg != 0 && hz > 0.0 {
                        out.push(CcEvent::Grant {
                            hz,
                            tg,
                            alg: CLEAR_ALG,
                        });
                    }
                }
            }
        }
        // `Clear (P_CLEAR)` is not an end. The slot decision is printed only
        // while dsd-fme trunk-tune is on, and this pipe leaves that off.
        // Encryption is a later voice-channel line. A value other than the
        // clear id sends the follower back. No key id is kept.
        if dmr_encrypted(&line) {
            out.push(CcEvent::Alg(0));
        }
        out
    }
}

fn is_dmr_voice_grant(line: &str) -> bool {
    if line.contains("Connect Plus") || line.contains("Data Channel Grant") {
        return false;
    }
    line.contains("(TV_GRANT)")
        || line.contains("(BTV_GRANT)")
        || line.contains("(PV_GRANT)")
        || line.contains("(PV_GRANT_DX)")
}

fn is_dmr_data_grant(line: &str) -> bool {
    line.contains("Data Channel Grant")
}

/// Turns NXDN Type-C and Type-D stderr into the same events the P25 follower
/// already applies. A channel number is not a frequency. Only a
/// `Frequency [` or `DFA Frequency [` line in megahertz can grant, and only
/// after a clear voice assignment on an earlier line of this process.
/// `Custom DFA` and a missing import do not tune. No key is stored.
#[derive(Clone, Debug, Default)]
pub struct NxdnFeed {
    open: Option<NxdnOpen>,
}

#[derive(Clone, Debug)]
struct NxdnOpen {
    tg: u32,
    encrypted: bool,
}

impl NxdnFeed {
    pub fn push_line(&mut self, raw: &str) -> Vec<CcEvent> {
        let line = strip_ansi(raw);
        let line = line.trim().to_string();
        if line.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        if line.contains("hamspot: search") {
            out.push(CcEvent::Search);
        }
        if line.contains("Sync: no sync") {
            out.push(CcEvent::SyncLost);
        } else if line.contains("Sync:")
            && (line.contains("NXDN48") || line.contains("NXDN96") || line.contains("IDAS D"))
        {
            out.push(CcEvent::Sync);
        }
        if nxdn_release(&line) {
            self.open = None;
            out.push(CcEvent::End);
            return out;
        }
        if line.contains("Channel not found in import file")
            || line.contains("Custom DFA Settings")
        {
            self.open = None;
            return out;
        }
        if line.contains("Data Call Assignment") {
            self.open = None;
            return out;
        }
        if let Some(open) = self.open.as_mut() {
            if nxdn_encrypted(&line) {
                open.encrypted = true;
            }
        } else if nxdn_encrypted(&line) {
            // Cipher text with no assignment still marks a voice call the
            // radio has already joined. The follower leaves that channel.
            out.push(CcEvent::Alg(0));
        }
        if line.contains("Channel Update") {
            if line.contains("Termination") || nxdn_channel(&line) == Some(31) {
                self.open = None;
                out.push(CcEvent::End);
                return out;
            }
            if line.contains("Group Call") || line.contains("Private Call") {
                if let Some(tg) = dec_after(&line, "TGT: ") {
                    if tg != 0 {
                        self.open = Some(NxdnOpen {
                            tg,
                            encrypted: nxdn_encrypted(&line),
                        });
                    }
                }
            }
        } else if line.contains("Src=") && line.contains("Dst/TG=") {
            if let Some(tg) = dec_after(&line, "Dst/TG=") {
                if tg != 0 {
                    self.open = Some(NxdnOpen {
                        tg,
                        encrypted: nxdn_encrypted(&line),
                    });
                }
            }
        }
        if let Some(hz) = frequency_hz(&line) {
            if let Some(open) = self.open.take() {
                let alg = if open.encrypted { 0 } else { CLEAR_ALG };
                if alg == CLEAR_ALG {
                    // The printed megahertz is the map. The follower will not
                    // tune a grant until an identifier update has been seen.
                    out.push(CcEvent::Iden);
                }
                out.push(CcEvent::Grant {
                    hz,
                    tg: open.tg,
                    alg,
                });
            }
        }
        out
    }
}

fn nxdn_channel(line: &str) -> Option<u32> {
    dec_after(line, "CH: ")
}

fn nxdn_release(line: &str) -> bool {
    line.contains("Transmission Release")
        || line.contains("TX_REL")
        || line.split_whitespace().any(|word| word == "DISC")
}

fn nxdn_encrypted(line: &str) -> bool {
    if line.contains("No Encrypted") {
        return false;
    }
    nxdn_enc_word(line) || line.contains("Scrambler") || line.contains("Unknown Cipher Type")
}

/// ` Enc` on a call, not the letters inside `Encoded`.
fn nxdn_enc_word(line: &str) -> bool {
    let mut rest = line;
    while let Some(pos) = rest.find(" Enc") {
        let after = &rest[pos + 4..];
        if !after.starts_with("od") {
            return true;
        }
        rest = &rest[pos + 4..];
    }
    line.contains(" DES") || line.contains(" AES")
}

/// One decoder line folded into the scanner glass. Fields that this line
/// does not carry stay as they were. A CRC or FEC error is not an identity.
/// Nothing here is a tune.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AirIdentity {
    pub src: String,
    pub dst: String,
    pub tg: Option<u32>,
    pub code_label: String,
    pub code: String,
    pub rpt1: String,
    pub rpt2: String,
    pub encrypted: bool,
}

pub fn note_identity(id: &mut AirIdentity, raw: &str) {
    let line = strip_ansi(raw);
    let line = line.trim();
    if line.is_empty() || line.contains("(CRC ERR)") || line.contains("(FEC ERR)") {
        return;
    }
    if let Some(value) = text_field(line, "RPT 1: ", &["RPT 2:", "DST:", "SRC:"]) {
        id.rpt1 = value;
    }
    if let Some(value) = text_field(line, "RPT 2: ", &["RPT 1:", "DST:", "SRC:"]) {
        id.rpt2 = value;
    }
    if let Some(value) = text_field(line, "DST RID: ", &["SRC RID:", "SRC:", "DST:"]) {
        id.dst = value;
    } else if let Some(value) = text_field(line, "DST: ", &["SRC:", "RPT 1:", "RPT 2:"]) {
        id.dst = value;
    }
    if let Some(value) = text_field(line, "SRC RID: ", &["DST RID:", "DST:", "SRC:"]) {
        id.src = value;
    } else if let Some(value) = text_field(line, "SRC: ", &["DST:", "RPT 1:", "RPT 2:"]) {
        id.src = value;
    }
    if let Some(n) = dec_after(line, "Src=") {
        if n != 0 {
            id.src = n.to_string();
        }
    } else if let Some(n) = dec_after(line, "SRC=") {
        if n != 0 {
            id.src = n.to_string();
        }
    }
    if let Some(n) = dec_after(line, "Dst/TG=") {
        if n != 0 {
            id.tg = Some(n);
        }
    } else if let Some(n) = dec_after(line, "TGT=") {
        if n != 0 {
            id.tg = Some(n);
        }
    } else if let Some(n) = dec_after(line, "TGT: ") {
        if n != 0 {
            id.tg = Some(n);
        }
    } else if let Some(n) = bounded_dec(line, "TG=") {
        if n != 0 {
            id.tg = Some(n);
        }
    } else if let Some(n) = dec_after(line, "TGT [") {
        if n != 0 {
            id.tg = Some(n);
        }
    } else if let Some(n) = dec_after(line, "Group: ").or_else(|| dec_after(line, "Target: ")) {
        if n != 0 {
            id.tg = Some(n);
        }
    }
    if id.tg.is_none() {
        if let Some(afs) = edacs_afs(line) {
            id.dst = afs;
        }
    }
    if let Some(n) = dec_after(line, "Source: ") {
        if n != 0 {
            id.src = n.to_string();
        }
    }
    if let Some(n) = color_code(line) {
        id.code_label = "CC".into();
        id.code = format!("{n:02}");
    } else if let Some(n) = dec_after(line, "Channel Code=") {
        id.code_label = "Code".into();
        id.code = format!("{n:02}");
    } else if let Some(n) = dec_after(line, "RAN ") {
        id.code_label = "RAN".into();
        id.code = format!("{n:02}");
    } else if line.contains("SQL ") {
        if let Some(n) = dec_after(line, "CODE: ") {
            id.code_label = "SQL".into();
            id.code = format!("{n:03}");
        }
    } else if let Some(n) = dec_after(line, "CAN: ") {
        id.code_label = "CAN".into();
        id.code = n.to_string();
    }
    if nxdn_encrypted(line) || (line.contains("Encrypted") && !line.contains("No Encrypted")) {
        id.encrypted = true;
    }
    if let Some(snr) = freedv_sync_snr(line) {
        id.code_label = "SNR".into();
        id.code = snr;
    }
    if let Some(name) = drm_service(line) {
        id.dst = name;
    }
    if let Some(snr) = drm_snr(line) {
        id.code_label = "SNR".into();
        id.code = snr;
    }
}

/// Digits after `marker` when the character before it is not a slash.
/// `Dst/TG=` must not be read as a dPMR `TG=`.
fn bounded_dec(line: &str, marker: &str) -> Option<u32> {
    let start = line.find(marker)?;
    if start > 0 && line.as_bytes()[start - 1] == b'/' {
        return None;
    }
    dec_after(line, marker)
}

/// FreeDV verbose level 1: `sync: 1` and `snr: -1.50 dB`. An unsynced line
/// is not a measurement.
fn freedv_sync_snr(line: &str) -> Option<String> {
    let sync = dec_after(line, "sync: ")?;
    if sync == 0 {
        return None;
    }
    let start = line.find("snr: ")? + "snr: ".len();
    signed_decimal(line[start..].trim_start())
}

/// DecDRM status: `SNR 12.4 dB` on the reception line.
fn drm_snr(line: &str) -> Option<String> {
    let start = line.find(" SNR ")? + " SNR ".len();
    signed_decimal(line[start..].trim_start())
}

/// DecDRM `service 0 id D0D000 BBC World Service ...`. A missing label is skipped.
fn drm_service(line: &str) -> Option<String> {
    let start = line.find("service ")?;
    let rest = &line[start..];
    let id_at = rest.find(" id ")? + " id ".len();
    let after_id = rest[id_at..].trim_start();
    let hex_end = after_id.find(' ')?;
    let name = after_id[hex_end..].trim();
    if name.is_empty() || name.starts_with("(no label yet)") {
        None
    } else {
        Some(name.to_string())
    }
}

fn signed_decimal(rest: &str) -> Option<String> {
    let mut chars = rest.chars().peekable();
    let mut out = String::new();
    if matches!(chars.peek(), Some('+') | Some('-')) {
        out.push(chars.next()?);
    }
    let mut digits = 0usize;
    let mut dot = false;
    for ch in chars {
        if ch.is_ascii_digit() {
            digits += 1;
            out.push(ch);
        } else if ch == '.' && !dot {
            dot = true;
            out.push(ch);
        } else {
            break;
        }
    }
    if digits == 0 { None } else { Some(out) }
}

/// `AFS [003] [02-041]` from an EDACS line that has no decimal target.
fn edacs_afs(line: &str) -> Option<String> {
    let start = line.find("AFS [")? + "AFS ".len();
    let rest = line[start..].trim_start();
    let first = rest.find(']')?;
    let second_open = rest[first + 1..].find('[')? + first + 1;
    let second = rest[second_open..].find(']')? + second_open;
    let text = rest[..=second].trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn color_code(line: &str) -> Option<u32> {
    let start = line.find("Color Code=")? + "Color Code=".len();
    let rest = line[start..].trim_start();
    if rest.starts_with("XX") {
        return None;
    }
    let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

fn text_field(line: &str, key: &str, next_keys: &[&str]) -> Option<String> {
    let start = line.find(key)? + key.len();
    let rest = &line[start..];
    let mut end = rest.len();
    for next in next_keys {
        if let Some(pos) = rest.find(next) {
            end = end.min(pos);
        }
    }
    let value = rest[..end].trim();
    if value.is_empty() || value.contains("CRC ERR") {
        None
    } else {
        Some(value.to_string())
    }
}

/// What the scanner glass may take from a decoded identity.
///
/// `None` means the caller keeps the trunk text it already has. An active
/// talkgroup owns the headline, the talkgroup row, and the detail. The
/// access code is still returned so the bottom row can show it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlassRows {
    pub headline: Option<String>,
    pub tgid: Option<u32>,
    pub detail: Option<String>,
    pub facts: Option<String>,
    pub code_line: Option<String>,
    pub encrypted: bool,
}

pub fn glass_rows(active_tg: Option<u32>, ident: &AirIdentity) -> GlassRows {
    let mut rows = GlassRows::default();
    if active_tg.is_none() {
        if !ident.src.is_empty() {
            rows.headline = Some(ident.src.clone());
        }
        rows.tgid = ident.tg;
        if !ident.dst.is_empty() {
            rows.detail = Some(ident.dst.clone());
        }
        rows.encrypted = ident.encrypted;
        if !ident.rpt1.is_empty() || !ident.rpt2.is_empty() {
            let rpt1 = if ident.rpt1.is_empty() {
                "—"
            } else {
                ident.rpt1.as_str()
            };
            let rpt2 = if ident.rpt2.is_empty() {
                "—"
            } else {
                ident.rpt2.as_str()
            };
            rows.facts = Some(format!("RPT1 {rpt1}   RPT2 {rpt2}"));
        }
    }
    if !ident.code.is_empty() {
        rows.code_line = Some(format!("{} {}", ident.code_label, ident.code));
    }
    rows
}

fn dmr_encrypted(line: &str) -> bool {
    if line.contains("No Encrypted") {
        return false;
    }
    if line.contains("DMR PI") && line.contains("Encryption") {
        return true;
    }
    line.contains("Encrypted")
}

/// `Frequency: 453.125000 MHz`. `RX Freq:` is the channel map, not a grant.
/// `TS:` on the grant line is the timeslot of this same carrier, so it is
/// not read as a frequency.
fn dmr_frequency_hz(line: &str) -> Option<f64> {
    if line.contains("Frequency Not Found") || line.contains("RX Freq:") {
        return None;
    }
    let start = line.find("Frequency:")? + "Frequency:".len();
    let rest = line[start..].trim_start();
    let token: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect();
    if token.is_empty() {
        return None;
    }
    let mhz: f64 = token.parse().ok()?;
    if !mhz.is_finite() || mhz <= 0.0 {
        return None;
    }
    Some(mhz * 1_000_000.0)
}

fn is_tdu(line: &str) -> bool {
    // ` TDU` and ` TDULC` are the terminator frames. A longer word that
    // merely contains those letters is not.
    let trimmed = line.trim();
    trimmed == "TDU" || trimmed == "TDULC" || trimmed.ends_with(" TDU") || trimmed.ends_with(" TDULC")
}

fn alg_id(line: &str) -> Option<u16> {
    let marker = "ALG ID: 0x";
    let start = line.find(marker)?;
    let rest = &line[start + marker.len()..];
    let hex: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    if hex.is_empty() {
        return None;
    }
    u16::from_str_radix(&hex, 16).ok()
}

/// Algorithm and key id printed by dsd-fme. Clear (`0x80`) and key id 0 are
/// not a request. `alg_id` is unchanged: a clear line is still an Alg event.
pub fn key_on_line(line: &str) -> Option<(u16, u16)> {
    let alg = hex_field(line, &["ALG ID:", "ALG:"])?;
    let kid = hex_field(line, &["KEY ID:", "KEY:"])?;
    if alg == CLEAR_ALG || kid == 0 {
        return None;
    }
    Some((alg, kid))
}

/// First marker in `markers` wins when two start at the same column, so
/// `KEY ID:` is not read as `KEY:`.
fn hex_field(line: &str, markers: &[&str]) -> Option<u16> {
    let mut at: Option<(usize, &str)> = None;
    for marker in markers {
        if let Some(pos) = line.find(marker) {
            if at.is_none_or(|(have, _)| pos < have) {
                at = Some((pos, marker));
            }
        }
    }
    let (pos, marker) = at?;
    let mut rest = line[pos + marker.len()..].trim_start();
    if let Some(stripped) = rest
        .strip_prefix("0x")
        .or_else(|| rest.strip_prefix("0X"))
    {
        rest = stripped;
    }
    let hex: String = rest.chars().take_while(|ch| ch.is_ascii_hexdigit()).collect();
    if hex.is_empty() || hex.len() > 4 {
        return None;
    }
    u16::from_str_radix(&hex, 16).ok()
}

fn talkgroup_on(line: &str) -> Option<u32> {
    if let Some(tg) = between(line, "Group [", "]") {
        return tg.parse().ok();
    }
    if let Some(tg) = between(line, "TGT: ", ";") {
        return tg.trim().parse().ok();
    }
    None
}

fn svc_encrypted(line: &str) -> bool {
    svc_bit(line, 0x40)
}

fn svc_emergency(line: &str) -> bool {
    svc_bit(line, 0x80)
}

fn svc_bit(line: &str, bit: u16) -> bool {
    let Some(hex) = between(line, "SVC [", "]") else {
        return false;
    };
    let Ok(value) = u16::from_str_radix(hex, 16) else {
        return false;
    };
    value & bit != 0
}

fn is_grant_header(line: &str) -> bool {
    // "Connect Plus Group Voice Channel Grant" is DMR. This follower is P25.
    if line.contains("Connect Plus") {
        return false;
    }
    // "Unit to Unit Channel Grant" is the abbreviated TSBK/vPDU form, with
    // " Update" or " Extended" on the same line. "Unit to Unit Voice Channel
    // Grant Update - Extended" is the Phase 1 multi-block form.
    line.contains("Group Voice Channel Grant")
        || line.contains("Unit to Unit Channel Grant")
        || line.contains("Unit to Unit Voice Channel Grant")
}

/// A direct call on an open unit-to-unit grant. Nothing when either radio
/// id was not printed.
fn push_direct(out: &mut Vec<CcEvent>, grant: &OpenGrant, hz: Option<f64>) {
    if !grant.unit {
        return;
    }
    if let (Some(src), Some(tgt)) = (grant.source, grant.target) {
        if src != 0 && tgt != 0 {
            out.push(CcEvent::Direct {
                src,
                tgt,
                hz,
                encrypted: grant.encrypted,
            });
        }
    }
}

/// `dsd-fme` frame info for a voice frame (`dsd_frame.c`, `p25p2_frame.c`).
pub fn is_voice_frame(line: &str) -> bool {
    if line.contains("Ignoring LDU2") {
        return false;
    }
    line.contains(" LDU1") || line.contains(" LDU2") || line.contains(" 4V ") || line.ends_with(" 4V")
        || line.contains(" 2V")
}

/// Source and target of a direct call named on a voice channel.
///
/// Phase 1 link control: ` Unit to Unit Voice Channel User - Target 456 Source 123`.
/// Phase 2 MAC: ` VCH 0 - TGT: 456; SRC 123;  Unit to Unit Voice`.
fn direct_voice(line: &str) -> Option<(u32, u32)> {
    if line.contains("Unit to Unit Voice Channel User") {
        let tgt = dec_after(line, " - Target ")?;
        let src = dec_after(line, " Source ")?;
        return (src != 0 && tgt != 0).then_some((src, tgt));
    }
    if line.contains("VCH ") && line.contains("Unit to Unit Voice") {
        let tgt = dec_after(line, "TGT: ")?;
        let src = dec_after(line, "SRC ")?;
        return (src != 0 && tgt != 0).then_some((src, tgt));
    }
    None
}

/// Readout status for a followed call. `live` is a grant being followed;
/// `voice` means voice frames were decoded within the hang.
pub fn call_status(status: String, live: bool, encrypted: bool, voice: bool) -> String {
    if !live {
        return status;
    }
    if encrypted {
        format!("{status} · ENC")
    } else if voice {
        status
    } else {
        format!("{status} · NO AUDIO")
    }
}

/// One decoder-log line for a direct call, as in
/// `DIRECT 123 → 456 · 853.5000 MHz · encrypted`. `src` and `tgt` are
/// already labelled by the caller.
pub fn direct_summary(src: &str, tgt: &str, hz: Option<f64>, encrypted: bool) -> String {
    let mut text = format!("DIRECT {src} → {tgt}");
    if let Some(hz) = hz {
        text.push_str(&format!(" · {:.4} MHz", hz / 1_000_000.0));
    } else {
        text.push_str(" · on voice");
    }
    if encrypted {
        text.push_str(" · encrypted");
    }
    text
}

/// The newest `cap` decoder-log lines to show, and how many housekeeping
/// lines in `lines` were left out. `show_all` keeps every line.
pub fn visible_lines(lines: &[String], show_all: bool, cap: usize) -> (Vec<&str>, usize) {
    let mut hidden = 0;
    let mut shown: Vec<&str> = Vec::new();
    for line in lines {
        if !show_all && is_housekeeping(line) {
            hidden += 1;
        } else {
            shown.push(line);
        }
    }
    if shown.len() > cap {
        shown.drain(0..shown.len() - cap);
    }
    (shown, hidden)
}

/// Control-channel housekeeping that says nothing about who is talking:
/// registrations, deregistrations, authentication, affiliation queries,
/// status and vendor acknowledgements. The decoder log hides these by
/// default. Their facts still reach the radio list through [`DsdFeed`].
pub fn is_housekeeping(line: &str) -> bool {
    const MARKERS: [&str; 14] = [
        "Unit Registration",
        "Unit Deregistration",
        "REG_ACCEPT",
        "REG_FAIL",
        "REG_DENY",
        "REG_REFUSED",
        "Authentication Demand",
        "Authentication FNE Response",
        "Unit Authentication Command",
        "Group Affiliation Query",
        "Status Query",
        "Status Update",
        "Acknowledge Response",
        "Queued Response",
    ];
    MARKERS.iter().any(|marker| line.contains(marker))
}

fn is_unit_admin(line: &str) -> bool {
    line.contains("Unit Registration") || line.contains("Unit Deregistration")
}

fn source_on(line: &str) -> Option<u32> {
    between(line, "Source [", "]")?.parse().ok()
}

/// Decimal after `marker`. `SRC: ` does not match `SRC_ID: `.
fn dec_after(line: &str, marker: &str) -> Option<u32> {
    let start = line.find(marker)? + marker.len();
    let rest = line[start..].trim_start();
    let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

fn hex_after(line: &str, marker: &str) -> Option<String> {
    let start = line.find(marker)? + marker.len();
    let rest = line[start..].trim_start();
    let rest = rest.strip_prefix('[').unwrap_or(rest);
    let hex: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_hexdigit())
        .collect();
    if hex.is_empty() {
        None
    } else {
        Some(hex.to_ascii_uppercase())
    }
}

fn hex_u32(line: &str, marker: &str) -> Option<u32> {
    let hex = hex_after(line, marker)?;
    u32::from_str_radix(&hex, 16).ok()
}

fn reg_result(line: &str) -> Option<String> {
    for token in ["REG_ACCEPT", "REG_REFUSED", "REG_DENY", "REG_FAIL"] {
        if line.contains(token) {
            return Some(token.to_string());
        }
    }
    None
}

fn push_unit(
    out: &mut Vec<CcEvent>,
    id: u32,
    last_tg: Option<u32>,
    peer: Option<u32>,
    emergency: bool,
    encrypted: bool,
) {
    if id == 0 {
        return;
    }
    out.push(CcEvent::Unit(UnitSight {
        id,
        last_tg,
        peer,
        emergency,
        encrypted,
        site: None,
        wacn: None,
        sys: None,
        reg: None,
    }));
}

fn push_admin(out: &mut Vec<CcEvent>, id: u32, line: &str, with_result: bool) {
    if id == 0 {
        return;
    }
    out.push(CcEvent::Unit(UnitSight {
        id,
        last_tg: None,
        peer: None,
        emergency: false,
        encrypted: false,
        site: hex_u32(line, "SITE: "),
        wacn: hex_after(line, "WACN: "),
        sys: hex_after(line, "SYS: "),
        reg: if with_result { reg_result(line) } else { None },
    }));
}

/// Registration and deregistration lines. A registration command names both
/// radios and does not make one the other's peer.
fn unit_admin_events(line: &str) -> Vec<CcEvent> {
    let mut out = Vec::new();
    if line.contains("Unit Registration Response") {
        if let Some(id) = dec_after(line, "SRC: ") {
            push_admin(&mut out, id, line, true);
        }
        return out;
    }
    if line.contains("Unit Deregistration") {
        if let Some(id) = dec_after(line, "SRC: ") {
            push_admin(&mut out, id, line, false);
        }
        return out;
    }
    if line.contains("Unit Registration") {
        let src = dec_after(line, "SRC: ");
        let tgt = dec_after(line, "TGT: ");
        if let Some(id) = src {
            push_admin(&mut out, id, line, false);
        }
        if let Some(id) = tgt {
            if src != Some(id) {
                push_admin(&mut out, id, line, false);
            }
        }
    }
    out
}

/// Number printed after `label`, as in `RFSS[003]`, `SITE [012]`, or `Site 12`.
/// `SYSID` is not `SITE`.
fn labeled_number(line: &str, label: &str) -> Option<u32> {
    let lower_line = line.to_ascii_lowercase();
    let lower_label = label.to_ascii_lowercase();
    let mut search = 0;
    while let Some(rel) = lower_line[search..].find(&lower_label) {
        let start = search + rel;
        let end = start + lower_label.len();
        let before = lower_line.as_bytes().get(start.wrapping_sub(1)).copied();
        let after = lower_line.as_bytes().get(end).copied();
        let before_ok = start == 0 || !before.is_some_and(|byte| byte.is_ascii_alphanumeric());
        let after_ok = !after.is_some_and(|byte| byte.is_ascii_alphanumeric());
        if before_ok && after_ok {
            return number_after(&line[end..]);
        }
        search = start + 1;
        if search >= lower_line.len() {
            break;
        }
    }
    None
}

/// Digits after optional spaces, an `ID` word, and one `[`.
fn number_after(rest: &str) -> Option<u32> {
    let rest = rest.trim_start();
    let rest = match rest.get(..2) {
        Some(word) if word.eq_ignore_ascii_case("id") => {
            let after = &rest[2..];
            if after
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphanumeric())
            {
                return None;
            }
            after.trim_start()
        }
        _ => rest,
    };
    let rest = rest.strip_prefix('[').unwrap_or(rest).trim_start();
    let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

fn frequency_hz(line: &str) -> Option<f64> {
    let mhz = between(line, "Frequency [", "] MHz")?;
    let mhz: f64 = mhz.parse().ok()?;
    if !mhz.is_finite() || mhz <= 0.0 {
        return None;
    }
    Some(mhz * 1_000_000.0)
}

fn between<'a>(line: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = line.find(open)? + open.len();
    let rest = &line[start..];
    let end = rest.find(close)?;
    Some(&rest[..end])
}

/// Decoder text with color codes removed. The log and the parser share it.
pub fn plain_line(line: &str) -> String {
    strip_ansi(line)
}

/// Identity from a Phase 1 sync line. Other lines, including a registration,
/// are not this site's banner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SiteBanner {
    pub wacn: String,
    pub sys: String,
    pub nac: String,
    pub rfss: String,
    pub site: String,
}

/// `Sync: +P25p1 WACN: BEE00; SYS: 6BD; NAC/CC: 6B5; RFSS: 001; Site: 037`.
pub fn site_banner(line: &str) -> Option<SiteBanner> {
    let line = plain_line(line);
    if !line.contains("Sync:") || !line.contains("P25p1") || !line.contains("WACN:") {
        return None;
    }
    let banner = SiteBanner {
        wacn: hex_after(&line, "WACN: ").unwrap_or_default(),
        sys: hex_after(&line, "SYS: ").unwrap_or_default(),
        nac: hex_after(&line, "NAC/CC: ").unwrap_or_default(),
        rfss: hex_after(&line, "RFSS: ").unwrap_or_default(),
        site: hex_after(&line, "Site: ")
            .or_else(|| hex_after(&line, "SITE: "))
            .unwrap_or_default(),
    };
    if banner.wacn.is_empty() && banner.sys.is_empty() {
        None
    } else {
        Some(banner)
    }
}

/// dsd-fme `-X` value. Phase 2 voice is muted, and stderr says
/// `P2 Missing Parameters`, until WACN, system id, and NAC are all set.
/// The voice decoder does not hear the control channel that carries them.
pub fn p2_parameter_word(banner: &SiteBanner) -> Option<String> {
    let wacn = u64::from_str_radix(banner.wacn.trim(), 16).ok()?;
    let sys = u64::from_str_radix(banner.sys.trim(), 16).ok()?;
    let nac = u64::from_str_radix(banner.nac.trim(), 16).ok()?;
    if wacn == 0 || sys == 0 || nac == 0 {
        return None;
    }
    if wacn > 0xF_FFFF || sys > 0xFFF || nac > 0xFFF {
        return None;
    }
    Some(format!("{:X}", (wacn << 24) | (sys << 12) | nac))
}

fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            for next in chars.by_ref() {
                if next == 'm' {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clear(hz: f64, tg: u32) -> CcEvent {
        CcEvent::Grant {
            hz,
            tg,
            alg: CLEAR_ALG,
        }
    }

    #[test]
    fn a_phase1_sync_line_names_the_site() {
        let line = "Sync: +P25p1 WACN: BEE00; SYS: 6BD; NAC/CC: 6B5; RFSS: 001; Site: 037; TSBK";
        let banner = site_banner(line).expect("banner");
        assert_eq!(banner.wacn, "BEE00");
        assert_eq!(banner.sys, "6BD");
        assert_eq!(banner.nac, "6B5");
        assert_eq!(banner.rfss, "001");
        assert_eq!(banner.site, "037");
        assert!(site_banner("Unit Registration Response - WACN: BEE00; SYS: 2A1; SRC: 99").is_none());
        assert_eq!(
            p2_parameter_word(&banner).as_deref(),
            Some("BEE006BD6B5")
        );
        let mut missing = banner.clone();
        missing.nac.clear();
        assert!(p2_parameter_word(&missing).is_none());
    }

    #[test]
    fn grant_before_an_identifier_update_does_not_tune() {
        let mut follow = Follower::new(vec![851_012_500.0]);
        follow.push(clear(852_000_000.0, 10));
        assert_eq!(follow.target_hz(), Some(851_012_500.0));
        assert_eq!(follow.status(), "waiting for a channel map");
    }

    #[test]
    fn a_later_clear_grant_replaces_the_one_not_yet_tuned() {
        let mut follow = Follower::new(vec![851_012_500.0]);
        follow.push(CcEvent::Iden);
        follow.push(clear(852_000_000.0, 10));
        follow.push(clear(853_000_000.0, 11));
        assert_eq!(follow.target_hz(), Some(853_000_000.0));
        assert_eq!(follow.control_hz(), Some(851_012_500.0));
        assert_eq!(follow.role(), Role::Watch);
    }

    #[test]
    fn an_encrypted_grant_does_not_tune() {
        let mut follow = Follower::new(vec![851_012_500.0]);
        follow.push(CcEvent::Iden);
        follow.push(CcEvent::Grant {
            hz: 852_000_000.0,
            tg: 12,
            alg: 0,
        });
        assert_eq!(follow.target_hz(), Some(851_012_500.0));
        assert!(follow.status().contains("encrypted"));
        assert_eq!(follow.voice_hz(), None);
        assert!(!hear_clear(0, false, false));
        assert!(!hear_clear(CLEAR_ALG, true, false));
        assert!(!hear_clear(CLEAR_ALG, false, true));
        assert!(hear_clear(CLEAR_ALG, false, false));
    }

    #[test]
    fn a_terminator_returns_to_the_same_control_channel() {
        let mut follow = Follower::new(vec![851_012_500.0, 852_475_000.0]);
        follow.push(CcEvent::Iden);
        follow.push(clear(853_500_000.0, 20));
        follow.note_sent(853_500_000.0);
        assert_eq!(follow.role(), Role::Voice);
        follow.push(CcEvent::End);
        assert_eq!(follow.target_hz(), Some(851_012_500.0));
        assert_eq!(follow.role(), Role::Watch);
        assert_eq!(follow.sent(), None);
    }

    #[test]
    fn voice_sync_loss_returns_to_the_same_control_channel() {
        let mut follow = Follower::new(vec![851_012_500.0]);
        follow.push(CcEvent::Iden);
        follow.push(clear(853_500_000.0, 20));
        follow.note_sent(853_500_000.0);
        follow.push(CcEvent::SyncLost);
        assert_eq!(follow.target_hz(), Some(851_012_500.0));
        assert_eq!(follow.voice_hz(), None);
    }

    #[test]
    fn control_sync_loss_stays_after_a_lock() {
        let start = std::time::Instant::now();
        let home = 851_012_500.0;
        let other = 852_475_000.0;
        let mut follow = Follower::new(vec![home, other]);
        scan_levels(&mut follow, start, &[-20.0, -40.0]);
        assert_eq!(follow.control_hz(), Some(home));
        assert_eq!(follow.status(), "locked");
        follow.push(CcEvent::SyncLost);
        follow.push(CcEvent::Search);
        follow.push(CcEvent::Search);
        assert_eq!(follow.control_hz(), Some(home));
        assert_eq!(follow.target_hz(), Some(home));
        follow.push(CcEvent::Sync);
        assert_eq!(follow.control_hz(), Some(home));
        assert_eq!(follow.status(), "locked");
    }

    #[test]
    fn sync_on_a_dead_channel_does_not_stick() {
        let mut follow = Follower::new(vec![851_012_500.0, 852_475_000.0]);
        follow.note_sent(851_012_500.0);
        follow.push(CcEvent::Sync);
        follow.push(CcEvent::SyncLost);
        assert_eq!(follow.control_hz(), Some(852_475_000.0));
        assert_ne!(follow.status(), "locked");
    }

    #[test]
    fn sync_loss_before_the_first_sync_moves_on() {
        let mut follow = Follower::new(vec![851_012_500.0, 852_475_000.0]);
        follow.push(CcEvent::SyncLost);
        assert_eq!(follow.control_hz(), Some(852_475_000.0));
    }

    #[test]
    fn a_quiet_search_moves_to_the_next_control() {
        let mut follow = Follower::new(vec![851_012_500.0, 852_475_000.0]);
        follow.note_sent(851_012_500.0);
        follow.push(CcEvent::Search);
        assert_eq!(follow.control_hz(), Some(851_012_500.0));
        follow.push(CcEvent::Search);
        assert_eq!(follow.control_hz(), Some(852_475_000.0));
        assert_eq!(follow.target_hz(), Some(852_475_000.0));
        assert_eq!(follow.sent(), None);
        assert_eq!(follow.status(), "next control channel");
    }

    #[test]
    fn a_search_while_locked_stays() {
        let start = std::time::Instant::now();
        let home = 851_012_500.0;
        let other = 852_475_000.0;
        let mut follow = Follower::new(vec![home, other]);
        scan_levels(&mut follow, start, &[-18.0, -40.0]);
        assert_eq!(follow.status(), "locked");
        follow.note_sent(home);
        follow.push(CcEvent::Search);
        follow.push(CcEvent::Search);
        assert_eq!(follow.control_hz(), Some(home));
        assert_eq!(follow.status(), "locked");
    }

    #[test]
    fn a_search_during_voice_stays_on_the_grant() {
        let mut follow = Follower::new(vec![851_012_500.0, 852_475_000.0]);
        follow.push(CcEvent::Iden);
        follow.push(clear(853_500_000.0, 20));
        follow.note_sent(853_500_000.0);
        follow.push(CcEvent::Search);
        follow.push(CcEvent::Search);
        assert_eq!(follow.target_hz(), Some(853_500_000.0));
        assert_eq!(follow.control_hz(), Some(851_012_500.0));
    }

    #[test]
    fn one_control_channel_stays_when_the_search_misses() {
        let mut follow = Follower::new(vec![851_012_500.0]);
        follow.note_sent(851_012_500.0);
        follow.push(CcEvent::Search);
        follow.push(CcEvent::Search);
        assert_eq!(follow.control_hz(), Some(851_012_500.0));
        assert_eq!(follow.sent(), Some(851_012_500.0));
        assert_eq!(follow.status(), "no sync");
    }

    #[test]
    fn housekeeping_lines_are_recognised_and_calls_are_not() {
        for line in [
            " Unit Registration Response - SITE: 025 SRC_ID: 1725 SRC: 610191 REG_ACCEPT;",
            " Unit Registration Response - WACN: BEE00; SYS: 6BD; SRC: 99 REG_DENY; - Extended;",
            " Unit Registration - SRC: 3; TGT: 8;",
            " Unit Deregistration Acknowlegement - WACN: BEE00; SYS: 6BD; SRC: 5",
            " Authentication Demand;",
            " Authentication FNE Response;",
            " MFID 90 (Moto) Acknowledge Response: ",
            " MFID 90 (Moto) Queued Response: ",
            " Group Affiliation Query",
            " Status Update",
        ] {
            assert!(is_housekeeping(line), "{line}");
        }
        for line in [
            " Group Voice Channel Grant",
            " Unit to Unit Channel Grant",
            "  CHAN: 0125; SRC: 1; TGT: 42;",
            "  Frequency [851.987500] MHz",
            "DIRECT 1 → 42 · 851.9875 MHz",
            "Sync: +P25p1  LDU1  ",
        ] {
            assert!(!is_housekeeping(line), "{line}");
        }
    }

    #[test]
    fn the_log_hides_housekeeping_unless_asked_and_counts_it() {
        let lines: Vec<String> = [
            " Group Voice Channel Grant",
            " Unit Registration Response - SITE: 025 SRC_ID: 1 SRC: 2 REG_ACCEPT;",
            " Unit Deregistration Acknowlegement - WACN: BEE00; SYS: 6BD; SRC: 5",
            "  Frequency [851.987500] MHz",
            "DIRECT 1 → 42 · on voice",
        ]
        .iter()
        .map(|line| line.to_string())
        .collect();
        let (shown, hidden) = visible_lines(&lines, false, 80);
        assert_eq!(hidden, 2);
        assert_eq!(
            shown,
            vec![" Group Voice Channel Grant", "  Frequency [851.987500] MHz", "DIRECT 1 → 42 · on voice"]
        );
        let (all, none) = visible_lines(&lines, true, 80);
        assert_eq!((all.len(), none), (5, 0));
        let (tail, _) = visible_lines(&lines, false, 2);
        assert_eq!(tail, vec!["  Frequency [851.987500] MHz", "DIRECT 1 → 42 · on voice"]);
    }

    #[test]
    fn a_grant_alone_does_not_read_as_live_voice() {
        let s = || "voice talkgroup 10035".to_string();
        assert_eq!(call_status(s(), true, false, true), "voice talkgroup 10035");
        assert_eq!(call_status(s(), true, false, false), "voice talkgroup 10035 · NO AUDIO");
        assert_eq!(call_status(s(), true, true, true), "voice talkgroup 10035 · ENC");
        assert_eq!(call_status("watching".into(), false, false, false), "watching");
    }

    #[test]
    fn voice_frames_are_evidence_and_other_frames_are_not() {
        assert!(is_voice_frame("Sync: +P25p1  LDU1  e:"));
        assert!(is_voice_frame("Sync: +P25p1  LDU2  e:"));
        assert!(is_voice_frame("Sync: +P25p2  4V 1 "));
        assert!(is_voice_frame("Sync: +P25p2  2V"));
        assert!(!is_voice_frame(" Ignoring LDU2 not preceeded by LDU1"));
        assert!(!is_voice_frame("Sync: +P25p1  TSBK"));
        assert!(!is_voice_frame(" Group Voice Channel Grant"));
        let mut feed = DsdFeed::default();
        assert!(feed.push_line("Sync: +P25p1  LDU1  ").contains(&CcEvent::Voice));
    }

    #[test]
    fn direct_calls_in_every_printed_form_name_both_radios() {
        // Abbreviated / update / extended vPDU (p25p2_vpdu.c).
        let mut feed = DsdFeed::default();
        let events = lines_of(
            &mut feed,
            &[
                " Unit to Unit Channel Grant Update",
                "  CHAN: 0125; SRC: 610191; TGT: 610096; ",
                "  Frequency [851.987500] MHz",
            ],
        );
        assert!(events.contains(&CcEvent::Direct {
            src: 610191,
            tgt: 610096,
            hz: Some(851_987_500.0),
            encrypted: false,
        }));
        // Phase 1 multi-block extended (p25p1_pdu_trunking.c).
        let mut feed = DsdFeed::default();
        let events = lines_of(
            &mut feed,
            &[
                "  Encrypted Unit to Unit Voice Channel Grant Update - Extended",
                "  SVC: 40; CHAN-T: 0125; CHAN-R: 0000; SRC: 123; TGT: 456; FULL SRC: 00BEE006-00000123; FULL TGT: 00BEE006-00000456;",
                "  Frequency [852.500000] MHz",
            ],
        );
        assert!(events.contains(&CcEvent::Direct {
            src: 123,
            tgt: 456,
            hz: Some(852_500_000.0),
            encrypted: true,
        }));
        assert!(events.contains(&peers_enc(123, 456)));
        // A grant without a channel map still names the call.
        let mut feed = DsdFeed::default();
        let events = lines_of(
            &mut feed,
            &[
                " Unit to Unit Channel Grant",
                "  CHAN: 0125; SRC: 7; TGT: 9; ",
                "  Base Frequency Not Found - Iden [1]",
            ],
        );
        assert!(events.contains(&CcEvent::Direct { src: 7, tgt: 9, hz: None, encrypted: false }));
        // Voice-channel link control repeats; it is reported once per call.
        let mut feed = DsdFeed::default();
        let events = lines_of(
            &mut feed,
            &[
                " Unit to Unit Voice Channel User - Target 456 Source 123",
                " Unit to Unit Voice Channel User - Target 456 Source 123",
                " TDU",
                " VCH 0 - TGT: 456; SRC 123;  Unit to Unit Voice",
            ],
        );
        let directs = events
            .iter()
            .filter(|event| matches!(event, CcEvent::Direct { src: 123, tgt: 456, hz: None, .. }))
            .count();
        assert_eq!(directs, 2);
        assert_eq!(
            direct_summary("123", "456 (Engine 4)", Some(851_987_500.0), true),
            "DIRECT 123 → 456 (Engine 4) · 851.9875 MHz · encrypted"
        );
        assert_eq!(direct_summary("1", "2", None, false), "DIRECT 1 → 2 · on voice");
    }

    fn peers_enc(id: u32, peer: u32) -> CcEvent {
        CcEvent::Unit(UnitSight {
            id,
            last_tg: None,
            peer: Some(peer),
            emergency: false,
            encrypted: true,
            site: None,
            wacn: None,
            sys: None,
            reg: None,
        })
    }

    #[test]
    fn decoder_lines_become_events() {
        let mut feed = DsdFeed::default();
        let mut events = Vec::new();
        for line in [
            "\u{1b}[33m Identifier Update (8.3.1.23)",
            " Group Voice Channel Grant",
            "  SVC [00] CHAN [0123] Group [100] Source [1]",
            "  Base Frequency Not Found - Iden [1]",
            " Encrypted Group Voice Channel Grant",
            "  SVC [40] CHAN [0124] Group [200] Source [2]",
            "  Frequency [852.000000] MHz",
            " Unit to Unit Channel Grant",
            "  CHAN: 0125; SRC: 1; TGT: 42;",
            "  Frequency [853.500000] MHz",
            "Sync: +P25p1 ",
            " TDU",
            "Sync: no sync",
            " ALG ID: 0x80 KEY ID: 0x0000",
            "hamspot: search",
        ] {
            events.extend(feed.push_line(line));
        }
        assert_eq!(
            events,
            vec![
                CcEvent::Iden,
                radio(1, Some(100), false),
                radio(2, Some(200), true),
                CcEvent::Grant {
                    hz: 852_000_000.0,
                    tg: 200,
                    alg: 0,
                },
                peers(1, 42),
                peers(42, 1),
                CcEvent::Direct {
                    src: 1,
                    tgt: 42,
                    hz: Some(853_500_000.0),
                    encrypted: false,
                },
                CcEvent::Grant {
                    hz: 853_500_000.0,
                    tg: 42,
                    alg: CLEAR_ALG,
                },
                CcEvent::Sync,
                CcEvent::End,
                CcEvent::SyncLost,
                CcEvent::Alg(CLEAR_ALG),
                CcEvent::Search,
            ]
        );
    }

    fn radio(id: u32, tg: Option<u32>, encrypted: bool) -> CcEvent {
        CcEvent::Unit(UnitSight {
            id,
            last_tg: tg,
            peer: None,
            emergency: false,
            encrypted,
            site: None,
            wacn: None,
            sys: None,
            reg: None,
        })
    }

    fn peers(id: u32, peer: u32) -> CcEvent {
        CcEvent::Unit(UnitSight {
            id,
            last_tg: None,
            peer: Some(peer),
            emergency: false,
            encrypted: false,
            site: None,
            wacn: None,
            sys: None,
            reg: None,
        })
    }

    fn lines_of(feed: &mut DsdFeed, lines: &[&str]) -> Vec<CcEvent> {
        let mut events = Vec::new();
        for line in lines {
            events.extend(feed.push_line(line));
        }
        events
    }

    #[test]
    fn an_abbreviated_adjacent_site_is_its_outbound_channel() {
        let mut feed = DsdFeed::default();
        let events = lines_of(
            &mut feed,
            &[
                " Adjacent Status Broadcast - Abbreviated",
                "  LRA [01] RFSS[003] SITE [027] SYSID [0AB] CHAN-T [0123] SSC [04]",
                "   Up to Date (Correct) Valid RFSS Connection Active",
                "  Frequency [858.962500] MHz",
                " Group Voice Channel Grant",
                "  SVC [00] CHAN [0124] Group [100] Source [1]",
                "  Frequency [859.012500] MHz",
            ],
        );
        assert_eq!(
            events,
            vec![
                CcEvent::Adjacent {
                    hz: 858_962_500.0,
                    rfss: 3,
                    site: 27,
                },
                radio(1, Some(100), false),
                CcEvent::Grant {
                    hz: 859_012_500.0,
                    tg: 100,
                    alg: CLEAR_ALG,
                },
            ]
        );
    }

    #[test]
    fn an_extended_adjacent_site_keeps_only_the_outbound_channel() {
        let mut feed = DsdFeed::default();
        let events = lines_of(
            &mut feed,
            &[
                " Adjacent Status Broadcast - Extended",
                "  LRA [01] CFVA [2] RFSS[001] SITE [012] SYSID [2A1]",
                "  CHAN-T [0456] CHAN-R [0457] SSC [04] WACN [BEE00]",
                "   Last Known",
                "  Frequency [858.962500] MHz",
                "  Frequency [803.962500] MHz",
            ],
        );
        assert_eq!(
            events,
            vec![CcEvent::Adjacent {
                hz: 858_962_500.0,
                rfss: 1,
                site: 12,
            }]
        );
    }

    #[test]
    fn a_missing_base_or_a_failed_site_is_not_stored() {
        let mut feed = DsdFeed::default();
        let missing = lines_of(
            &mut feed,
            &[
                " Adjacent Status Broadcast - Extended",
                "  LRA [01] RFSS[004] SITE [008] SYSID [111] CHAN-T [0001] CHAN-R [0002]",
                "   Up to Date (Correct)",
                "  Base Frequency Not Found - Iden [1]",
                "    or Channel not found in import file",
                "  Frequency [803.100000] MHz",
            ],
        );
        assert!(missing.is_empty());
        let failed = lines_of(
            &mut feed,
            &[
                " Adjacent Status Broadcast - Abbreviated",
                "  LRA [01] RFSS[005] SITE [009] SYSID [111] CHAN-T [0003] SSC [04]",
                "   Failure Condition Last Known",
                "  Frequency [859.100000] MHz",
                " Adjacent Status Broadcast - Abbreviated",
                "  RFSS ID [006] SITE [010]",
                "   Conventional Up to Date (Correct)",
                "  Frequency [859.200000] MHz",
            ],
        );
        assert!(failed.is_empty());
    }

    #[test]
    fn a_link_control_adjacent_line_does_not_steal_the_next_grant() {
        let mut feed = DsdFeed::default();
        let events = lines_of(
            &mut feed,
            &[
                " Adjacent Site Status Broadcast",
                " Adjacent Site Status (LCASBX) - RFSS 1 Site 27 CH 0123",
                " Secondary Control Channel Broadcast",
                " Group Voice Channel Grant",
                "  SVC [00] CHAN [0123] Group [9] Source [1]",
                "  Frequency [851.012500] MHz",
            ],
        );
        assert_eq!(
            events,
            vec![
                radio(1, Some(9), false),
                CcEvent::Grant {
                    hz: 851_012_500.0,
                    tg: 9,
                    alg: CLEAR_ALG,
                },
            ]
        );
    }

    #[test]
    fn a_grant_update_without_a_source_adds_no_radio() {
        let mut feed = DsdFeed::default();
        let events = lines_of(
            &mut feed,
            &[
                " Group Voice Channel Grant Update",
                "  SVC [00] CHAN [0123] Group [100]",
                "  Frequency [852.000000] MHz",
            ],
        );
        assert_eq!(events, vec![clear(852_000_000.0, 100)]);
    }

    #[test]
    fn registration_lines_name_the_radio_and_not_the_source_id() {
        let mut feed = DsdFeed::default();
        let events = lines_of(
            &mut feed,
            &[
                " Unit Registration Response - SITE: 00A SRC_ID: 7 SRC: 42 RES: 01 REG_ACCEPT",
                " Unit Registration Response - WACN: BEE00; SYS: 2A1; SRC: 99 - Extended;",
                " Unit Registration - SRC: 3; TGT: 8;",
                " Unit Deregistration Acknowlegement - WACN: 00001; SYS: 001; SRC: 5",
            ],
        );
        assert_eq!(
            events,
            vec![
                CcEvent::Unit(UnitSight {
                    id: 42,
                    last_tg: None,
                    peer: None,
                    emergency: false,
                    encrypted: false,
                    site: Some(0x0A),
                    wacn: None,
                    sys: None,
                    reg: Some("REG_ACCEPT".into()),
                }),
                CcEvent::Unit(UnitSight {
                    id: 99,
                    last_tg: None,
                    peer: None,
                    emergency: false,
                    encrypted: false,
                    site: None,
                    wacn: Some("BEE00".into()),
                    sys: Some("2A1".into()),
                    reg: None,
                }),
                CcEvent::Unit(UnitSight {
                    id: 3,
                    last_tg: None,
                    peer: None,
                    emergency: false,
                    encrypted: false,
                    site: None,
                    wacn: None,
                    sys: None,
                    reg: None,
                }),
                CcEvent::Unit(UnitSight {
                    id: 8,
                    last_tg: None,
                    peer: None,
                    emergency: false,
                    encrypted: false,
                    site: None,
                    wacn: None,
                    sys: None,
                    reg: None,
                }),
                CcEvent::Unit(UnitSight {
                    id: 5,
                    last_tg: None,
                    peer: None,
                    emergency: false,
                    encrypted: false,
                    site: None,
                    wacn: Some("00001".into()),
                    sys: Some("001".into()),
                    reg: None,
                }),
            ]
        );
    }

    #[test]
    fn an_emergency_grant_marks_the_radio() {
        let mut feed = DsdFeed::default();
        let events = lines_of(
            &mut feed,
            &[
                " Emergency Group Voice Channel Grant",
                "  SVC [80] CHAN [0123] Group [9] Source [15]",
                "  Frequency [851.012500] MHz",
            ],
        );
        assert_eq!(
            events,
            vec![
                CcEvent::Unit(UnitSight {
                    id: 15,
                    last_tg: Some(9),
                    peer: None,
                    emergency: true,
                    encrypted: false,
                    site: None,
                    wacn: None,
                    sys: None,
                    reg: None,
                }),
                clear(851_012_500.0, 9),
            ]
        );
    }

    #[test]
    fn a_radio_id_does_not_tune() {
        let mut follow = Follower::new(vec![851_012_500.0]);
        follow.note_sent(851_012_500.0);
        follow.push(radio(7, Some(9), false));
        assert_eq!(follow.target_hz(), Some(851_012_500.0));
        assert_eq!(follow.sent(), Some(851_012_500.0));
    }

    #[test]
    fn a_return_from_voice_stays_on_the_locked_control() {
        let mut follow = Follower::new(vec![851_012_500.0, 852_475_000.0]);
        scan_levels(&mut follow, std::time::Instant::now(), &[-18.0, -40.0]);
        follow.push(CcEvent::Iden);
        follow.push(clear(853_500_000.0, 20));
        follow.note_sent(853_500_000.0);
        follow.push(CcEvent::End);
        follow.note_sent(851_012_500.0);
        follow.push(CcEvent::Search);
        follow.push(CcEvent::Search);
        assert_eq!(follow.target_hz(), Some(851_012_500.0));
        assert_eq!(follow.control_hz(), Some(851_012_500.0));
        assert_eq!(follow.status(), "locked");
    }

    /// One strength reading per control. The first `observe` only opens the pass.
    fn scan_levels(follow: &mut Follower, start: std::time::Instant, levels: &[f64]) {
        follow.observe(start, 0.0, true);
        let mut at = start;
        for db in levels {
            follow.observe(at, *db, true);
            at += SCAN_DWELL;
            follow.observe(at, *db, true);
        }
    }

    #[test]
    fn a_scan_parks_on_the_strongest_control_and_stays() {
        let home = 851_012_500.0;
        let mid = 852_475_000.0;
        let last = 853_500_000.0;
        let mut follow = Follower::new(vec![home, mid, last]);
        let start = std::time::Instant::now();
        follow.observe(start, 0.0, true);
        assert_eq!(follow.status(), "scanning 1/3");
        assert_eq!(follow.target_hz(), Some(home));
        scan_levels(&mut follow, start, &[-40.0, -12.0, -30.0]);
        assert_eq!(follow.control_hz(), Some(mid));
        assert_eq!(follow.target_hz(), Some(mid));
        assert_eq!(follow.status(), "locked");
        follow.push(CcEvent::SyncLost);
        follow.push(CcEvent::Search);
        follow.push(CcEvent::Search);
        assert_eq!(follow.control_hz(), Some(mid));
        assert_eq!(follow.target_hz(), Some(mid));
    }

    #[test]
    fn equal_readings_are_not_a_signal() {
        let home = 851_012_500.0;
        let other = 852_475_000.0;
        let mut follow = Follower::new(vec![home, other]);
        scan_levels(&mut follow, std::time::Instant::now(), &[-48.0, -48.0]);
        assert_eq!(follow.control_hz(), Some(home));
        assert!(follow.status().starts_with("scanning"));
        assert_ne!(follow.status(), "locked");
    }

    #[test]
    fn a_chosen_control_is_the_one_the_operator_picked() {
        let dead = 858_512_500.0;
        let live = 859_962_500.0;
        let mut follow = Follower::new(vec![dead, live]);
        assert_eq!(follow.control_hz(), Some(dead));
        assert!(follow.hold(live, std::time::Instant::now()));
        assert_eq!(follow.control_hz(), Some(live));
        assert_eq!(follow.target_hz(), Some(live));
        assert!(!follow.hold(851_000_000.0, std::time::Instant::now()));
        assert_eq!(follow.control_hz(), Some(live));
    }

    #[test]
    fn an_adjacent_site_does_not_retune() {
        let mut follow = Follower::new(vec![859_962_500.0]);
        follow.note_sent(859_962_500.0);
        follow.push(CcEvent::Adjacent {
            hz: 858_962_500.0,
            rfss: 1,
            site: 27,
        });
        assert_eq!(follow.target_hz(), Some(859_962_500.0));
        assert_eq!(follow.control_hz(), Some(859_962_500.0));
        assert_eq!(follow.sent(), Some(859_962_500.0));
    }

    #[test]
    fn a_dmr_talkgroup_grant_is_not_a_p25_grant() {
        let mut feed = DsdFeed::default();
        let events = lines_of(
            &mut feed,
            &[
                " Talkgroup Voice Channel Grant (TV_GRANT)",
                "  LPCN: 0004; TS: 1; LPCN+TS: 0007; Target: 00001216 - Source: 00000101 ",
                "  Frequency: 453.125000 MHz",
                " Connect Plus Group Voice Channel Grant; Target: 1216; Source: 113114; LCN: 2; TS: 1;",
            ],
        );
        assert!(events
            .iter()
            .all(|event| !matches!(event, CcEvent::Grant { .. })));
    }

    fn drive(follow: &mut Follower, feed: &mut DmrFeed, lines: &[&str]) {
        for line in lines {
            for event in feed.push_line(line) {
                follow.push(event);
            }
        }
    }

    #[test]
    fn a_tier3_grant_tunes_after_chan_freq_and_notes_the_target() {
        let mut feed = DmrFeed::default();
        let mut follow = Follower::new(vec![453_000_000.0]);
        drive(
            &mut follow,
            &mut feed,
            &[
                " Announce Logical Channel/Frequency Relationship (Chan_Freq)",
                " Talkgroup Voice Channel Grant (TV_GRANT)",
                "  LPCN: 0004; TS: 2; LPCN+TS: 0008; Target: 00001216 - Source: 00000101 Emergency;",
                "  Frequency: 453.125000 MHz",
            ],
        );
        assert_eq!(follow.target_hz(), Some(453_125_000.0));
        assert_eq!(follow.status(), "voice talkgroup 1216");
        let heard = follow.heard().expect("heard");
        assert_eq!(heard.tg, 1216);
        assert_eq!(heard.alg, CLEAR_ALG);
    }

    #[test]
    fn a_tier3_grant_before_chan_freq_does_not_tune() {
        let mut feed = DmrFeed::default();
        let mut follow = Follower::new(vec![453_000_000.0]);
        drive(
            &mut follow,
            &mut feed,
            &[
                " Talkgroup Voice Channel Grant (TV_GRANT)",
                "  LPCN: 0004; TS: 1; LPCN+TS: 0007; Target: 00001216 - Source: 00000101 ",
                "  Frequency: 453.125000 MHz",
            ],
        );
        assert_eq!(follow.target_hz(), Some(453_000_000.0));
        assert_eq!(follow.status(), "waiting for a channel map");
    }

    #[test]
    fn a_missing_tier3_channel_map_does_not_tune() {
        let mut feed = DmrFeed::default();
        let mut follow = Follower::new(vec![453_000_000.0]);
        drive(
            &mut follow,
            &mut feed,
            &[
                " Announce Logical Channel/Frequency Relationship (Chan_Freq)",
                " Talkgroup Voice Channel Grant (TV_GRANT)",
                "  LPCN: 0004; TS: 1; LPCN+TS: 0007; Target: 00001216 - Source: 00000101 ",
                "  Frequency Not Found in Channel Map;",
            ],
        );
        assert_eq!(follow.target_hz(), Some(453_000_000.0));
        assert_eq!(follow.active_tg(), None);
    }

    #[test]
    fn a_tier3_data_grant_does_not_tune() {
        let mut feed = DmrFeed::default();
        let mut follow = Follower::new(vec![453_000_000.0]);
        drive(
            &mut follow,
            &mut feed,
            &[
                " Announce Logical Channel/Frequency Relationship (Chan_Freq)",
                " Private Data Channel Grant: Single Item (PD_GRANT)",
                "  LPCN: 0004; TS: 1; LPCN+TS: 0007; Target: 00000099 - Source: 00000003 ",
                "  Frequency: 453.200000 MHz",
                " Talkgroup Data Channel Grant: Single Item (TD_GRANT)",
                "  Frequency: 453.250000 MHz",
            ],
        );
        assert_eq!(follow.target_hz(), Some(453_000_000.0));
        assert_eq!(follow.active_tg(), None);
    }

    #[test]
    fn a_private_voice_grant_notes_the_called_radio() {
        let mut feed = DmrFeed::default();
        let events = lines_of_dmr(
            &mut feed,
            &[
                " Announce Logical Channel/Frequency Relationship (Chan_Freq)",
                " Private Voice Channel Grant (PV_GRANT)",
                "  LPCN: 0002; TS: 1; LPCN+TS: 0003; Target: 00000042 - Source: 00000007 ",
                "  Frequency: 453.125000 MHz",
            ],
        );
        assert!(events.iter().any(|event| matches!(
            event,
            CcEvent::Grant {
                tg: 42,
                alg: CLEAR_ALG,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            CcEvent::Unit(UnitSight { id: 7, last_tg: Some(42), .. })
        )));
        let mut follow = Follower::new(vec![453_000_000.0]);
        for event in events {
            follow.push(event);
        }
        assert_eq!(follow.target_hz(), Some(453_125_000.0));
    }

    #[test]
    fn dmr_encryption_returns_to_the_control_channel() {
        let mut feed = DmrFeed::default();
        let mut follow = Follower::new(vec![453_000_000.0]);
        drive(
            &mut follow,
            &mut feed,
            &[
                " Announce Logical Channel/Frequency Relationship (Chan_Freq)",
                " Broadcast Voice Channel Grant (BTV_GRANT)",
                "  LPCN: 0004; TS: 1; LPCN+TS: 0007; Target: 00001000 - Source: 00000008 ",
                "  Frequency: 453.125000 MHz",
            ],
        );
        follow.note_sent(453_125_000.0);
        assert_eq!(follow.role(), Role::Voice);
        let marked = feed.push_line(" SLOT 1 TGT=1000 SRC=8 Encrypted ");
        assert!(marked
            .iter()
            .any(|event| matches!(event, CcEvent::Alg(0))));
        for event in marked {
            follow.push(event);
        }
        assert_eq!(follow.target_hz(), Some(453_000_000.0));
        assert!(follow.status().starts_with("encrypted"));
        let pi = feed.push_line(
            " DMR PI H- ALG ID: 21; KEY ID: 01; MI(32): 00000000; Encryption;",
        );
        assert!(pi.iter().any(|event| matches!(event, CcEvent::Alg(0))));
        assert!(pi.iter().all(|event| !matches!(
            event,
            CcEvent::Unit(_) | CcEvent::Grant { .. }
        )));
    }

    #[test]
    fn dmr_sync_loss_on_voice_returns_and_a_held_control_stays() {
        let mut feed = DmrFeed::default();
        let mut follow = Follower::new(vec![453_000_000.0, 453_500_000.0]);
        drive(
            &mut follow,
            &mut feed,
            &[
                " Announce Logical Channel/Frequency Relationship (Chan_Freq)",
                " Duplex Private Voice Channel Grant (PV_GRANT_DX)",
                "  LPCN: 0005; TS: 2; LPCN+TS: 0010; Target: 00000015 - Source: 00000004 ",
                "  Frequency: 453.125000 MHz",
            ],
        );
        assert_eq!(follow.status(), "voice talkgroup 15");
        assert_eq!(follow.target_hz(), Some(453_125_000.0));
        follow.note_sent(453_125_000.0);
        assert_eq!(follow.role(), Role::Voice);
        for event in feed.push_line("Sync: no sync") {
            follow.push(event);
        }
        assert_eq!(follow.role(), Role::Watch);
        assert_eq!(follow.target_hz(), Some(453_000_000.0));

        let start = std::time::Instant::now();
        let mut held = Follower::new(vec![453_000_000.0, 453_500_000.0]);
        scan_levels(&mut held, start, &[-18.0, -40.0]);
        assert_eq!(held.status(), "locked");
        assert_eq!(held.control_hz(), Some(453_000_000.0));
        for event in feed.push_line("Sync: no sync") {
            held.push(event);
        }
        assert_eq!(held.control_hz(), Some(453_000_000.0));
    }

    #[test]
    fn a_bare_p_clear_does_not_end_the_hop() {
        let mut feed = DmrFeed::default();
        let mut follow = Follower::new(vec![453_000_000.0]);
        drive(
            &mut follow,
            &mut feed,
            &[
                " Announce Logical Channel/Frequency Relationship (Chan_Freq)",
                " Talkgroup Voice Channel Grant (TV_GRANT)",
                "  LPCN: 0004; TS: 1; LPCN+TS: 0007; Target: 00001216 - Source: 00000101 ",
                "  Frequency: 453.125000 MHz",
            ],
        );
        follow.note_sent(453_125_000.0);
        let events = feed.push_line(" Clear (P_CLEAR) ");
        assert!(events.iter().all(|event| !matches!(event, CcEvent::End)));
        for event in events {
            follow.push(event);
        }
        assert_eq!(follow.role(), Role::Voice);
        assert_eq!(follow.target_hz(), Some(453_125_000.0));
    }

    #[test]
    fn dmr_sync_lines_and_a_p25_grant_stay_in_their_lanes() {
        let mut feed = DmrFeed::default();
        let events = lines_of_dmr(
            &mut feed,
            &[
                "Sync: +DMR  ",
                "Sync: +DMR MS/DM MODE/MONO ",
                "Sync: -DMR  ",
                "Sync: +P25p1 ",
                " Group Voice Channel Grant",
                "  Frequency [851.500000] MHz",
                "  RX Freq: 453125000;",
                "hamspot: search",
            ],
        );
        assert_eq!(
            events,
            vec![
                CcEvent::Sync,
                CcEvent::Sync,
                CcEvent::Sync,
                CcEvent::Search,
            ]
        );
    }

    fn lines_of_dmr(feed: &mut DmrFeed, lines: &[&str]) -> Vec<CcEvent> {
        let mut events = Vec::new();
        for line in lines {
            events.extend(feed.push_line(line));
        }
        events
    }

    fn lines_of_nxdn(feed: &mut NxdnFeed, lines: &[&str]) -> Vec<CcEvent> {
        let mut events = Vec::new();
        for line in lines {
            events.extend(feed.push_line(line));
        }
        events
    }

    #[test]
    fn nxdn_dfa_and_import_frequencies_grant_and_unknowns_do_not() {
        let mut feed = NxdnFeed::default();
        let dfa = lines_of_nxdn(
            &mut feed,
            &[
                " Src=12 - Dst/TG=400 - DFA Channel [0001][00001] ",
                "  DFA Frequency [453.012500] MHz",
            ],
        );
        assert!(dfa.iter().any(|event| matches!(event, CcEvent::Iden)));
        assert!(dfa.iter().any(|event| matches!(
            event,
            CcEvent::Grant {
                hz,
                tg: 400,
                alg: CLEAR_ALG,
            } if (*hz - 453_012_500.0).abs() < 1.0
        )));
        let imported = lines_of_nxdn(
            &mut feed,
            &[
                " Src=3 - Dst/TG=8 - Channel [003][0003] ",
                "  Frequency [451.100000] MHz",
            ],
        );
        assert!(imported.iter().any(|event| matches!(
            event,
            CcEvent::Grant { tg: 8, alg: CLEAR_ALG, .. }
        )));
        let unknown = lines_of_nxdn(
            &mut feed,
            &[
                " Src=3 - Dst/TG=8 - DFA Channel [0001][00001] ",
                "    Custom DFA Settings -- Unknown Freq;",
                "  DFA Frequency [453.012500] MHz",
            ],
        );
        assert!(unknown.iter().all(|event| !matches!(event, CcEvent::Grant { .. })));
        let missing = lines_of_nxdn(
            &mut feed,
            &[
                " Src=3 - Dst/TG=8 - Channel [003][0003] ",
                "    Channel not found in import file",
                "  Frequency [451.100000] MHz",
            ],
        );
        assert!(missing.iter().all(|event| !matches!(event, CcEvent::Grant { .. })));
    }

    #[test]
    fn nxdn_data_encryption_termination_and_ran_do_not_tune() {
        let mut feed = NxdnFeed::default();
        let data = lines_of_nxdn(
            &mut feed,
            &[
                " Data Call Assignment (00) - Src=1 - Dst/TG=2 ",
                "  Frequency [450.000000] MHz",
            ],
        );
        assert!(data.iter().all(|event| !matches!(event, CcEvent::Grant { .. })));
        let enc = lines_of_nxdn(
            &mut feed,
            &[
                " Src=1 - Dst/TG=9  Enc - Channel [001][0001] ",
                "  Frequency [451.100000] MHz",
            ],
        );
        assert!(enc.iter().any(|event| matches!(
            event,
            CcEvent::Grant { tg: 9, alg: 0, .. }
        )));
        assert!(enc.iter().all(|event| !matches!(event, CcEvent::Iden)));
        let scrambler = lines_of_nxdn(
            &mut feed,
            &[
                " Src=1 - Dst/TG=9 - Channel [001][0001] ",
                " Scrambler; ",
                "  DFA Frequency [453.012500] MHz",
            ],
        );
        assert!(scrambler.iter().any(|event| matches!(
            event,
            CcEvent::Grant { alg: 0, .. }
        )));
        let ran = feed.push_line("Sync: NXDN48  RDCH Voice  RAN 01");
        assert!(ran.iter().any(|event| matches!(event, CcEvent::Sync)));
        assert!(ran.iter().all(|event| !matches!(event, CcEvent::Grant { .. })));
        let alias = feed.push_line(" Completed Alias Encoded: TEST");
        assert!(alias.iter().all(|event| !matches!(event, CcEvent::Alg(0) | CcEvent::Grant { .. })));
        let end = lines_of_nxdn(
            &mut feed,
            &[
                " Channel Update - CH: 4 - TGT: 15 Group Call ",
                "  Frequency [460.025000] MHz",
            ],
        );
        assert!(end.iter().any(|event| matches!(
            event,
            CcEvent::Grant { tg: 15, alg: CLEAR_ALG, .. }
        )));
        let stop = lines_of_nxdn(
            &mut feed,
            &[
                " Channel Update - CH: 31 - TGT: 15 Group Call Termination ",
                "  Transmission Release  - ",
                " TX_REL_EX",
                " DISC",
            ],
        );
        assert!(stop.iter().filter(|event| matches!(event, CcEvent::End)).count() >= 3);
        assert!(stop.iter().all(|event| !matches!(event, CcEvent::Grant { .. })));
        let mut follow = Follower::new(vec![460_025_000.0]);
        for event in lines_of_nxdn(
            &mut NxdnFeed::default(),
            &[
                " Src=4 - Dst/TG=15 - Channel [004][0004] ",
                "  Frequency [460.100000] MHz",
            ],
        ) {
            follow.push(event);
        }
        follow.note_sent(460_100_000.0);
        assert_eq!(follow.role(), Role::Voice);
        let during = lines_of_nxdn(
            &mut NxdnFeed::default(),
            &[
                " Src=4 - Dst/TG=99 - Channel [005][0005] ",
                "  Frequency [460.200000] MHz",
            ],
        );
        for event in during {
            follow.push(event);
        }
        assert_eq!(follow.target_hz(), Some(460_100_000.0));
    }

    #[test]
    fn identity_lines_fill_the_glass_and_a_trunk_talkgroup_keeps_the_headline() {
        let mut ident = AirIdentity::default();
        note_identity(&mut ident, "Sync: +DMR  | Color Code=03 | VC6");
        note_identity(&mut ident, " Color Code=XX ");
        note_identity(&mut ident, " TGT=1216 SRC=101 ");
        assert_eq!(ident.code_label, "CC");
        assert_eq!(ident.code, "03");
        assert_eq!(ident.tg, Some(1216));
        assert_eq!(ident.src, "101");
        note_identity(&mut ident, "Sync: NXDN96  RCCH  RAN 07 ");
        assert_eq!(ident.code, "07");
        assert_eq!(ident.code_label, "RAN");
        let mut dpmr = AirIdentity::default();
        note_identity(&mut dpmr, " TG=(CRC ERR)");
        note_identity(&mut dpmr, " Src=(CRC ERR) Channel Code =(CRC ERR)");
        assert_eq!(dpmr, AirIdentity::default());
        note_identity(&mut dpmr, " TG=44 Src=9 Channel Code=12");
        assert_eq!(dpmr.tg, Some(44));
        assert_eq!(dpmr.src, "9");
        assert_eq!(dpmr.code_label, "Code");
        assert_eq!(dpmr.code, "12");
        let mut ysf = AirIdentity::default();
        note_identity(&mut ysf, "Repeater HC CC FN: 1/8 ");
        assert!(ysf.code.is_empty());
        note_identity(&mut ysf, "Repeater SQL  CODE: 009 DST: K9ABC     SRC: N9DEF    ");
        assert_eq!(ysf.code_label, "SQL");
        assert_eq!(ysf.code, "009");
        assert_eq!(ysf.dst, "K9ABC");
        assert_eq!(ysf.src, "N9DEF");
        let mut m17 = AirIdentity::default();
        note_identity(&mut m17, " DST: BROADCAST");
        note_identity(&mut m17, " SRC: N0CALL");
        note_identity(&mut m17, " CAN: 7");
        assert_eq!(m17.dst, "BROADCAST");
        assert_eq!(m17.src, "N0CALL");
        assert_eq!(m17.code_label, "CAN");
        assert_eq!(m17.code, "7");
        let mut dstar = AirIdentity::default();
        note_identity(
            &mut dstar,
            " RPT 2: WW8IND B RPT 1: WW8IND G DST: CQCQCQ   SRC: N9ABC ",
        );
        assert_eq!(dstar.rpt2, "WW8IND B");
        assert_eq!(dstar.rpt1, "WW8IND G");
        assert_eq!(dstar.dst, "CQCQCQ");
        assert_eq!(dstar.src, "N9ABC");
        let mut edacs = AirIdentity::default();
        note_identity(&mut edacs, " TGT [00001234] LCN [02]");
        assert_eq!(edacs.tg, Some(1234));
        note_identity(&mut edacs, " AFS [003] [02-041] LCN [02]");
        assert!(edacs.dst.is_empty());
        let mut edacs_afs = AirIdentity::default();
        note_identity(&mut edacs_afs, " TGT [ SYSTEM ] LCN [02]");
        note_identity(&mut edacs_afs, " AFS [003] [02-041] LCN [02]");
        assert!(edacs_afs.tg.is_none());
        assert_eq!(edacs_afs.dst, "[003] [02-041]");
        let mut provoice = AirIdentity::default();
        note_identity(&mut provoice, " Site: 1 Group: 12 Source: 34 LCN: 2 ");
        assert_eq!(provoice.tg, Some(12));
        assert_eq!(provoice.src, "34");
        note_identity(&mut provoice, " Site: 1 Target: 8 Source: 3 LCN: 2 ");
        assert_eq!(provoice.tg, Some(8));
        assert_eq!(provoice.src, "3");
        let mut freedv = AirIdentity::default();
        note_identity(
            &mut freedv,
            "frame: 3  sync: 0  nin: 640 snr: 1.00 dB  bit errors: 0 clock_off: 0.00 foff: 0.00",
        );
        assert!(freedv.code.is_empty());
        note_identity(
            &mut freedv,
            "frame: 4  sync: 1  nin: 640 snr: -1.50 dB  bit errors: 0 clock_off: 0.10 foff: 2.00",
        );
        assert_eq!(freedv.code_label, "SNR");
        assert_eq!(freedv.code, "-1.50");
        let mut drm = AirIdentity::default();
        note_identity(
            &mut drm,
            "[   5.0s] Tracking mode B 10 DC 350.0 Hz SNR 12.4 dB MER 18.0 dB",
        );
        assert_eq!(drm.code_label, "SNR");
        assert_eq!(drm.code, "12.4");
        note_identity(&mut drm, "service 0 id D0D000 (no label yet) audio, News");
        assert!(drm.dst.is_empty());
        note_identity(
            &mut drm,
            "service 0 id D0D000 BBC World Service xHE-AAC stereo",
        );
        assert_eq!(drm.dst, "BBC World Service xHE-AAC stereo");
        let rows = glass_rows(None, &dstar);
        assert_eq!(rows.headline.as_deref(), Some("N9ABC"));
        assert_eq!(rows.detail.as_deref(), Some("CQCQCQ"));
        assert_eq!(rows.facts.as_deref(), Some("RPT1 WW8IND G   RPT2 WW8IND B"));
        let busy = glass_rows(Some(5), &dstar);
        assert!(busy.headline.is_none());
        assert!(busy.tgid.is_none());
        assert!(busy.detail.is_none());
        let coded = glass_rows(Some(5), &ident);
        assert_eq!(coded.code_line.as_deref(), Some("RAN 07"));
        assert!(coded.headline.is_none());
    }

    #[test]
    fn a_decoder_line_names_a_missing_key() {
        assert_eq!(
            key_on_line(" HDU  ALG ID: 0x81 KEY ID: 0x1A2B MI: 0x11"),
            Some((0x81, 0x1A2B))
        );
        assert_eq!(
            key_on_line(" DMR PI H- ALG ID: 21; KEY ID: 01; MI(32): 00000000; Encryption;"),
            Some((0x21, 0x01))
        );
        assert_eq!(key_on_line(" ALG ID: 0x80 KEY ID: 0x0000"), None);
        assert_eq!(key_on_line("  SG: 1; KEY: 1A2B; WGID: 100; "), None);
    }
}

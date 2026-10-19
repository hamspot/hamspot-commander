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
    /// Phase 1 frame sync is present.
    Sync,
    /// The decoder reported that sync is gone.
    SyncLost,
    /// An adjacent site's outbound control channel. `rfss` and `site` are the
    /// numbers printed on the advertisement. The follower does not tune this.
    Adjacent { hz: f64, rfss: u32, site: u32 },
    /// A radio named on this control channel. The follower does not tune it.
    Unit(UnitSight),
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
            CcEvent::Adjacent { .. } | CcEvent::Unit(_) => {}
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
        }
        if let Some(alg) = alg_id(&line) {
            out.push(CcEvent::Alg(alg));
        }
        out.extend(unit_admin_events(&line));
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
                unit: line.contains("Unit to Unit Channel Grant"),
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
                self.open = None;
            } else if let Some(hz) = frequency_hz(&line) {
                if let Some(grant) = self.open.take() {
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
    line.contains("Group Voice Channel Grant") || line.contains("Unit to Unit Channel Grant")
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
}

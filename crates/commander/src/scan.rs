//! Channel scan for the zone on screen.
//!
//! The rig does not say when an S-meter reading belongs to the channel just
//! tuned. The first poll whose frequency matches is discarded. The next
//! matching poll is the sample. Hopping waits for that sample. It does not
//! wait on a timer.
//!
//! A trunking radio hops its control memories when the zone has one it can
//! tune. A zone with none is scanned like a conventional zone. Staying on an
//! open control is what gives the decoder time to print a neighbor. Those
//! announcements are stored by the control feed; this type only hops the
//! list it was given.

use std::time::{Duration, Instant};

/// Listen time when DWELL is lit. The operator asked for five seconds.
/// The rig has no event that means that long, so this deadline is wall
/// clock. The hop itself still waits for the next settled poll.
pub const DWELL: Duration = Duration::from_secs(5);

const OPEN_MARGIN_DB: f64 = 6.0;
const MARGIN_MAX_DB: f64 = 12.0;
const MARGIN_MIN_DB: f64 = 2.0;
/// A pass whose meter moved this far, and never opened, is traffic under
/// the threshold. A flat meter is left alone.
const PASS_MOVE_DB: f64 = 4.0;
/// `opened / n` at least four fifths, and at least three memories.
fn mostly_open(opened: usize, n: usize) -> bool {
    n >= 3 && opened.saturating_mul(5) >= n.saturating_mul(4)
}
/// Same window the control follower uses for "the radio is on this frequency".
const FREQ_MATCH_HZ: f64 = 500.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Memory {
    pub zone: usize,
    pub index: usize,
    pub hz: f64,
}

/// One zone memory before the trunk filter.
#[derive(Clone, Copy, Debug)]
pub struct Candidate {
    pub index: usize,
    pub hz: f64,
    pub control: bool,
    pub tunable: bool,
}

/// Memories this scan will hop. A trunking radio hops controls when the zone
/// has one it can tune. With none, every tunable memory is used.
pub fn targets(zone: usize, channels: &[Candidate], trunk: bool) -> Vec<Memory> {
    let conventional: Vec<Memory> = channels
        .iter()
        .filter(|channel| channel.tunable && channel.hz.is_finite() && channel.hz > 0.0)
        .map(|channel| Memory {
            zone,
            index: channel.index,
            hz: channel.hz,
        })
        .collect();
    if !trunk {
        return conventional;
    }
    let controls: Vec<Memory> = channels
        .iter()
        .filter(|channel| {
            channel.control && channel.tunable && channel.hz.is_finite() && channel.hz > 0.0
        })
        .map(|channel| Memory {
            zone,
            index: channel.index,
            hz: channel.hz,
        })
        .collect();
    if controls.is_empty() {
        conventional
    } else {
        controls
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub now: Instant,
    pub poll_gen: u64,
    pub freq_hz: f64,
    /// dB relative to S9. Callers convert a GQRX dBFS reading first.
    pub strength_db: f64,
    pub trunk: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Idle,
    /// `resend` is false when the radio is already on the only memory.
    /// The caller must not write the frequency again.
    Tune { index: usize, resend: bool },
    Stop,
}

#[derive(Clone, Debug)]
struct Active {
    zone: usize,
    index: usize,
    trunk: bool,
    /// Walk every zone in `memories`. A one-zone scan stops when the operator
    /// picks another zone until the face retargets. This mode follows the list.
    all_zones: bool,
    phase: Phase,
    last_poll: u64,
    /// Quietest settled sample from the seed pass, then whatever a later
    /// pass raised or lowered it to.
    floor: Option<f64>,
    margin: f64,
    pass_db: Vec<f64>,
    opened: usize,
    /// The first pass only measures. A hold before that pass finishes would
    /// freeze on whichever channel happened to be noisiest.
    seeding: bool,
}

#[derive(Clone, Copy, Debug)]
enum Phase {
    /// Send the current memory once.
    Kick { hz: f64 },
    /// `after_poll` commanded the tune. A later poll that matches arms.
    Arm { hz: f64, after_poll: u64 },
    /// `armed_poll` matched and its strength was discarded.
    Settled { hz: f64, armed_poll: u64 },
    /// Already on this memory. The next poll is a sample. No tune.
    Direct { hz: f64 },
    Hold { hz: f64, until: Option<Instant> },
}

pub struct Scanner {
    dwell: bool,
    active: Option<Active>,
}

impl Scanner {
    pub fn new() -> Self {
        Self {
            dwell: false,
            active: None,
        }
    }

    pub fn is_on(&self) -> bool {
        self.active.is_some()
    }

    pub fn dwell_on(&self) -> bool {
        self.dwell
    }

    pub fn toggle_dwell(&mut self) {
        self.dwell = !self.dwell;
    }

    pub fn stop(&mut self) {
        self.active = None;
    }

    pub fn all_zones(&self) -> bool {
        self.active.as_ref().is_some_and(|active| active.all_zones)
    }

    pub fn zone(&self) -> Option<usize> {
        self.active.as_ref().map(|active| active.zone)
    }

    pub fn index(&self) -> Option<usize> {
        self.active.as_ref().map(|active| active.index)
    }

    /// Memory the scan is holding because the carrier opened.
    /// A hop, a seed pass, and an idle scanner are not a hold.
    pub fn hold_memory(&self) -> Option<(usize, usize)> {
        let active = self.active.as_ref()?;
        if matches!(active.phase, Phase::Hold { .. }) {
            Some((active.zone, active.index))
        } else {
            None
        }
    }

    /// Walk whatever list the caller passes, including zones other than the
    /// one on screen. The floor, the dwell, and the current memory stay.
    pub fn follow_list(&mut self) {
        if let Some(active) = self.active.as_mut() {
            active.all_zones = true;
        }
    }

    /// Wall-clock end of a DWELL listen. `None` while the scan is hopping
    /// or holding until the carrier falls.
    pub fn dwell_deadline(&self) -> Option<Instant> {
        match self.active.as_ref().map(|active| active.phase) {
            Some(Phase::Hold { until, .. }) => until,
            _ => None,
        }
    }

    pub fn start(&mut self, zone: usize, index: usize, hz: f64, trunk: bool, all_zones: bool) {
        self.active = Some(Active {
            zone,
            index,
            trunk,
            all_zones,
            phase: Phase::Kick { hz },
            last_poll: 0,
            floor: None,
            margin: OPEN_MARGIN_DB,
            pass_db: Vec::new(),
            opened: 0,
            seeding: true,
        });
    }

    /// Move an all-zone scan onto another zone without clearing the floor
    /// or the amber mode. The next step sends this memory.
    pub fn jump(&mut self, zone: usize, index: usize, hz: f64) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        active.zone = zone;
        active.index = index;
        active.phase = Phase::Kick { hz };
        active.pass_db.clear();
        active.opened = 0;
    }

    pub fn step(&mut self, sample: &Sample, zone: usize, memories: &[Memory]) -> Action {
        let Some(active) = self.active.as_ref() else {
            return Action::Idle;
        };
        if active.trunk != sample.trunk {
            self.active = None;
            return Action::Stop;
        }
        if active.zone != zone {
            if active.all_zones {
                return self.jump_listed_zone(sample, zone, memories);
            }
            self.active = None;
            return Action::Stop;
        }
        if memories.is_empty() {
            self.active = None;
            return Action::Stop;
        }
        self.sync_dwell(sample.now);
        if self.deadline_due(sample.now) {
            return self.hop(sample, memories);
        }
        if matches!(
            self.active.as_ref().map(|active| active.phase),
            Some(Phase::Kick { .. })
        ) {
            return self.kick(sample);
        }
        let last = match self.active.as_ref() {
            Some(active) => active.last_poll,
            None => return Action::Stop,
        };
        if sample.poll_gen == last {
            return Action::Idle;
        }
        if let Some(active) = self.active.as_mut() {
            active.last_poll = sample.poll_gen;
        }
        let index = match self.active.as_ref() {
            Some(active) => active.index,
            None => return Action::Stop,
        };
        let here = match self.active.as_ref() {
            Some(active) => active.zone,
            None => return Action::Stop,
        };
        if !memories
            .iter()
            .any(|memory| memory.zone == here && memory.index == index)
        {
            return self.hop(sample, memories);
        }
        self.on_poll(sample, memories)
    }

    fn sync_dwell(&mut self, now: Instant) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let Phase::Hold { until, .. } = &mut active.phase else {
            return;
        };
        if self.dwell {
            if until.is_none() {
                // Five seconds from the moment DWELL becomes lit during a
                // hold. See [`DWELL`].
                *until = Some(now + DWELL);
            }
        } else {
            *until = None;
        }
    }

    fn deadline_due(&self, now: Instant) -> bool {
        self.dwell_deadline().is_some_and(|deadline| now >= deadline)
    }

    fn kick(&mut self, sample: &Sample) -> Action {
        let Some(active) = self.active.as_mut() else {
            return Action::Stop;
        };
        let Phase::Kick { hz } = active.phase else {
            return Action::Idle;
        };
        let index = active.index;
        active.last_poll = sample.poll_gen;
        active.phase = Phase::Arm {
            hz,
            after_poll: sample.poll_gen,
        };
        Action::Tune {
            index,
            resend: true,
        }
    }

    fn on_poll(&mut self, sample: &Sample, memories: &[Memory]) -> Action {
        let phase = match self.active.as_ref().map(|active| active.phase) {
            Some(phase) => phase,
            None => return Action::Stop,
        };
        match phase {
            Phase::Kick { .. } => Action::Idle,
            Phase::Arm { hz, after_poll } => {
                if sample.poll_gen <= after_poll || !near(sample.freq_hz, hz) {
                    return Action::Idle;
                }
                if let Some(active) = self.active.as_mut() {
                    active.phase = Phase::Settled {
                        hz,
                        armed_poll: sample.poll_gen,
                    };
                }
                Action::Idle
            }
            Phase::Settled { hz, armed_poll } => {
                if !near(sample.freq_hz, hz) {
                    if let Some(active) = self.active.as_mut() {
                        active.phase = Phase::Arm {
                            hz,
                            after_poll: sample.poll_gen,
                        };
                    }
                    return Action::Idle;
                }
                if sample.poll_gen <= armed_poll || !sample.strength_db.is_finite() {
                    return Action::Idle;
                }
                self.classify(sample, memories)
            }
            Phase::Direct { hz } => {
                if !near(sample.freq_hz, hz) {
                    if let Some(active) = self.active.as_mut() {
                        active.phase = Phase::Arm {
                            hz,
                            after_poll: sample.poll_gen,
                        };
                    }
                    let index = self.active.as_ref().map(|active| active.index).unwrap_or(0);
                    return Action::Tune {
                        index,
                        resend: true,
                    };
                }
                if !sample.strength_db.is_finite() {
                    return Action::Idle;
                }
                self.classify(sample, memories)
            }
            Phase::Hold { hz, until } => {
                if until.is_some() {
                    return Action::Idle;
                }
                if !near(sample.freq_hz, hz) {
                    if let Some(active) = self.active.as_mut() {
                        active.phase = Phase::Arm {
                            hz,
                            after_poll: sample.poll_gen,
                        };
                    }
                    let index = self.active.as_ref().map(|active| active.index).unwrap_or(0);
                    return Action::Tune {
                        index,
                        resend: true,
                    };
                }
                if !sample.strength_db.is_finite() {
                    return Action::Idle;
                }
                let (floor, margin) = self.threshold();
                if sample.strength_db < floor + margin * 0.5 {
                    return self.hop(sample, memories);
                }
                Action::Idle
            }
        }
    }

    fn classify(&mut self, sample: &Sample, memories: &[Memory]) -> Action {
        let db = sample.strength_db;
        let seeding = self.active.as_ref().is_some_and(|active| active.seeding);
        if seeding {
            if let Some(active) = self.active.as_mut() {
                active.pass_db.push(db);
                if active.pass_db.len() >= memories.len() {
                    finish_seed(active);
                }
            }
            return self.hop(sample, memories);
        }
        if let Some(active) = self.active.as_mut() {
            if active.pass_db.len() > memories.len() {
                active.pass_db.clear();
                active.opened = 0;
            }
        }
        let (floor, margin) = self.threshold();
        if let Some(active) = self.active.as_mut() {
            if db > floor + margin {
                active.opened += 1;
            }
            active.pass_db.push(db);
            if active.pass_db.len() >= memories.len() {
                adjust(active);
            }
        }
        let (floor, margin) = self.threshold();
        if db > floor + margin {
            let until = if self.dwell {
                // See [`DWELL`]. Hopping does not use this deadline.
                Some(sample.now + DWELL)
            } else {
                None
            };
            let hz = self.current_hz(memories);
            if let Some(active) = self.active.as_mut() {
                active.phase = Phase::Hold { hz, until };
            }
            return Action::Idle;
        }
        self.hop(sample, memories)
    }

    fn hop(&mut self, sample: &Sample, memories: &[Memory]) -> Action {
        let Some(active) = self.active.as_mut() else {
            return Action::Stop;
        };
        let Some(next) = next_memory(active.zone, active.index, memories) else {
            self.active = None;
            return Action::Stop;
        };
        let same = next.zone == active.zone && next.index == active.index;
        active.zone = next.zone;
        active.index = next.index;
        active.last_poll = sample.poll_gen;
        if same {
            active.phase = Phase::Direct { hz: next.hz };
            Action::Tune {
                index: next.index,
                resend: false,
            }
        } else {
            active.phase = Phase::Arm {
                hz: next.hz,
                after_poll: sample.poll_gen,
            };
            Action::Tune {
                index: next.index,
                resend: true,
            }
        }
    }

    fn threshold(&self) -> (f64, f64) {
        let Some(active) = self.active.as_ref() else {
            return (f64::MAX, OPEN_MARGIN_DB);
        };
        (active.floor.unwrap_or(f64::MAX), active.margin)
    }

    fn current_hz(&self, memories: &[Memory]) -> f64 {
        let Some(active) = self.active.as_ref() else {
            return 0.0;
        };
        memories
            .iter()
            .find(|memory| memory.zone == active.zone && memory.index == active.index)
            .map(|memory| memory.hz)
            .unwrap_or(0.0)
    }

    fn jump_listed_zone(&mut self, sample: &Sample, zone: usize, memories: &[Memory]) -> Action {
        let Some(first) = memories.iter().copied().find(|memory| memory.zone == zone) else {
            self.active = None;
            return Action::Stop;
        };
        self.jump(first.zone, first.index, first.hz);
        self.kick(sample)
    }
}

fn finish_seed(active: &mut Active) {
    let min = min_of(&active.pass_db);
    let med = median(&active.pass_db);
    let opened = active
        .pass_db
        .iter()
        .filter(|db| **db > min + active.margin)
        .count();
    let n = active.pass_db.len();
    active.floor = Some(min);
    if mostly_open(opened, n) {
        // The minimum was an outlier. Almost every channel cleared it, so
        // that minimum is not the noise floor.
        active.floor = Some(med);
        active.margin = (active.margin + 1.0).min(MARGIN_MAX_DB);
    }
    active.seeding = false;
    active.pass_db.clear();
    active.opened = 0;
}

fn adjust(active: &mut Active) {
    let n = active.pass_db.len();
    if n == 0 {
        return;
    }
    let min = min_of(&active.pass_db);
    let max = max_of(&active.pass_db);
    let med = median(&active.pass_db);
    if mostly_open(active.opened, n) {
        active.floor = Some(med);
        active.margin = (active.margin + 1.0).min(MARGIN_MAX_DB);
    } else if active.opened == 0 && max - min >= PASS_MOVE_DB {
        active.margin = (active.margin - 1.0).max(MARGIN_MIN_DB);
        if let Some(floor) = active.floor {
            if min < floor {
                active.floor = Some((floor + min) / 2.0);
            }
        }
    }
    active.pass_db.clear();
    active.opened = 0;
}

fn next_memory(zone: usize, index: usize, memories: &[Memory]) -> Option<Memory> {
    if memories.is_empty() {
        return None;
    }
    if let Some(memory) = memories.iter().copied().find(|memory| {
        memory.zone > zone || (memory.zone == zone && memory.index > index)
    }) {
        return Some(memory);
    }
    memories.first().copied()
}

fn near(reported: f64, wanted: f64) -> bool {
    (reported - wanted).abs() < FREQ_MATCH_HZ
}

fn min_of(values: &[f64]) -> f64 {
    values.iter().copied().fold(f64::INFINITY, f64::min)
}

fn max_of(values: &[f64]) -> f64 {
    values.iter().copied().fold(f64::NEG_INFINITY, f64::max)
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memories(n: usize) -> Vec<Memory> {
        (0..n)
            .map(|index| Memory {
                zone: 0,
                index,
                hz: 100_000_000.0 + index as f64 * 1_000_000.0,
            })
            .collect()
    }

    fn at(now: Instant, poll: u64, hz: f64, db: f64) -> Sample {
        Sample {
            now,
            poll_gen: poll,
            freq_hz: hz,
            strength_db: db,
            trunk: false,
        }
    }

    /// Reach one settled sample. A kick sends, then waits an extra poll.
    /// A tune that is already armed discards the next poll. A direct
    /// re-sample uses the next poll as the sample.
    fn land(
        scan: &mut Scanner,
        now: Instant,
        poll: &mut u64,
        hz: f64,
        db: f64,
        mem: &[Memory],
    ) -> Action {
        let active = scan.active.as_ref().expect("scanning");
        let trunk = active.trunk;
        let phase = active.phase;
        let sample = |poll: u64| Sample {
            now,
            poll_gen: poll,
            freq_hz: hz,
            strength_db: db,
            trunk,
        };
        match phase {
            Phase::Kick { .. } => {
                *poll += 1;
                let kicked = scan.step(&sample(*poll), 0, mem);
                assert!(matches!(kicked, Action::Tune { resend: true, .. }));
                *poll += 1;
                assert_eq!(scan.step(&sample(*poll), 0, mem), Action::Idle);
                *poll += 1;
                scan.step(&sample(*poll), 0, mem)
            }
            Phase::Arm { .. } => {
                *poll += 1;
                assert_eq!(
                    scan.step(&sample(*poll), 0, mem),
                    Action::Idle,
                    "arming poll is not a sample"
                );
                *poll += 1;
                scan.step(&sample(*poll), 0, mem)
            }
            Phase::Direct { .. } => {
                *poll += 1;
                scan.step(&sample(*poll), 0, mem)
            }
            other => panic!("land from {other:?}"),
        }
    }

    fn floor_margin(scan: &Scanner) -> (f64, f64) {
        let active = scan.active.as_ref().expect("scanning");
        (active.floor.expect("floor"), active.margin)
    }

    #[test]
    fn all_zones_hops_into_the_next_zone() {
        let mem = vec![
            Memory {
                zone: 0,
                index: 0,
                hz: 100_000_000.0,
            },
            Memory {
                zone: 1,
                index: 0,
                hz: 200_000_000.0,
            },
        ];
        let mut scan = Scanner::new();
        scan.start(0, 0, mem[0].hz, false, true);
        let now = Instant::now();
        let mut poll = 0;
        let hopped = land(&mut scan, now, &mut poll, mem[0].hz, -54.0, &mem);
        assert_eq!(hopped, Action::Tune { index: 0, resend: true });
        assert_eq!(scan.zone(), Some(1));
        assert!(scan.all_zones());
        assert!(scan.is_on());
    }

    /// One quiet sample on a memory, using that memory's zone. `land` always
    /// reports zone 0, which an all-zone scan would treat as a zone change.
    fn settle(
        scan: &mut Scanner,
        poll: &mut u64,
        zone: usize,
        hz: f64,
        db: f64,
        mem: &[Memory],
    ) -> Action {
        let now = Instant::now();
        let trunk = scan.active.as_ref().expect("scanning").trunk;
        let phase = scan.active.as_ref().expect("scanning").phase;
        let sample = |poll: u64| Sample {
            now,
            poll_gen: poll,
            freq_hz: hz,
            strength_db: db,
            trunk,
        };
        match phase {
            Phase::Kick { .. } => {
                *poll += 1;
                let kicked = scan.step(&sample(*poll), zone, mem);
                assert!(matches!(kicked, Action::Tune { resend: true, .. }));
                *poll += 1;
                assert_eq!(scan.step(&sample(*poll), zone, mem), Action::Idle);
                *poll += 1;
                scan.step(&sample(*poll), zone, mem)
            }
            Phase::Arm { .. } => {
                *poll += 1;
                assert_eq!(scan.step(&sample(*poll), zone, mem), Action::Idle);
                *poll += 1;
                scan.step(&sample(*poll), zone, mem)
            }
            other => panic!("settle from {other:?}"),
        }
    }

    #[test]
    fn a_filtered_list_skips_a_zone_and_the_full_list_returns() {
        let all = vec![
            Memory {
                zone: 0,
                index: 0,
                hz: 100_000_000.0,
            },
            Memory {
                zone: 1,
                index: 0,
                hz: 200_000_000.0,
            },
            Memory {
                zone: 2,
                index: 0,
                hz: 300_000_000.0,
            },
        ];
        let filtered = vec![all[0], all[2]];
        let mut scan = Scanner::new();
        scan.start(0, 0, all[0].hz, false, false);
        scan.follow_list();
        assert!(scan.all_zones());
        let mut poll = 0;
        let first = settle(&mut scan, &mut poll, 0, all[0].hz, -54.0, &filtered);
        assert_eq!(first, Action::Tune { index: 0, resend: true });
        assert_eq!(scan.zone(), Some(2));
        assert_eq!(scan.index(), Some(0));
        let second = settle(&mut scan, &mut poll, 2, all[2].hz, -54.0, &filtered);
        assert_eq!(second, Action::Tune { index: 0, resend: true });
        assert_eq!(scan.zone(), Some(0));
        assert_eq!(floor_margin(&scan).0, -54.0);
        let restored = settle(&mut scan, &mut poll, 0, all[0].hz, -54.0, &all);
        assert_eq!(restored, Action::Tune { index: 0, resend: true });
        assert_eq!(scan.zone(), Some(1));
        assert!(scan.all_zones());
        assert!(scan.is_on());
        assert_eq!(floor_margin(&scan).0, -54.0);
    }

    #[test]
    fn a_new_zone_keeps_the_scan_and_tunes_that_zone() {
        let mem = memories(2);
        let mut scan = Scanner::new();
        scan.dwell = true;
        scan.start(0, 0, mem[0].hz, false, false);
        scan.start(3, 1, mem[1].hz, false, false);
        assert!(scan.is_on());
        assert!(scan.dwell_on());
        let now = Instant::now();
        let action = scan.step(&at(now, 1, mem[1].hz, -54.0), 3, &mem);
        assert_eq!(action, Action::Tune { index: 1, resend: true });
        let stopped = scan.step(&at(now, 1, mem[1].hz, -54.0), 4, &mem);
        assert_eq!(stopped, Action::Stop);
        assert!(!scan.is_on());
    }

    #[test]
    fn the_arming_poll_is_discarded_and_a_quiet_sample_hops() {
        let mem = memories(2);
        let mut scan = Scanner::new();
        scan.start(0, 0, mem[0].hz, false, false);
        let now = Instant::now();
        let mut poll = 0;
        let hopped = land(&mut scan, now, &mut poll, mem[0].hz, -54.0, &mem);
        assert_eq!(
            hopped,
            Action::Tune {
                index: 1,
                resend: true
            }
        );
    }

    #[test]
    fn a_loud_arming_poll_does_not_hold() {
        let mem = memories(2);
        let mut scan = Scanner::new();
        scan.start(0, 0, mem[0].hz, false, false);
        let now = Instant::now();
        let kick = scan.step(&at(now, 1, mem[0].hz, -20.0), 0, &mem);
        assert_eq!(
            kick,
            Action::Tune {
                index: 0,
                resend: true
            }
        );
        let armed = scan.step(&at(now, 2, mem[0].hz, -20.0), 0, &mem);
        assert_eq!(armed, Action::Idle);
        assert!(scan.dwell_deadline().is_none());
    }

    #[test]
    fn dwell_off_holds_until_the_carrier_falls() {
        let mem = memories(2);
        let mut scan = Scanner::new();
        scan.start(0, 0, mem[0].hz, false, false);
        let now = Instant::now();
        let mut poll = 0;
        // Seed both channels at the noise floor. No hold on this pass.
        let to_second = land(&mut scan, now, &mut poll, mem[0].hz, -54.0, &mem);
        assert_eq!(to_second, Action::Tune { index: 1, resend: true });
        let back = land(&mut scan, now, &mut poll, mem[1].hz, -54.0, &mem);
        assert_eq!(back, Action::Tune { index: 0, resend: true });
        let hold = land(&mut scan, now, &mut poll, mem[0].hz, -40.0, &mem);
        assert_eq!(hold, Action::Idle);
        poll += 1;
        let still = scan.step(&at(now, poll, mem[0].hz, -40.0), 0, &mem);
        assert_eq!(still, Action::Idle);
        let later = now + Duration::from_secs(10);
        poll += 1;
        let staying = scan.step(&at(later, poll, mem[0].hz, -40.0), 0, &mem);
        assert_eq!(staying, Action::Idle);
        poll += 1;
        let resume = scan.step(&at(later, poll, mem[0].hz, -54.0), 0, &mem);
        assert_eq!(resume, Action::Tune { index: 1, resend: true });
    }

    #[test]
    fn dwell_resumes_five_seconds_after_a_hit_and_a_busy_single_memory_listens_again() {
        let mem = memories(1);
        let mut scan = Scanner::new();
        scan.dwell = true;
        scan.start(0, 0, mem[0].hz, false, false);
        let now = Instant::now();
        let mut poll = 0;
        // The only sample is the floor. The next one has to rise to open.
        let seeded = land(&mut scan, now, &mut poll, mem[0].hz, -54.0, &mem);
        assert_eq!(seeded, Action::Tune { index: 0, resend: false });
        poll += 1;
        let opened = scan.step(&at(now, poll, mem[0].hz, -40.0), 0, &mem);
        assert_eq!(opened, Action::Idle);
        let deadline = scan.dwell_deadline().expect("dwell");
        assert!(deadline >= now + DWELL);
        let same = scan.step(&at(now, poll, mem[0].hz, -40.0), 0, &mem);
        assert_eq!(same, Action::Idle);
        let resume = scan.step(&at(deadline, poll, mem[0].hz, -40.0), 0, &mem);
        assert_eq!(resume, Action::Tune { index: 0, resend: false });
        poll += 1;
        let again = scan.step(&at(deadline, poll, mem[0].hz, -40.0), 0, &mem);
        assert_eq!(again, Action::Idle);
        assert!(scan.dwell_deadline().expect("again") >= deadline + DWELL);
    }

    #[test]
    fn a_pass_that_opens_on_every_channel_raises_the_floor() {
        // Four of five sit well above the one quiet outlier.
        let levels = [-70.0, -40.0, -40.0, -40.0, -40.0];
        let mem = memories(levels.len());
        let mut scan = Scanner::new();
        scan.start(0, 0, mem[0].hz, false, false);
        let now = Instant::now();
        let mut poll = 0;
        let mut hz = mem[0].hz;
        for level in levels {
            let action = land(&mut scan, now, &mut poll, hz, level, &mem);
            if let Action::Tune { index, .. } = action {
                hz = mem[index].hz;
            }
        }
        let (floor, margin) = floor_margin(&scan);
        assert!((floor - -40.0).abs() < 0.01);
        assert!((margin - 7.0).abs() < 0.01);
    }

    #[test]
    fn a_moving_pass_that_never_opens_lowers_the_threshold() {
        let mem = memories(3);
        let mut scan = Scanner::new();
        scan.start(0, 0, mem[0].hz, false, false);
        let now = Instant::now();
        let mut poll = 0;
        let mut hz = mem[0].hz;
        for _ in 0..3 {
            let action = land(&mut scan, now, &mut poll, hz, -30.0, &mem);
            if let Action::Tune { index, .. } = action {
                hz = mem[index].hz;
            }
        }
        let (floor, margin) = floor_margin(&scan);
        assert!((floor - -30.0).abs() < 0.01);
        assert!((margin - 6.0).abs() < 0.01);
        // Entirely under the open line, and 4 dB from softest to loudest.
        let under = [-40.0, -36.0, -40.0];
        for level in under {
            let action = land(&mut scan, now, &mut poll, hz, level, &mem);
            assert!(matches!(action, Action::Tune { .. }), "{action:?}");
            if let Action::Tune { index, .. } = action {
                hz = mem[index].hz;
            }
        }
        let (floor, margin) = floor_margin(&scan);
        assert!((floor - -35.0).abs() < 0.01);
        assert!((margin - 5.0).abs() < 0.01);
    }

    #[test]
    fn a_flat_pass_leaves_the_floor() {
        let mem = memories(3);
        let mut scan = Scanner::new();
        scan.start(0, 0, mem[0].hz, false, false);
        let now = Instant::now();
        let mut poll = 0;
        let mut hz = mem[0].hz;
        for _ in 0..3 {
            let action = land(&mut scan, now, &mut poll, hz, -40.0, &mem);
            if let Action::Tune { index, .. } = action {
                hz = mem[index].hz;
            }
        }
        for _ in 0..3 {
            let action = land(&mut scan, now, &mut poll, hz, -40.0, &mem);
            assert!(matches!(action, Action::Tune { .. }));
            if let Action::Tune { index, .. } = action {
                hz = mem[index].hz;
            }
        }
        let (floor, margin) = floor_margin(&scan);
        assert!((floor - -40.0).abs() < 0.01);
        assert!((margin - 6.0).abs() < 0.01);
    }

    #[test]
    fn trunk_targets_are_the_control_memories_and_the_hop_wraps() {
        let listed = [
            Candidate {
                index: 0,
                hz: 100_000_000.0,
                control: false,
                tunable: true,
            },
            Candidate {
                index: 1,
                hz: 101_000_000.0,
                control: true,
                tunable: true,
            },
            Candidate {
                index: 2,
                hz: 102_000_000.0,
                control: true,
                tunable: false,
            },
            Candidate {
                index: 4,
                hz: 104_000_000.0,
                control: true,
                tunable: true,
            },
        ];
        assert_eq!(
            targets(0, &listed, true),
            vec![
                Memory {
                    zone: 0,
                    index: 1,
                    hz: 101_000_000.0
                },
                Memory {
                    zone: 0,
                    index: 4,
                    hz: 104_000_000.0
                },
            ]
        );
        let all = targets(0, &listed, false);
        assert_eq!(all.len(), 3);
        let voice_only = [
            Candidate {
                index: 0,
                hz: 100_000_000.0,
                control: false,
                tunable: true,
            },
            Candidate {
                index: 1,
                hz: 101_000_000.0,
                control: false,
                tunable: false,
            },
            Candidate {
                index: 2,
                hz: 102_000_000.0,
                control: false,
                tunable: true,
            },
        ];
        assert_eq!(
            targets(0, &voice_only, true),
            vec![
                Memory {
                    zone: 0,
                    index: 0,
                    hz: 100_000_000.0
                },
                Memory {
                    zone: 0,
                    index: 2,
                    hz: 102_000_000.0
                },
            ]
        );
        let mem = targets(0, &listed, true);
        let mut scan = Scanner::new();
        scan.start(0, 1, mem[0].hz, true, false);
        let now = Instant::now();
        let mut poll = 0;
        let hopped = land(&mut scan, now, &mut poll, mem[0].hz, -54.0, &mem);
        assert_eq!(hopped, Action::Tune { index: 4, resend: true });
        let wrapped = land(&mut scan, now, &mut poll, mem[1].hz, -54.0, &mem);
        assert_eq!(wrapped, Action::Tune { index: 1, resend: true });
    }

    #[test]
    fn one_memory_does_not_resend_the_frequency() {
        let mem = memories(1);
        let mut scan = Scanner::new();
        scan.start(0, 0, mem[0].hz, false, false);
        let now = Instant::now();
        let mut poll = 0;
        let first = land(&mut scan, now, &mut poll, mem[0].hz, -54.0, &mem);
        assert_eq!(first, Action::Tune { index: 0, resend: false });
        poll += 1;
        let again = scan.step(&at(now, poll, mem[0].hz, -54.0), 0, &mem);
        assert_eq!(again, Action::Tune { index: 0, resend: false });
        let repeat = scan.step(&at(now, poll, mem[0].hz, -54.0), 0, &mem);
        assert_eq!(repeat, Action::Idle);
    }
}

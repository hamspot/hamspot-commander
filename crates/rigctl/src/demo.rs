//! In-process rig frozen on the Ham Radio Deluxe FT-2000 demo face.

use std::collections::BTreeMap;

use crate::caps::{Caps, Range};
use crate::{
    Command, LinkKind, Rig, RigError, Snapshot, DEMO_FREQ_HZ, DEMO_MODE, DEMO_PASSBAND_HZ,
    DEMO_STRENGTH_DB,
};

#[derive(Clone, Debug)]
struct Vfo {
    freq_hz: f64,
    mode: String,
    passband_hz: i32,
}

#[derive(Clone, Debug)]
pub struct DemoRig {
    current: String,
    a: Vfo,
    b: Vfo,
    tx_vfo: String,
    rit_hz: i32,
    ptt: bool,
    power_on: bool,
    ant: i32,
    levels: BTreeMap<String, f64>,
    funcs: BTreeMap<String, bool>,
    caps: Caps,
}

impl DemoRig {
    pub fn new() -> Self {
        let mut levels = BTreeMap::new();
        levels.insert("AF".into(), 0.20);
        levels.insert("RF".into(), 1.0);
        levels.insert("RFPOWER".into(), 0.78);
        levels.insert("RFPOWER_METER".into(), 0.78);
        levels.insert("IF".into(), -200.0);
        levels.insert("NR".into(), 0.08);
        levels.insert("MICGAIN".into(), 0.50);
        levels.insert("VOXDELAY".into(), 1000.0);
        levels.insert("VOXGAIN".into(), 0.39);
        levels.insert("AGC".into(), 5.0);
        levels.insert("PREAMP".into(), 10.0);
        levels.insert("ALC".into(), 0.04);
        levels.insert("SWR".into(), 1.1);
        levels.insert("STRENGTH".into(), DEMO_STRENGTH_DB);

        let mut funcs = BTreeMap::new();
        for name in [
            "NR", "LOCK", "TUNER", "VOX", "SBKIN", "MON", "COMP", "NB", "FAGC", "ANF",
        ] {
            funcs.insert((*name).to_string(), false);
        }

        Self {
            current: "VFOA".into(),
            a: Vfo {
                freq_hz: DEMO_FREQ_HZ,
                mode: DEMO_MODE.into(),
                passband_hz: DEMO_PASSBAND_HZ,
            },
            b: Vfo {
                freq_hz: 7_074_000.0,
                mode: "LSB".into(),
                passband_hz: DEMO_PASSBAND_HZ,
            },
            tx_vfo: "VFOA".into(),
            rit_hz: 0,
            ptt: false,
            power_on: true,
            ant: 1,
            levels,
            funcs,
            caps: demo_caps(),
        }
    }

    fn current_mut(&mut self) -> &mut Vfo {
        if self.current == "VFOB" {
            &mut self.b
        } else {
            &mut self.a
        }
    }
}

impl Default for DemoRig {
    fn default() -> Self {
        Self::new()
    }
}

impl Rig for DemoRig {
    fn apply(&mut self, cmd: &Command) -> Result<(), RigError> {
        match cmd {
            Command::SetFreq(hz) => {
                self.current_mut().freq_hz = *hz;
            }
            Command::SetMode { mode, passband_hz } => {
                let vfo = self.current_mut();
                vfo.mode = mode.clone();
                vfo.passband_hz = *passband_hz;
            }
            Command::SetSubMode { mode, passband_hz } => {
                let vfo = if self.current == "VFOB" {
                    &mut self.a
                } else {
                    &mut self.b
                };
                vfo.mode = mode.clone();
                vfo.passband_hz = *passband_hz;
            }
            Command::SetVfo(name) => {
                if name == "VFOA" || name == "VFOB" {
                    self.current = name.clone();
                } else {
                    return Err(RigError::Remote(format!("demo has no {name}")));
                }
            }
            Command::VfoOp(op) if op == "XCHG" => {
                std::mem::swap(&mut self.a, &mut self.b);
            }
            Command::VfoOp(op) if op == "CPY" => {
                self.b = self.a.clone();
            }
            Command::VfoOp(op) => {
                return Err(RigError::Remote(format!("demo has no vfo op {op}")));
            }
            Command::SetRit(hz) => self.rit_hz = *hz,
            Command::SetLevel { name, value } => {
                if !self.caps.set_levels.contains_key(name) {
                    return Err(RigError::Remote(format!("demo has no level {name}")));
                }
                self.levels.insert(name.clone(), *value);
                if name == "RFPOWER" {
                    self.levels.insert("RFPOWER_METER".into(), *value);
                }
            }
            Command::SetFunc { name, on } => {
                if !self.caps.has_func(name) {
                    return Err(RigError::Remote(format!("demo has no func {name}")));
                }
                self.funcs.insert(name.clone(), *on);
            }
            Command::SetPtt(on) => self.ptt = *on,
            Command::SetAnt(n) => self.ant = *n,
            Command::SetSplit { on, tx_vfo } => {
                self.tx_vfo = if *on {
                    tx_vfo.clone()
                } else {
                    self.current.clone()
                };
            }
            Command::SetPower(on) => self.power_on = *on,
        }
        Ok(())
    }

    fn poll(&mut self) -> Result<Snapshot, RigError> {
        let (freq_hz, mode, passband_hz, sub_mode) = if self.current == "VFOB" {
            (
                self.b.freq_hz,
                self.b.mode.clone(),
                self.b.passband_hz,
                self.a.mode.clone(),
            )
        } else {
            (
                self.a.freq_hz,
                self.a.mode.clone(),
                self.a.passband_hz,
                self.b.mode.clone(),
            )
        };
        Ok(Snapshot {
            kind: LinkKind::Demo,
            model: "Demo".into(),
            endpoint: "in-process".into(),
            message: String::new(),
            freq_hz,
            mode,
            passband_hz,
            sub_mode,
            vfo: self.current.clone(),
            tx_vfo: self.tx_vfo.clone(),
            rit_hz: self.rit_hz,
            ptt: self.ptt,
            power_on: self.power_on,
            ant: self.ant,
            strength_db: self
                .levels
                .get("STRENGTH")
                .copied()
                .unwrap_or(DEMO_STRENGTH_DB),
            levels: self.levels.clone(),
            funcs: self.funcs.clone(),
            caps: self.caps.clone(),
            roofing: Some("15 kHz".into()),
        })
    }
}

fn range(min: f64, max: f64, step: f64) -> Range {
    Range { min, max, step }
}

fn demo_caps() -> Caps {
    let mut caps = Caps::empty();
    caps.model = "Demo".into();
    let sets = [
        ("AF", range(0.0, 1.0, 0.01)),
        ("RF", range(0.0, 1.0, 0.01)),
        ("RFPOWER", range(0.05, 1.0, 0.01)),
        ("IF", range(-10_000.0, 10_000.0, 10.0)),
        ("NR", range(0.0, 1.0, 0.01)),
        ("MICGAIN", range(0.0, 1.0, 0.01)),
        ("VOXDELAY", range(0.0, 3000.0, 10.0)),
        ("VOXGAIN", range(0.0, 1.0, 0.01)),
        ("AGC", range(0.0, 6.0, 1.0)),
        ("PREAMP", range(0.0, 20.0, 10.0)),
    ];
    for (name, span) in sets {
        caps.set_levels.insert(name.into(), span);
        caps.get_levels.insert(name.into(), span);
    }
    for (name, span) in [
        ("STRENGTH", range(-54.0, 60.0, 1.0)),
        ("ALC", range(0.0, 1.0, 0.01)),
        ("SWR", range(1.0, 10.0, 0.1)),
        ("RFPOWER_METER", range(0.0, 1.0, 0.01)),
    ] {
        caps.get_levels.insert(name.into(), span);
    }
    for name in [
        "NR", "LOCK", "TUNER", "VOX", "SBKIN", "MON", "COMP", "NB", "FAGC", "ANF",
    ] {
        caps.set_funcs.insert(name.into());
    }
    caps.modes = ["USB", "LSB", "CW", "AM", "FM", "RTTY"]
        .into_iter()
        .map(str::to_string)
        .collect();
    caps.vfos = vec!["VFOA".into(), "VFOB".into()];
    caps.vfo_ops = ["XCHG", "CPY"].into_iter().map(str::to_string).collect();
    caps.agc = vec![
        (0, "OFF".into()),
        (1, "SUPERFAST".into()),
        (2, "FAST".into()),
        (5, "MEDIUM".into()),
        (3, "SLOW".into()),
        (6, "AUTO".into()),
    ];
    caps.can_set_freq = true;
    caps.can_set_mode = true;
    caps.can_set_ant = true;
    caps.can_set_ptt = true;
    caps.can_set_rit = true;
    caps.can_set_power = true;
    caps.can_set_split = true;
    caps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_on_the_screenshot_frequency() {
        let snap = DemoRig::new().poll().unwrap();
        assert!((snap.freq_hz - DEMO_FREQ_HZ).abs() < 0.5);
        assert_eq!(snap.mode, "USB");
        assert!((snap.strength_db - 20.0).abs() < 0.1);
        assert_eq!(snap.roofing.as_deref(), Some("15 kHz"));
        assert_eq!(snap.sub_mode, "LSB");
    }

    #[test]
    fn exchange_moves_the_displayed_frequency() {
        let mut rig = DemoRig::new();
        rig.apply(&Command::VfoOp("XCHG".into())).unwrap();
        let snap = rig.poll().unwrap();
        assert!((snap.freq_hz - 7_074_000.0).abs() < 0.5);
        assert_eq!(snap.vfo, "VFOA");
        assert_eq!(snap.mode, "LSB");
    }

    #[test]
    fn slider_and_band_frequency_stick() {
        let mut rig = DemoRig::new();
        rig.apply(&Command::SetFreq(14_230_000.0)).unwrap();
        rig.apply(&Command::SetLevel {
            name: "AF".into(),
            value: 0.42,
        })
        .unwrap();
        let snap = rig.poll().unwrap();
        assert!((snap.freq_hz - 14_230_000.0).abs() < 0.5);
        assert!((snap.levels["AF"] - 0.42).abs() < 1e-6);
        assert!((snap.levels["RFPOWER_METER"] - 0.78).abs() < 1e-6);
    }
}

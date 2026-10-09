//! Capability lines from `rigctld`'s `dump_caps` reply.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

impl Range {
    pub fn usable(self) -> bool {
        (self.max - self.min).abs() > 1e-6
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Caps {
    pub model: String,
    pub set_levels: BTreeMap<String, Range>,
    pub get_levels: BTreeMap<String, Range>,
    pub set_funcs: BTreeSet<String>,
    pub modes: Vec<String>,
    pub vfos: Vec<String>,
    pub vfo_ops: BTreeSet<String>,
    pub agc: Vec<(i32, String)>,
    pub can_set_freq: bool,
    pub can_set_mode: bool,
    pub can_set_ant: bool,
    pub can_set_ptt: bool,
    pub can_set_rit: bool,
    pub can_set_power: bool,
    pub can_set_split: bool,
}

impl Caps {
    pub fn empty() -> Self {
        Self {
            model: String::new(),
            set_levels: BTreeMap::new(),
            get_levels: BTreeMap::new(),
            set_funcs: BTreeSet::new(),
            modes: Vec::new(),
            vfos: Vec::new(),
            vfo_ops: BTreeSet::new(),
            agc: Vec::new(),
            can_set_freq: false,
            can_set_mode: false,
            can_set_ant: false,
            can_set_ptt: false,
            can_set_rit: false,
            can_set_power: false,
            can_set_split: false,
        }
    }

    pub fn has_func(&self, name: &str) -> bool {
        self.set_funcs.contains(name)
    }

    pub fn level_range(&self, name: &str) -> Option<Range> {
        self.set_levels.get(name).copied().filter(|r| r.usable())
    }
}

/// Read the text lines of a `dump_caps` reply. Missing "Can set" lines stay false.
pub fn parse_caps(lines: &[String]) -> Caps {
    let mut caps = Caps::empty();
    for line in lines {
        let line = line.trim();
        if let Some(rest) = strip_field(line, "Model name") {
            caps.model = rest.to_string();
        } else if let Some(rest) = strip_field(line, "Set level") {
            caps.set_levels = parse_level_list(rest);
        } else if let Some(rest) = strip_field(line, "Get level") {
            caps.get_levels = parse_level_list(rest);
        } else if let Some(rest) = strip_field(line, "Set functions") {
            caps.set_funcs = tokens(rest);
        } else if let Some(rest) = strip_field(line, "Mode list") {
            caps.modes = words(rest);
        } else if let Some(rest) = strip_field(line, "VFO list") {
            caps.vfos = words(rest);
        } else if let Some(rest) = strip_field(line, "VFO Ops") {
            caps.vfo_ops = tokens(rest);
        } else if let Some(rest) = strip_field(line, "AGC levels") {
            caps.agc = parse_agc(rest);
        } else if let Some(rest) = strip_field(line, "Can set Frequency") {
            caps.can_set_freq = yes(rest);
        } else if let Some(rest) = strip_field(line, "Can set Mode") {
            caps.can_set_mode = yes(rest);
        } else if let Some(rest) = strip_field(line, "Can set Ant") {
            caps.can_set_ant = yes(rest);
        } else if let Some(rest) = strip_field(line, "Can set PTT") {
            caps.can_set_ptt = yes(rest);
        } else if let Some(rest) = strip_field(line, "Can set RIT") {
            caps.can_set_rit = yes(rest);
        } else if let Some(rest) = strip_field(line, "Can set Power Stat") {
            caps.can_set_power = yes(rest);
        } else if let Some(rest) = strip_field(line, "Can set Split VFO") {
            caps.can_set_split = yes(rest);
        }
    }
    caps
}

fn strip_field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(key)?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix(':').unwrap_or(rest);
    Some(rest.trim())
}

fn yes(value: &str) -> bool {
    value.starts_with('Y') || value.starts_with('y')
}

fn tokens(rest: &str) -> BTreeSet<String> {
    words(rest).into_iter().collect()
}

fn words(rest: &str) -> Vec<String> {
    rest.split_whitespace()
        .filter(|tok| tok.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .map(str::to_string)
        .collect()
}

fn parse_agc(rest: &str) -> Vec<(i32, String)> {
    let mut out = Vec::new();
    for tok in rest.split_whitespace() {
        let Some((id, name)) = tok.split_once('=') else {
            continue;
        };
        let Ok(id) = id.parse::<i32>() else {
            continue;
        };
        if !name.is_empty() {
            out.push((id, name.to_string()));
        }
    }
    out
}

fn parse_level_list(rest: &str) -> BTreeMap<String, Range> {
    let mut out = BTreeMap::new();
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            let mut start = i;
            while start > 0 && is_name(bytes[start - 1]) {
                start -= 1;
            }
            let name = &rest[start..i];
            if let Some(end) = rest[i + 1..].find(')') {
                if let Some(range) = parse_range(&rest[i + 1..i + 1 + end]) {
                    if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                        out.insert(name.to_string(), range);
                    }
                }
                i += end + 1;
            }
        }
        i += 1;
    }
    out
}

fn is_name(b: u8) -> bool {
    b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'
}

fn parse_range(inside: &str) -> Option<Range> {
    let (minmax, step) = inside.split_once('/')?;
    let (min, max) = minmax.split_once("..")?;
    Some(Range {
        min: min.parse().ok()?,
        max: max.parse().ok()?,
        step: step.parse().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(s: &str) -> String {
        s.to_string()
    }

    #[test]
    fn parses_a_trimmed_dump() {
        let lines = vec![
            line("dump_caps:"),
            line("Model name:\tDummy"),
            line("Set level: AF(0.000000..0.000000/0.000000) RF(0.000000..1.000000/0.003922) RFPOWER(0.050000..1.000000/0.001957) IF(-10000..10000/10) STRENGTH(0..0/0)"),
            line("Get level: RF(0.000000..1.000000/0.003922) STRENGTH(0..0/0) ALC(0.000000..1.000000/0.010000) SWR(1.000000..10.000000/0.1) RFPOWER_METER(0.000000..1.000000/0.003922)"),
            line("Set functions: NR LOCK TUNER VOX "),
            line("Mode list: AM CW USB LSB FM"),
            line("VFO list: VFOA VFOB MEM"),
            line("VFO Ops: CPY XCHG"),
            line("AGC levels: 0=OFF 5=MEDIUM 6=AUTO"),
            line("Can set Frequency:\tY"),
            line("Can set Mode:\tY"),
            line("Can set Ant:\tN"),
            line("Can set PTT:\tY"),
            line("Can set RIT:\tY"),
            line("Can set Power Stat:\tY"),
            line("Can set Split VFO:\tN"),
        ];
        let caps = parse_caps(&lines);
        assert_eq!(caps.model, "Dummy");
        assert!(!caps.level_range("AF").is_some());
        let rf = caps.level_range("RF").unwrap();
        assert!((rf.max - 1.0).abs() < 1e-6);
        assert!(caps.level_range("IF").unwrap().usable());
        assert!(caps.level_range("STRENGTH").is_none());
        assert!(caps.get_levels.contains_key("STRENGTH"));
        assert!(caps.has_func("NR"));
        assert_eq!(caps.modes, ["AM", "CW", "USB", "LSB", "FM"]);
        assert!(caps.vfo_ops.contains("XCHG"));
        assert_eq!(caps.agc.len(), 3);
        assert!(caps.can_set_freq);
        assert!(!caps.can_set_ant);
        assert!(!caps.can_set_split);
    }

    #[test]
    fn scientific_step_parses() {
        let map = parse_level_list("PREAMP(0..20/1.4013e-45)");
        let range = map["PREAMP"];
        assert!((range.max - 20.0).abs() < 1e-6);
        assert!(range.step.abs() < 1e-30);
    }
}

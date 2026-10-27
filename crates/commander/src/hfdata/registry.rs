//! Plug-in registry for HF data decoders.
//!
//! Each mode issue (#206-#216) adds one [`DecodeMode`] and registers a
//! factory for it. The decoder itself is an external rxpipe program; its
//! stdout lines arrive on a channel and the mode turns each line into a
//! [`Decode`] row. Modes without a decoder stay listed so the window can say
//! plainly that none is available.

use std::sync::mpsc::{Receiver, TryRecvError};

use super::decodes::{Decode, DecodeList, Pushed};
use super::slots::{self, SlotLen};

/// What one decoder output line meant.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    Decode(Decode),
    /// Banner, progress or blank line.
    Ignored,
    /// A line the decoder flagged as an error, shown to the operator.
    Error(String),
}

pub trait DecodeMode: Send {
    fn mode(&self) -> &str;
    fn parse_line(&mut self, line: &str, now_ms: i64) -> Line;
}

pub type Factory = fn() -> Box<dyn DecodeMode>;

#[derive(Clone, Debug, PartialEq)]
pub struct ModeInfo {
    pub id: &'static str,
    pub name: &'static str,
    /// `None` for free-running modes (PSK31, RTTY, CW).
    pub slot: Option<SlotLen>,
}

pub struct Entry {
    pub info: ModeInfo,
    pub factory: Option<Factory>,
}

#[derive(Default)]
pub struct Registry {
    entries: Vec<Entry>,
}

impl Registry {
    /// The slotted modes the window knows about. No decoder is shipped yet.
    pub fn builtin() -> Self {
        let mut r = Registry::default();
        for (id, name, slot) in [
            ("FT8", "FT8", Some(SlotLen::FT8)),
            ("FT4", "FT4", Some(SlotLen::FT4)),
            ("WSPR", "WSPR", Some(SlotLen::WSPR)),
            ("JS8", "JS8 (Normal)", Some(SlotLen::FT8)),
        ] {
            r.add_mode(ModeInfo { id, name, slot }).expect("unique built-in mode");
        }
        r
    }

    /// List a mode with no decoder yet.
    pub fn add_mode(&mut self, info: ModeInfo) -> Result<(), String> {
        if self.get(info.id).is_some() {
            return Err(format!("mode {} is already listed", info.id));
        }
        self.entries.push(Entry { info, factory: None });
        Ok(())
    }

    /// Attach a decoder to a listed mode, or list it with one.
    pub fn register(&mut self, info: ModeInfo, factory: Factory) -> Result<(), String> {
        if let Some(e) = self.entries.iter_mut().find(|e| e.info.id == info.id) {
            if e.factory.is_some() {
                return Err(format!("mode {} already has a decoder", info.id));
            }
            e.info = info;
            e.factory = Some(factory);
            return Ok(());
        }
        self.entries.push(Entry { info, factory: Some(factory) });
        Ok(())
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.info.id.eq_ignore_ascii_case(id))
    }

    /// A fresh decoder for `id`, or the text the window shows instead.
    pub fn decoder(&self, id: &str) -> Result<Box<dyn DecodeMode>, String> {
        match self.get(id) {
            None => Err(format!("{id} is not a known mode.")),
            Some(Entry { factory: None, info }) => Err(format!(
                "No {} decoder in this version of Commander. Slot timing and presets work; decodes will appear here once a decoder is added.",
                info.name
            )),
            Some(Entry { factory: Some(f), .. }) => Ok(f()),
        }
    }
}

/// Parse the common WSJT-X style line `HHMMSS SNR DT FREQ ~ MESSAGE`
/// (also `HHMM` for WSPR-style output). Shared by the FT8/FT4/JS8 plug-ins.
pub fn parse_wsjt_line(line: &str, mode: &str, now_ms: i64) -> Line {
    let mut it = line.split_whitespace();
    let Some(stamp) = it.next() else { return Line::Ignored };
    let Some((h, m, s)) = slots::parse_hhmmss(stamp) else { return Line::Ignored };
    let (Some(snr), Some(dt), Some(freq)) = (it.next(), it.next(), it.next()) else {
        return Line::Ignored;
    };
    let (Ok(snr), Ok(dt), Ok(freq)) = (snr.parse::<f32>(), dt.parse::<f32>(), freq.parse::<f64>()) else {
        return Line::Ignored;
    };
    let rest: Vec<&str> = it.collect();
    let rest = match rest.first() {
        Some(&"~") | Some(&"+") | Some(&"#") | Some(&"@") => &rest[1..],
        _ => &rest[..],
    };
    let text = rest.join(" ");
    if text.is_empty() {
        return Line::Ignored;
    }
    Line::Decode(Decode {
        utc_ms: slots::resolve_time_of_day(h, m, s, now_ms),
        offset_hz: freq,
        snr_db: Some(snr),
        dt_s: Some(dt),
        mode: mode.to_string(),
        text,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub enum SourceStatus {
    Running,
    /// The decoder's output closed; shown as text.
    Ended(String),
}

/// One running decoder: its line feed and its parser.
pub struct Session {
    decoder: Box<dyn DecodeMode>,
    lines: Receiver<String>,
    pub status: SourceStatus,
    pub last_error: Option<String>,
}

impl Session {
    pub fn new(decoder: Box<dyn DecodeMode>, lines: Receiver<String>) -> Self {
        Self { decoder, lines, status: SourceStatus::Running, last_error: None }
    }

    pub fn mode(&self) -> &str {
        self.decoder.mode()
    }

    /// Move waiting lines into `list`. Returns rows added.
    pub fn pump(&mut self, list: &mut DecodeList, now_ms: i64) -> usize {
        let mut added = 0;
        loop {
            match self.lines.try_recv() {
                Ok(line) => match self.decoder.parse_line(&line, now_ms) {
                    Line::Decode(d) => {
                        if list.push(d, now_ms) == Pushed::Added {
                            added += 1;
                        }
                    }
                    Line::Error(e) => self.last_error = Some(e),
                    Line::Ignored => {}
                },
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.status == SourceStatus::Running {
                        self.status = SourceStatus::Ended("the decoder stopped sending output".into());
                    }
                    break;
                }
            }
        }
        added
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// TEST-ONLY feeder: replays canned WSJT-X style lines. Not a decoder.
    struct CannedFt8;
    impl DecodeMode for CannedFt8 {
        fn mode(&self) -> &str {
            "FT8"
        }
        fn parse_line(&mut self, line: &str, now_ms: i64) -> Line {
            if let Some(e) = line.strip_prefix("ERROR ") {
                return Line::Error(e.to_string());
            }
            parse_wsjt_line(line, "FT8", now_ms)
        }
    }
    fn canned() -> Box<dyn DecodeMode> {
        Box::new(CannedFt8)
    }

    const MIDNIGHT: i64 = 1_793_059_200_000;

    #[test]
    fn builtin_modes_have_no_decoder_and_say_so() {
        let r = Registry::builtin();
        let ids: Vec<_> = r.entries().iter().map(|e| e.info.id).collect();
        assert_eq!(ids, ["FT8", "FT4", "WSPR", "JS8"]);
        let err = r.decoder("FT8").err().expect("no decoder yet");
        assert!(err.contains("No FT8 decoder"));
        assert!(r.decoder("NOPE").is_err());
        assert_eq!(r.get("ft4").unwrap().info.slot, Some(SlotLen::FT4));
    }

    #[test]
    fn a_plugin_registers_once() {
        let mut r = Registry::builtin();
        let info = r.get("FT8").unwrap().info.clone();
        r.register(info.clone(), canned).unwrap();
        assert!(r.decoder("FT8").is_ok());
        assert!(r.register(info, canned).is_err());
        assert!(r.add_mode(ModeInfo { id: "FT8", name: "x", slot: None }).is_err());
        r.register(ModeInfo { id: "RTTY", name: "RTTY", slot: None }, canned).unwrap();
        assert_eq!(r.entries().len(), 5);
    }

    #[test]
    fn wsjt_lines_parse() {
        let now = MIDNIGHT + 20_000;
        let Line::Decode(d) = parse_wsjt_line("000015 -12  0.3 1234 ~  CQ K1ABC FN42", "FT8", now) else {
            panic!("decode")
        };
        assert_eq!(d.utc_ms, MIDNIGHT + 15_000);
        assert_eq!(d.snr_db, Some(-12.0));
        assert_eq!(d.dt_s, Some(0.3));
        assert_eq!(d.offset_hz, 1234.0);
        assert_eq!(d.text, "CQ K1ABC FN42");
        assert_eq!(parse_wsjt_line("<DecodeFinished>", "FT8", now), Line::Ignored);
        assert_eq!(parse_wsjt_line("", "FT8", now), Line::Ignored);
        assert_eq!(parse_wsjt_line("000015 -12 0.3 1234 ~", "FT8", now), Line::Ignored);
    }

    /// Acceptance for #217: canned decoder output becomes list rows.
    #[test]
    fn canned_output_fills_the_list_across_midnight() {
        let mut r = Registry::builtin();
        let info = r.get("FT8").unwrap().info.clone();
        r.register(info, canned).unwrap();
        let (tx, rx) = mpsc::channel();
        let mut s = Session::new(r.decoder("FT8").unwrap(), rx);
        for l in [
            "235945  -5  0.1  800 ~  CQ W1AW FN31",
            "000000 -18 -0.4 1500 ~  K1ABC W1AW -15",
            "000000 -17 -0.4 1502 ~  K1ABC W1AW -15",
            "ERROR sound card overrun",
            "garbage",
        ] {
            tx.send(l.to_string()).unwrap();
        }
        let mut list = DecodeList::new(50);
        let now = MIDNIGHT + 3_000;
        assert_eq!(s.pump(&mut list, now), 2);
        let rows = list.rows();
        assert_eq!(rows[0].text, "K1ABC W1AW -15");
        assert_eq!(rows[0].utc_ms, MIDNIGHT);
        assert_eq!(rows[0].snr_db, Some(-17.0));
        assert_eq!(rows[1].utc_ms, MIDNIGHT - 15_000);
        assert_eq!(s.last_error.as_deref(), Some("sound card overrun"));
        assert_eq!(s.status, SourceStatus::Running);
        drop(tx);
        s.pump(&mut list, now);
        assert!(matches!(s.status, SourceStatus::Ended(_)));
    }
}

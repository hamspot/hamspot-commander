//! Is this machine's clock good enough for slotted modes? FT8-style decoders
//! need UTC within about a second. The answer is always shown as text; a
//! clock that cannot be checked is reported, never assumed good.

use std::process::Command;

/// Offset (seconds) above which the window shows a clock warning.
pub const WARN_S: f64 = 1.0;

#[derive(Clone, Debug, PartialEq)]
pub enum ClockState {
    /// No answer yet from the first check.
    Checking,
    Synced { offset_s: Option<f64>, source: &'static str },
    NotSynced { source: &'static str, detail: String },
    Unknown(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub warn: bool,
    pub text: String,
}

/// `chronyc -c tracking`: field 5 is the system time offset in seconds,
/// the last field is the leap status (`Normal`, `Not synchronised`, ...).
pub fn parse_chrony_csv(text: &str) -> Result<ClockState, String> {
    let line = text.lines().find(|l| !l.trim().is_empty()).ok_or("chronyc printed nothing")?;
    let fields: Vec<&str> = line.split(',').collect();
    if fields.len() < 6 {
        return Err(format!("chronyc output not understood: {}", line.trim()));
    }
    let leap = fields.last().map(|s| s.trim()).unwrap_or("");
    if leap.eq_ignore_ascii_case("Not synchronised") || leap.eq_ignore_ascii_case("Not synchronized") {
        return Ok(ClockState::NotSynced {
            source: "chrony",
            detail: "chrony has no time source".into(),
        });
    }
    let offset: f64 = fields[4]
        .trim()
        .parse()
        .map_err(|_| format!("chronyc offset not a number: {}", fields[4].trim()))?;
    if !offset.is_finite() {
        return Err("chronyc offset not finite".into());
    }
    Ok(ClockState::Synced { offset_s: Some(offset), source: "chrony" })
}

/// `timedatectl show -p NTPSynchronized --value`.
pub fn parse_timedatectl(text: &str) -> Option<ClockState> {
    match text.trim() {
        "yes" => Some(ClockState::Synced { offset_s: None, source: "timedatectl" }),
        "no" => Some(ClockState::NotSynced {
            source: "timedatectl",
            detail: "the system reports NTP is not synchronised".into(),
        }),
        _ => None,
    }
}

pub fn verdict(state: &ClockState) -> Verdict {
    match state {
        ClockState::Checking => Verdict { warn: false, text: "Clock: checking…".into() },
        ClockState::Synced { offset_s: Some(o), source } if o.abs() > WARN_S => Verdict {
            warn: true,
            text: format!(
                "CLOCK WARNING: off by {o:+.2} s ({source}). Slotted modes need UTC within {WARN_S:.0} s; decodes will be missed."
            ),
        },
        ClockState::Synced { offset_s: Some(o), source } => Verdict {
            warn: false,
            text: format!("Clock OK: offset {o:+.3} s ({source})"),
        },
        ClockState::Synced { offset_s: None, source } => Verdict {
            warn: false,
            text: format!("Clock synchronised ({source}); offset not reported"),
        },
        ClockState::NotSynced { source, detail } => Verdict {
            warn: true,
            text: format!("CLOCK WARNING: not synchronised ({source}): {detail}. Slot timing may be wrong."),
        },
        ClockState::Unknown(why) => Verdict {
            warn: true,
            text: format!("CLOCK WARNING: cannot verify the clock: {why}. Slot timing may be wrong."),
        },
    }
}

/// Ask chrony, then timedatectl. Blocking: run it off the UI thread.
pub fn probe() -> ClockState {
    let mut why = Vec::new();
    match Command::new("chronyc").args(["-c", "tracking"]).output() {
        Ok(out) if out.status.success() => {
            match parse_chrony_csv(&String::from_utf8_lossy(&out.stdout)) {
                Ok(state) => return state,
                Err(e) => why.push(e),
            }
        }
        Ok(out) => why.push(format!("chronyc failed ({})", out.status)),
        Err(e) => why.push(format!("chronyc not available ({e})")),
    }
    match Command::new("timedatectl")
        .args(["show", "-p", "NTPSynchronized", "--value"])
        .output()
    {
        Ok(out) if out.status.success() => {
            if let Some(state) = parse_timedatectl(&String::from_utf8_lossy(&out.stdout)) {
                return state;
            }
            why.push("timedatectl answer not understood".into());
        }
        Ok(out) => why.push(format!("timedatectl failed ({})", out.status)),
        Err(e) => why.push(format!("timedatectl not available ({e})")),
    }
    ClockState::Unknown(why.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "A9FEA97B,169.254.169.123,4,1793059200.123,0.000012345,-0.000001,0.000020,-12.3,0.001,0.02,0.0001,0.0002,64.0,Normal\n";

    #[test]
    fn chrony_good_offset_is_ok() {
        let s = parse_chrony_csv(GOOD).unwrap();
        let v = verdict(&s);
        assert!(!v.warn);
        assert!(v.text.starts_with("Clock OK"));
    }

    #[test]
    fn large_offset_warns_with_text() {
        let s = ClockState::Synced { offset_s: Some(-1.5), source: "chrony" };
        let v = verdict(&s);
        assert!(v.warn);
        assert!(v.text.contains("-1.50 s"));
        // Right at the limit is still fine.
        assert!(!verdict(&ClockState::Synced { offset_s: Some(1.0), source: "chrony" }).warn);
    }

    #[test]
    fn chrony_not_synchronised_warns() {
        let line = "00000000,,0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,Not synchronised\n";
        let s = parse_chrony_csv(line).unwrap();
        assert!(matches!(s, ClockState::NotSynced { .. }));
        assert!(verdict(&s).warn);
    }

    #[test]
    fn garbage_is_an_error_not_ok() {
        assert!(parse_chrony_csv("").is_err());
        assert!(parse_chrony_csv("506 Cannot talk to daemon").is_err());
        assert!(parse_chrony_csv("a,b,c,d,notanumber,f,Normal").is_err());
    }

    #[test]
    fn timedatectl_answers() {
        assert!(!verdict(&parse_timedatectl("yes\n").unwrap()).warn);
        assert!(verdict(&parse_timedatectl("no\n").unwrap()).warn);
        assert!(parse_timedatectl("maybe").is_none());
    }

    #[test]
    fn unknown_always_warns() {
        let v = verdict(&ClockState::Unknown("chronyc not available".into()));
        assert!(v.warn);
        assert!(v.text.contains("cannot verify"));
    }
}

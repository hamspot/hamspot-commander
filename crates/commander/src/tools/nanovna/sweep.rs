//! Pure sweep-setup logic for the VNA SWEEP panel: center and span, the
//! clamps the NanoVNA V2.2 needs, and typed frequencies.
//! Issue 68. Nothing here touches egui.

/// Center and span of a start/stop pair.
pub fn center_span(start: f64, stop: f64) -> (f64, f64) {
    ((start + stop) / 2.0, stop - start)
}

/// Start and stop of a center/span pair.
pub fn start_stop(center: f64, span: f64) -> (f64, f64) {
    let half = span.abs() / 2.0;
    (center - half, center + half)
}

/// A sweep after the device limits, with one plain note per value changed.
#[derive(Clone, Debug, PartialEq)]
pub struct Clamped {
    pub start: f64,
    pub stop: f64,
    pub points: u16,
    pub notes: Vec<String>,
}

/// Clamp to the board: start at or above `floor`, stop at or below `max_hz`,
/// stop above start, and 2..=`max_points` points.
pub fn clamp(start: f64, stop: f64, points: i64, floor: f64, max_hz: f64, max_points: u16) -> Clamped {
    let mut notes = Vec::new();
    let fmt = super::display::fmt_hz;
    let mut a = start.round();
    let mut b = stop.round();
    if !a.is_finite() || a < floor {
        notes.push(format!("START raised to {} (board floor)", fmt(floor)));
        a = floor;
    }
    if !b.is_finite() || b > max_hz {
        notes.push(format!("STOP lowered to {} (board maximum)", fmt(max_hz)));
        b = max_hz;
    }
    if a >= max_hz {
        a = max_hz - 1e6;
        notes.push(format!("START lowered to {}", fmt(a)));
    }
    if b <= a {
        b = (a + 1e6).min(max_hz);
        notes.push(format!("STOP moved above START to {}", fmt(b)));
    }
    let p = points.clamp(2, max_points as i64) as u16;
    if p as i64 != points {
        notes.push(format!("POINTS set to {p} (2 to {max_points})"));
    }
    Clamped { start: a, stop: b, points: p, notes }
}

/// "14.2 MHz", "500k", "1.2G", "7074000 Hz". A bare number is MHz, which is
/// what the panel hint says.
pub fn parse_freq(text: &str) -> Option<f64> {
    let t = text.trim().to_ascii_lowercase().replace(' ', "");
    let t = t.trim_end_matches("hz");
    let (num, k) = match t.chars().last()? {
        'k' => (&t[..t.len() - 1], 1e3),
        'm' => (&t[..t.len() - 1], 1e6),
        'g' => (&t[..t.len() - 1], 1e9),
        _ if text.to_ascii_lowercase().trim_end().ends_with("hz") => (t, 1.0),
        _ => (t, 1e6),
    };
    let v: f64 = num.parse().ok()?;
    (v.is_finite() && v >= 0.0).then_some(v * k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_and_span_round_trip() {
        assert_eq!(center_span(1e6, 30e6), (15.5e6, 29e6));
        assert_eq!(start_stop(15.5e6, 29e6), (1e6, 30e6));
        assert_eq!(start_stop(14.175e6, -350e3), (14.0e6, 14.35e6));
    }

    #[test]
    fn invalid_values_are_clamped_with_a_note() {
        let c = clamp(10e3, 4e9, 5000, 50e3, 3e9, 1024);
        assert_eq!((c.start, c.stop, c.points), (50e3, 3e9, 1024));
        assert_eq!(c.notes.len(), 3);
        assert!(c.notes[0].contains("START"));
        let ok = clamp(1e6, 30e6, 201, 50e3, 3e9, 1024);
        assert!(ok.notes.is_empty());
        let flipped = clamp(30e6, 1e6, 1, 50e3, 3e9, 1024);
        assert!(flipped.stop > flipped.start);
        assert_eq!(flipped.points, 2);
    }

    #[test]
    fn typed_frequencies_take_units_and_bare_numbers_are_mhz() {
        assert_eq!(parse_freq("14.2"), Some(14.2e6));
        assert_eq!(parse_freq("14.2 MHz"), Some(14.2e6));
        assert_eq!(parse_freq("500k"), Some(500e3));
        assert_eq!(parse_freq("1.2 GHz"), Some(1.2e9));
        assert_eq!(parse_freq("7074000 Hz"), Some(7.074e6));
        assert_eq!(parse_freq("abc"), None);
    }
}

//! Red seven-segment AZ/EL readouts (issue #85). Formatting is pure and tested.
//! The painter draws lit segments in red and unlit ones as dim ghosts.

use super::geom::{self, StopAt};

/// Azimuth shown on the AZ readout. With a north stop the G-1000 travel is
/// 0..450, so the overlap reads 360.0..450.0. With the factory south stop the
/// rotctld value is -180..270. The readout shows the compass bearing and the
/// OVL annunciator says which pass it is.
pub fn display_az(az: f64, stop: StopAt) -> f64 {
    match stop {
        StopAt::North => az,
        StopAt::South => geom::bearing(az),
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// `360.0` style: three digits and a tenth, right aligned in five characters.
/// A bearing (`wrap`) that rounds up to 360.0 reads 0.0. A travel value does
/// not wrap. Beyond -99.9..999.9 the readout shows `OVr`.
pub fn format_az(az: Option<f64>, wrap: bool) -> String {
    let Some(az) = az.filter(|v| v.is_finite()) else {
        return "---.-".into();
    };
    let mut v = round1(az);
    if wrap && v >= 360.0 {
        v -= 360.0;
    }
    if v == 0.0 {
        v = 0.0; // no "-0.0"
    }
    if !(-99.9..=999.9).contains(&v) {
        return "  OVr".into();
    }
    format!("{v:>5.1}")
}

/// `90.0` style: two digits and a tenth in four characters, or `135.0` when a
/// flip-capable rotator is past 90. Below -9.9 or above 180.0 shows `OVr`.
pub fn format_el(el: Option<f64>) -> String {
    let Some(el) = el.filter(|v| v.is_finite()) else {
        return "--.-".into();
    };
    let mut v = round1(el);
    if v == 0.0 {
        v = 0.0;
    }
    if !(-9.9..=180.0).contains(&v) {
        return " OVr".into();
    }
    format!("{v:>4.1}")
}

/// Elevation past straight up: the antenna has flipped over.
pub fn el_flipped(el: f64) -> bool {
    round1(el) > 90.0
}

/// Segments a..g as bits 0..6 (a top, b top right, c bottom right, d bottom,
/// e bottom left, f top left, g middle).
pub fn segments(c: char) -> u8 {
    match c {
        '0' => 0b011_1111,
        '1' => 0b000_0110,
        '2' => 0b101_1011,
        '3' => 0b100_1111,
        '4' => 0b110_0110,
        '5' => 0b110_1101,
        '6' => 0b111_1101,
        '7' => 0b000_0111,
        '8' => 0b111_1111,
        '9' => 0b110_1111,
        '-' => 0b100_0000,
        'O' => 0b011_1111,
        'V' => 0b011_1110,
        'r' => 0b101_0000,
        _ => 0,
    }
}

/// Digit cells with their decimal point: "360.0" is [('3',false),('6',false),('0',true),('0',false)].
pub fn cells(text: &str) -> Vec<(char, bool)> {
    let mut out: Vec<(char, bool)> = Vec::new();
    for c in text.chars() {
        if c == '.' {
            if let Some(last) = out.last_mut() {
                last.1 = true;
                continue;
            }
            out.push((' ', true));
        } else {
            out.push((c, false));
        }
    }
    out
}

pub const LIT: egui::Color32 = egui::Color32::from_rgb(0xFF, 0x2A, 0x1A);
pub const GHOST: egui::Color32 = egui::Color32::from_rgb(0x30, 0x0C, 0x0A);
const WINDOW: egui::Color32 = egui::Color32::from_rgb(0x0A, 0x04, 0x04);

/// Paint `text` as seven-segment digits in `rect`, slanted a little.
pub fn paint(painter: &egui::Painter, rect: egui::Rect, text: &str, color: egui::Color32) {
    painter.rect_filled(rect, egui::CornerRadius::same(3), WINDOW);
    let cells = cells(text);
    if cells.is_empty() {
        return;
    }
    let pad = rect.height() * 0.14;
    let inner = rect.shrink(pad);
    let cell_w = inner.width() / cells.len() as f32;
    let h = inner.height();
    let w = (cell_w * 0.72).min(h * 0.55);
    let th = (h * 0.11).max(1.5);
    let slant = h * 0.08;
    for (i, (c, dp)) in cells.iter().enumerate() {
        let x0 = inner.left() + cell_w * i as f32 + (cell_w - w) * 0.35;
        let top = inner.top();
        let mid = top + h * 0.5;
        let bot = inner.bottom();
        // Shift x by slant: higher points lean right.
        let p = |x: f32, y: f32| egui::pos2(x + slant * (bot - y) / h, y);
        let segs = [
            [p(x0, top), p(x0 + w, top)],
            [p(x0 + w, top), p(x0 + w, mid)],
            [p(x0 + w, mid), p(x0 + w, bot)],
            [p(x0, bot), p(x0 + w, bot)],
            [p(x0, mid), p(x0, bot)],
            [p(x0, top), p(x0, mid)],
            [p(x0, mid), p(x0 + w, mid)],
        ];
        let bits = segments(*c);
        for (n, seg) in segs.iter().enumerate() {
            let on = bits & (1 << n) != 0;
            let shrink = th * 0.6;
            let d = (seg[1] - seg[0]).normalized() * shrink;
            painter.line_segment(
                [seg[0] + d, seg[1] - d],
                egui::Stroke::new(th, if on { color } else { GHOST }),
            );
        }
        let dot = p(x0 + w + th * 1.4, bot);
        painter.circle_filled(dot, th * 0.6, if *dp { color } else { GHOST });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn azimuth_reads_three_digits_and_a_tenth() {
        assert_eq!(format_az(None, true), "---.-");
        assert_eq!(format_az(Some(5.0), true), "  5.0");
        assert_eq!(format_az(Some(359.94), true), "359.9");
        assert_eq!(format_az(Some(359.96), true), "  0.0");
        assert_eq!(format_az(Some(359.96), false), "360.0");
        assert_eq!(format_az(Some(400.0), false), "400.0");
        assert_eq!(format_az(Some(450.0), false), "450.0");
        assert_eq!(format_az(Some(-12.34), false), "-12.3");
        assert_eq!(format_az(Some(-0.04), false), "  0.0");
        assert_eq!(format_az(Some(-180.0), false), "  OVr");
        assert_eq!(format_az(Some(f64::NAN), false), "---.-");
    }

    #[test]
    fn elevation_reads_two_digits_or_flip() {
        assert_eq!(format_el(None), "--.-");
        assert_eq!(format_el(Some(90.0)), "90.0");
        assert_eq!(format_el(Some(9.04)), " 9.0");
        assert_eq!(format_el(Some(89.96)), "90.0");
        assert_eq!(format_el(Some(135.0)), "135.0");
        assert_eq!(format_el(Some(-2.5)), "-2.5");
        assert_eq!(format_el(Some(200.0)), " OVr");
        assert!(el_flipped(90.2));
        assert!(!el_flipped(90.04));
    }

    #[test]
    fn overlap_shows_past_360_with_a_north_stop() {
        assert_eq!(display_az(400.0, StopAt::North), 400.0);
        assert_eq!(display_az(200.0, StopAt::South), 200.0);
        assert_eq!(display_az(-90.0, StopAt::South), 270.0);
        assert!(geom::in_overlap(400.0, StopAt::North));
    }

    #[test]
    fn cells_carry_the_decimal_point() {
        assert_eq!(
            cells("360.0"),
            vec![('3', false), ('6', false), ('0', true), ('0', false)]
        );
        assert_eq!(cells("--.-").len(), 3);
        assert_eq!(segments('8'), 0x7F);
        assert_eq!(segments(' '), 0);
    }
}

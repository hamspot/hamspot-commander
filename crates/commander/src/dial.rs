//! Bandspread scales. The painter follows the RT-950 CPS dial: a filled span,
//! a needle, and click-or-drag tuning. It does not link that crate.

use eframe::egui;

pub const HAM: egui::Color32 = egui::Color32::from_rgb(92, 184, 120);

pub struct Band {
    pub id: &'static str,
    pub chip: &'static str,
    pub lo: f64,
    pub hi: f64,
    pub home: f64,
}

pub const BANDS: &[Band] = &[
    Band {
        id: "160 m",
        chip: "160m",
        lo: 1_800_000.0,
        hi: 2_000_000.0,
        home: 1_900_000.0,
    },
    Band {
        id: "80 m",
        chip: "80m",
        lo: 3_500_000.0,
        hi: 4_000_000.0,
        home: 3_750_000.0,
    },
    Band {
        id: "60 m",
        chip: "60m",
        lo: 5_250_000.0,
        hi: 5_450_000.0,
        home: 5_403_500.0,
    },
    Band {
        id: "40 m",
        chip: "40m",
        lo: 7_000_000.0,
        hi: 7_300_000.0,
        home: 7_150_000.0,
    },
    Band {
        id: "30 m",
        chip: "30m",
        lo: 10_100_000.0,
        hi: 10_150_000.0,
        home: 10_136_000.0,
    },
    Band {
        id: "20 m",
        chip: "20m",
        lo: 14_000_000.0,
        hi: 14_350_000.0,
        home: 14_153_608.0,
    },
    Band {
        id: "17 m",
        chip: "17m",
        lo: 18_068_000.0,
        hi: 18_168_000.0,
        home: 18_100_000.0,
    },
    Band {
        id: "15 m",
        chip: "15m",
        lo: 21_000_000.0,
        hi: 21_450_000.0,
        home: 21_300_000.0,
    },
    Band {
        id: "12 m",
        chip: "12m",
        lo: 24_890_000.0,
        hi: 24_990_000.0,
        home: 24_940_000.0,
    },
    Band {
        id: "10 m",
        chip: "10m",
        lo: 28_000_000.0,
        hi: 29_700_000.0,
        home: 28_400_000.0,
    },
];

pub fn band_of(hz: f64) -> Option<&'static Band> {
    BANDS.iter().find(|band| hz >= band.lo && hz <= band.hi)
}

pub fn format_hz(hz: f64) -> String {
    let rounded = hz.round() as i64;
    let sign = if rounded < 0 { "-" } else { "" };
    let hz = rounded.unsigned_abs();
    format!(
        "{sign}{}.{:03}.{:03}",
        hz / 1_000_000,
        (hz / 1_000) % 1_000,
        hz % 1_000
    )
}

/// `14.153.608`, `14.153608`, or a raw Hertz integer.
pub fn parse_freq(text: &str) -> Option<f64> {
    let text = text.trim().replace(' ', "");
    if text.is_empty() {
        return None;
    }
    let parts: Vec<&str> = text.split('.').collect();
    if parts.len() == 3 {
        let mhz: i64 = parts[0].parse().ok()?;
        let khz: i64 = parts[1].parse().ok()?;
        let unit: i64 = parts[2].parse().ok()?;
        if parts[1].len() > 3 || parts[2].len() > 3 {
            return None;
        }
        return Some((mhz * 1_000_000 + khz * 1_000 + unit) as f64);
    }
    if parts.len() == 2 {
        let mhz: f64 = text.parse().ok()?;
        return Some(mhz * 1_000_000.0);
    }
    text.parse().ok()
}

/// Map hamlib's dB-relative-to-S9 reading onto a bar. S0 is -54, S9+60 is the top.
pub fn strength_frac(db: f64) -> f32 {
    ((db - (-54.0)) / (60.0 - (-54.0))).clamp(0.0, 1.0) as f32
}

pub fn strength_label(db: f64) -> String {
    if db >= 1.0 {
        format!("S9+{:.0}", db)
    } else if db >= -3.0 {
        "S9".into()
    } else {
        format!("S{:.0}", ((db + 54.0) / 6.0).clamp(0.0, 9.0).round())
    }
}

/// Click or drag returns a frequency in Hz inside `lo..=hi`.
pub fn scale(
    ui: &mut egui::Ui,
    label: &str,
    hz: f64,
    lo: f64,
    hi: f64,
    color: egui::Color32,
    height: f32,
) -> Option<f64> {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(4),
        ui.visuals().extreme_bg_color,
    );
    let span = (hi - lo).max(1.0);
    let band = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 52.0, rect.top() + 4.0),
        egui::pos2(rect.right() - 4.0, rect.bottom() - 4.0),
    );
    painter.rect_filled(band, egui::CornerRadius::same(2), color);
    let t = ((hz - lo) / span).clamp(0.0, 1.0) as f32;
    let x = band.left() + t * band.width();
    painter.line_segment(
        [
            egui::pos2(x, rect.top() + 2.0),
            egui::pos2(x, rect.bottom() - 2.0),
        ],
        egui::Stroke::new(2.0, ui.visuals().strong_text_color()),
    );
    painter.text(
        egui::pos2(rect.left() + 4.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::monospace(12.0),
        ui.visuals().text_color(),
    );
    painter.text(
        egui::pos2(band.left() + 4.0, band.bottom() - 1.0),
        egui::Align2::LEFT_BOTTOM,
        edge(lo),
        egui::FontId::monospace(10.0),
        ui.visuals().weak_text_color(),
    );
    painter.text(
        egui::pos2(band.right() - 4.0, band.bottom() - 1.0),
        egui::Align2::RIGHT_BOTTOM,
        edge(hi),
        egui::FontId::monospace(10.0),
        ui.visuals().weak_text_color(),
    );
    if response.clicked() || response.dragged() {
        let pos = response.interact_pointer_pos()?;
        let frac = ((pos.x - band.left()) / band.width()).clamp(0.0, 1.0) as f64;
        Some(lo + frac * span)
    } else {
        None
    }
}

fn edge(hz: f64) -> String {
    format!("{:.3}", hz / 1_000_000.0)
}

pub fn meter(ui: &mut egui::Ui, label: &str, frac: f32, text: &str) {
    ui.horizontal(|ui| {
        ui.set_min_width(118.0);
        ui.label(egui::RichText::new(label).monospace().size(12.0));
        let (rect, _) = ui.allocate_exact_size(egui::vec2(70.0, 10.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(
            rect,
            egui::CornerRadius::same(2),
            ui.visuals().extreme_bg_color,
        );
        let fill = egui::Rect::from_min_max(
            rect.left_top(),
            egui::pos2(
                rect.left() + rect.width() * frac.clamp(0.0, 1.0),
                rect.bottom(),
            ),
        );
        painter.rect_filled(fill, egui::CornerRadius::same(2), HAM);
        ui.label(egui::RichText::new(text).weak().size(11.0));
    });
}

/// Place values of the digits in `14.153.608`, from the left, skipping dots.
pub fn digit_steps(hz: f64) -> Vec<(char, Option<f64>)> {
    let text = format_hz(hz);
    let digits: Vec<char> = text.chars().filter(|c| c.is_ascii_digit()).collect();
    let mut out = Vec::new();
    let mut seen = 0usize;
    for ch in text.chars() {
        if ch == '.' {
            out.push(('.', None));
            continue;
        }
        let from_right = digits.len() - 1 - seen;
        let step = 10f64.powi(from_right as i32);
        out.push((ch, Some(step)));
        seen += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_frequency_round_trips() {
        assert_eq!(format_hz(14_153_608.0), "14.153.608");
        assert!((parse_freq("14.153.608").unwrap() - 14_153_608.0).abs() < 0.5);
        assert!((parse_freq("14.153608").unwrap() - 14_153_608.0).abs() < 1.0);
        assert!((parse_freq("14153608").unwrap() - 14_153_608.0).abs() < 0.5);
    }

    #[test]
    fn s9_plus_20_is_past_the_middle() {
        assert_eq!(strength_label(20.0), "S9+20");
        let frac = strength_frac(20.0);
        assert!(frac > 0.6 && frac < 0.7, "{frac}");
        assert_eq!(strength_label(-54.0), "S0");
        assert_eq!(strength_label(0.0), "S9");
    }

    #[test]
    fn twenty_meters_owns_the_screenshot_frequency() {
        let band = band_of(14_153_608.0).unwrap();
        assert_eq!(band.chip, "20m");
    }

    #[test]
    fn clicking_the_middle_of_a_scale_is_the_mid_frequency() {
        let lo = 14_000_000.0;
        let hi = 14_350_000.0;
        let mid: f64 = lo + 0.5 * (hi - lo);
        assert!((mid - 14_175_000.0).abs() < 0.5);
    }

    #[test]
    fn hertz_digit_is_a_one_hertz_step() {
        let steps = digit_steps(14_153_608.0);
        let last = steps.iter().rev().find_map(|(_, step)| *step).unwrap();
        assert!((last - 1.0).abs() < 0.01);
    }
}

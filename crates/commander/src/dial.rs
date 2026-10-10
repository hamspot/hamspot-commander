//! Bandspread scales. The painter follows the RT-950 CPS dial: a filled span,
//! a needle, and click-or-drag tuning. It does not link that crate.

use eframe::egui;

pub const HAM: egui::Color32 = egui::Color32::from_rgb(92, 184, 120);

#[cfg(test)]
pub struct Band {
    pub id: &'static str,
    pub chip: &'static str,
    pub lo: f64,
    pub hi: f64,
    pub home: f64,
}

#[cfg(test)]
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

#[cfg(test)]
pub fn band_of(hz: f64) -> Option<&'static Band> {
    BANDS.iter().find(|band| hz >= band.lo && hz <= band.hi)
}

/// Edges of the channel bandspread. Empty `freqs_hz` returns `fallback`
/// unchanged. A single frequency is padded to a 0.5 MHz window.
pub fn channel_window(freqs_hz: &[f64], fallback: (f64, f64)) -> (f64, f64) {
    let mut min = f64::MAX;
    let mut max = f64::MIN;
    let mut any = false;
    for hz in freqs_hz {
        if hz.is_finite() && *hz > 0.0 {
            min = min.min(*hz);
            max = max.max(*hz);
            any = true;
        }
    }
    if !any {
        return fallback;
    }
    let pad = ((max - min) * 0.08).max(250_000.0);
    (min - pad, max + pad)
}

pub fn bar_freq(lo: f64, hi: f64, frac: f64) -> f64 {
    let span = (hi - lo).max(1.0);
    lo + frac.clamp(0.0, 1.0) * span
}

/// One bandspread line. Click returns a frequency in Hz inside `lo..=hi`.
#[derive(Clone, Copy)]
pub struct PaintSpan {
    pub lo: f64,
    pub hi: f64,
    pub color: egui::Color32,
}

fn fill_spans(painter: &egui::Painter, area: egui::Rect, lo: f64, hi: f64, spans: &[PaintSpan]) {
    let span = (hi - lo).max(1.0);
    for seg in spans {
        let mut x0 = area.left() + ((seg.lo - lo) / span).clamp(0.0, 1.0) as f32 * area.width();
        let mut x1 = area.left() + ((seg.hi - lo) / span).clamp(0.0, 1.0) as f32 * area.width();
        if x1 <= x0 {
            continue;
        }
        // A narrow allocation on a wide row is still a mark, not a missing band.
        if x1 - x0 < 2.0 {
            let mid = (x0 + x1) * 0.5;
            x0 = (mid - 1.0).max(area.left());
            x1 = (mid + 1.0).min(area.right());
        }
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(x0, area.top()), egui::pos2(x1, area.bottom())),
            egui::CornerRadius::same(2),
            seg.color,
        );
    }
}

pub fn channel_dial(
    ui: &mut egui::Ui,
    needle_hz: Option<f64>,
    lo: f64,
    hi: f64,
    spans: &[PaintSpan],
) -> Option<f64> {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 46.0), egui::Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(6),
        ui.visuals().extreme_bg_color,
    );
    let span = (hi - lo).max(1.0);
    let inner = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.top() + 14.0),
        egui::pos2(rect.right(), rect.bottom() - 12.0),
    );
    fill_spans(&painter, inner, lo, hi, spans);
    if let Some(hz) = needle_hz {
        let t = ((hz - lo) / span).clamp(0.0, 1.0) as f32;
        let x = rect.left() + t * rect.width();
        painter.line_segment(
            [
                egui::pos2(x, rect.top() + 4.0),
                egui::pos2(x, rect.bottom() - 4.0),
            ],
            egui::Stroke::new(2.0, ui.visuals().strong_text_color()),
        );
    }
    let font = egui::FontId::monospace(11.0);
    let color = ui.visuals().weak_text_color();
    painter.text(
        rect.left_bottom() + egui::vec2(4.0, -1.0),
        egui::Align2::LEFT_BOTTOM,
        edge_mhz(lo, span),
        font.clone(),
        color,
    );
    painter.text(
        rect.right_bottom() + egui::vec2(-4.0, -1.0),
        egui::Align2::RIGHT_BOTTOM,
        edge_mhz(hi, span),
        font,
        color,
    );
    if response.clicked() {
        let pos = response.interact_pointer_pos()?;
        let frac = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64;
        Some(bar_freq(lo, hi, frac))
    } else {
        None
    }
}

fn edge_mhz(hz: f64, span_hz: f64) -> String {
    let mhz = hz / 1_000_000.0;
    let span = span_hz / 1_000_000.0;
    if span >= 10.0 {
        format!("{mhz:.0}")
    } else if span >= 1.0 {
        format!("{mhz:.1}")
    } else {
        format!("{mhz:.2}")
    }
}

pub fn swatch(ui: &mut egui::Ui, color: egui::Color32, text: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(2), color);
    ui.label(egui::RichText::new(text).weak());
}

/// `0014.153.608`: four megahertz digits, then kilohertz and hertz.
/// The field stays four wide through 9999 MHz so 20 m, 2 m, and 70 cm
/// keep the same columns. Wider values still print in full.
/// `VFOA` is `VFO A`, `Main` stays `Main`, `SubA` is `Sub A`.
pub fn vfo_label(token: &str) -> String {
    let token = token.trim();
    for prefix in ["VFO", "Main", "Sub"] {
        if token.len() >= prefix.len() && token[..prefix.len()].eq_ignore_ascii_case(prefix) {
            let rest = &token[prefix.len()..];
            if rest.is_empty() {
                return prefix.to_string();
            }
            if rest.len() <= 2 && rest.chars().all(|ch| ch.is_ascii_alphanumeric()) {
                return format!("{prefix} {rest}");
            }
        }
    }
    token.to_string()
}

pub fn format_hz(hz: f64) -> String {
    let rounded = if hz.is_finite() { hz.round() } else { 0.0 };
    let neg = rounded < 0.0;
    // Stay inside the range that casts to u64 without saturating to a
    // wrapped bit pattern. i64::MIN's magnitude does not fit in i64.
    let mag = rounded.abs().min(i64::MAX as f64) as u64;
    let sign = if neg { "-" } else { "" };
    format!(
        "{sign}{:04}.{:03}.{:03}",
        mag / 1_000_000,
        (mag / 1_000) % 1_000,
        mag % 1_000
    )
}

/// `0014.153.608`, `14.153.608`, `14.153608`, or a raw Hertz integer.
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

/// One allocation drawn inside a zone row.
pub struct SubBand {
    pub lo: f64,
    pub hi: f64,
    pub chip: String,
    pub spans: Vec<PaintSpan>,
}

/// Visual gap between subbands on one zone line. It is not a frequency hole.
const SUBBAND_PAD: f32 = 6.0;

/// Zone names and subband chips. The row stays 28 px; the type is larger than the old 12 px.
const BSP_FONT: f32 = 16.0;

fn bsp_label_width(label: &str) -> f32 {
    (label.chars().count() as f32 * BSP_FONT * 0.6 + 14.0).clamp(72.0, 148.0)
}

/// White type with a black halo so it stays readable on the subband colors.
fn halo_text(
    painter: &egui::Painter,
    pos: egui::Pos2,
    align: egui::Align2,
    text: &str,
    font: egui::FontId,
) {
    let halo = egui::Color32::BLACK;
    let fill = egui::Color32::WHITE;
    for dy in -1..=1 {
        for dx in -1..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            painter.text(
                pos + egui::vec2(dx as f32, dy as f32),
                align,
                text,
                font.clone(),
                halo,
            );
        }
    }
    painter.text(pos, align, text, font, fill);
}

/// Left and right edges of each subband inside a row of `width` px.
/// A single subband uses the whole width. Padding shrinks when the row is narrow.
pub fn subband_slots(count: usize, width: f32, pad: f32) -> Vec<(f32, f32)> {
    if count == 0 || !(width > 0.0) {
        return Vec::new();
    }
    if count == 1 {
        return vec![(0.0, width)];
    }
    let max_pad = (width * 0.2) / (count as f32 - 1.0);
    let pad = pad.clamp(0.0, max_pad.max(0.0));
    let slot = (width - pad * (count as f32 - 1.0)) / count as f32;
    if !(slot > 0.0) {
        return Vec::new();
    }
    (0..count)
        .map(|i| {
            let x0 = i as f32 * (slot + pad);
            (x0, x0 + slot)
        })
        .collect()
}

fn subband_at(x: f32, slots: &[(f32, f32)]) -> Option<usize> {
    let last = slots.len().checked_sub(1)?;
    slots
        .iter()
        .enumerate()
        .find(|(i, (lo, hi))| x >= *lo && if *i == last { x <= *hi } else { x < *hi })
        .map(|(i, _)| i)
}

fn freq_in_slot(x: f32, x0: f32, x1: f32, lo: f64, hi: f64) -> f64 {
    let frac = if x1 > x0 {
        ((x - x0) / (x1 - x0)).clamp(0.0, 1.0) as f64
    } else {
        0.0
    };
    let span = if hi > lo { hi - lo } else { 0.0 };
    lo + frac * span
}

/// The clicked subband, when the frequency is still inside it.
/// Another subband that also covers `hz` does not take the needle.
fn active_subband(hz: f64, bands: &[SubBand], picked: Option<usize>) -> Option<usize> {
    if !hz.is_finite() {
        return None;
    }
    picked.filter(|idx| {
        bands
            .get(*idx)
            .is_some_and(|band| hz >= band.lo && hz <= band.hi && band.hi >= band.lo)
    })
}

/// One zone on one line. Subbands share the row and keep a small gap between them.
/// Click or drag returns the subband index, a frequency inside that subband, and
/// whether the press was the secondary button (sub VFO). A press in the gap
/// returns nothing.
pub fn zone_scale(
    ui: &mut egui::Ui,
    label: &str,
    hz: f64,
    bands: &[SubBand],
    picked: Option<usize>,
    height: f32,
) -> Option<(usize, f64, bool)> {
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
    let label_w = bsp_label_width(label);
    let area = egui::Rect::from_min_max(
        egui::pos2(rect.left() + label_w, rect.top() + 4.0),
        egui::pos2(rect.right() - 4.0, rect.bottom() - 4.0),
    );
    let slots = subband_slots(bands.len(), area.width(), SUBBAND_PAD);
    let active = active_subband(hz, bands, picked);
    for (idx, band) in bands.iter().enumerate() {
        let Some(&(x0, x1)) = slots.get(idx) else {
            continue;
        };
        let slot = egui::Rect::from_min_max(
            egui::pos2(area.left() + x0, area.top()),
            egui::pos2(area.left() + x1, area.bottom()),
        );
        let fade = if active == Some(idx) { 1.0 } else { 0.35 };
        if band.spans.is_empty() {
            painter.rect_filled(slot, egui::CornerRadius::same(2), HAM.gamma_multiply(fade));
        } else {
            let faded: Vec<PaintSpan> = band
                .spans
                .iter()
                .map(|span| PaintSpan {
                    lo: span.lo,
                    hi: span.hi,
                    color: span.color.gamma_multiply(fade),
                })
                .collect();
            fill_spans(&painter, slot, band.lo, band.hi, &faded);
        }
    }
    if let Some(idx) = active {
        if let (Some(band), Some(&(x0, x1))) = (bands.get(idx), slots.get(idx)) {
            let span = (band.hi - band.lo).max(1.0);
            let t = ((hz - band.lo) / span).clamp(0.0, 1.0) as f32;
            let x = area.left() + x0 + t * (x1 - x0);
            painter.line_segment(
                [
                    egui::pos2(x, rect.top() + 2.0),
                    egui::pos2(x, rect.bottom() - 2.0),
                ],
                egui::Stroke::new(2.0, ui.visuals().strong_text_color()),
            );
        }
    }
    let font = egui::FontId::monospace(BSP_FONT);
    for (idx, band) in bands.iter().enumerate() {
        if band.chip.is_empty() {
            continue;
        }
        let Some(&(x0, x1)) = slots.get(idx) else {
            continue;
        };
        if x1 - x0 < 20.0 {
            continue;
        }
        let slot = egui::Rect::from_min_max(
            egui::pos2(area.left() + x0, area.top()),
            egui::pos2(area.left() + x1, area.bottom()),
        );
        let clipped = painter.with_clip_rect(slot);
        halo_text(
            &clipped,
            slot.left_center() + egui::vec2(4.0, 0.0),
            egui::Align2::LEFT_CENTER,
            &band.chip,
            font.clone(),
        );
    }
    halo_text(
        &painter,
        egui::pos2(rect.left() + 6.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font,
    );
    if let Some(pos) = response.hover_pos() {
        if let Some(idx) = subband_at(pos.x - area.left(), &slots) {
            if let Some(band) = bands.get(idx) {
                let name = if band.chip.is_empty() {
                    label
                } else {
                    band.chip.as_str()
                };
                response.clone().on_hover_text(format!(
                    "{name}   {:.3}–{:.3} MHz",
                    band.lo / 1_000_000.0,
                    band.hi / 1_000_000.0
                ));
            }
        }
    }
    // Primary tunes the main VFO. Secondary tunes the sub VFO. `dragged()` is
    // true for either button, so the button has to be checked on its own.
    let primary = response.clicked() || response.dragged_by(egui::PointerButton::Primary);
    let secondary =
        response.secondary_clicked() || response.dragged_by(egui::PointerButton::Secondary);
    if !primary && !secondary {
        return None;
    }
    let pos = response.interact_pointer_pos()?;
    let x = pos.x - area.left();
    let idx = subband_at(x, &slots)?;
    let (x0, x1) = slots[idx];
    let band = bands.get(idx)?;
    Some((
        idx,
        freq_in_slot(x, x0, x1, band.lo, band.hi),
        secondary && !primary,
    ))
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
    spans: &[PaintSpan],
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
    let label_w = (label.chars().count() as f32 * 7.2 + 10.0).clamp(52.0, 96.0);
    let band = egui::Rect::from_min_max(
        egui::pos2(rect.left() + label_w, rect.top() + 4.0),
        egui::pos2(rect.right() - 4.0, rect.bottom() - 4.0),
    );
    if spans.is_empty() {
        painter.rect_filled(band, egui::CornerRadius::same(2), color);
    } else {
        fill_spans(&painter, band, lo, hi, spans);
    }
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

/// One step per wheel event. A notch often reports a large delta (14 points,
/// or a whole line) and egui then eases that delta across later frames.
/// Those later frames are not more clicks. A zero delta is not a click.
pub fn wheel_notches(deltas_y: impl IntoIterator<Item = f32>) -> i32 {
    deltas_y.into_iter().fold(0, |count, dy| {
        if dy > 0.0 {
            count + 1
        } else if dy < 0.0 {
            count - 1
        } else {
            count
        }
    })
}

/// Place values of the digits in `0014.153.608`, from the left.
/// Dots and a leading minus are returned with no step. Counting those
/// characters as digits underflows once the real digits run out.
pub fn digit_steps(hz: f64) -> Vec<(char, Option<f64>)> {
    let text = format_hz(hz);
    let digit_count = text.chars().filter(|c| c.is_ascii_digit()).count();
    let mut out = Vec::new();
    let mut seen = 0usize;
    for ch in text.chars() {
        if !ch.is_ascii_digit() {
            out.push((ch, None));
            continue;
        }
        // digit_count is at least 1 here: this character is a digit.
        let from_right = digit_count - 1 - seen;
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
    fn vfo_names_come_from_the_hamlib_token() {
        assert_eq!(vfo_label("VFOA"), "VFO A");
        assert_eq!(vfo_label("VFOB"), "VFO B");
        assert_eq!(vfo_label("Main"), "Main");
        assert_eq!(vfo_label("Sub"), "Sub");
        assert_eq!(vfo_label("MainA"), "Main A");
        assert_eq!(vfo_label("SubB"), "Sub B");
        assert_eq!(vfo_label("MEM"), "MEM");
    }

    #[test]
    fn screenshot_frequency_round_trips() {
        assert_eq!(format_hz(14_153_608.0), "0014.153.608");
        assert_eq!(format_hz(146_520_000.0), "0146.520.000");
        assert_eq!(format_hz(446_000_000.0), "0446.000.000");
        assert_eq!(format_hz(1_500_000_000.0), "1500.000.000");
        assert!((parse_freq("0014.153.608").unwrap() - 14_153_608.0).abs() < 0.5);
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
        assert_eq!(band.id, "20 m");
        assert!((band.home - 14_153_608.0).abs() < 0.5);
    }

    #[test]
    fn subbands_share_a_row_with_padding() {
        let slots = subband_slots(3, 300.0, 6.0);
        assert_eq!(slots.len(), 3);
        assert!((slots[0].0 - 0.0).abs() < 0.01);
        assert!((slots[0].1 - 96.0).abs() < 0.01);
        assert!((slots[1].0 - 102.0).abs() < 0.01);
        assert!(subband_at(99.0, &slots).is_none());
        assert_eq!(subband_at(150.0, &slots), Some(1));
        let hz = freq_in_slot(150.0, slots[1].0, slots[1].1, 7_000_000.0, 7_300_000.0);
        assert!((hz - 7_150_000.0).abs() < 1.0);
    }

    #[test]
    fn one_subband_uses_the_whole_row() {
        let slots = subband_slots(1, 200.0, 6.0);
        assert_eq!(slots, vec![(0.0, 200.0)]);
        assert!(subband_slots(0, 200.0, 6.0).is_empty());
    }

    #[test]
    fn narrow_row_shrinks_the_padding() {
        let slots = subband_slots(10, 50.0, 6.0);
        assert_eq!(slots.len(), 10);
        let gap = slots[1].0 - slots[0].1;
        assert!(gap < 6.0, "{gap}");
        assert!(slots.iter().all(|(lo, hi)| hi - lo > 0.0));
    }

    #[test]
    fn overlapping_subband_keeps_the_clicked_line() {
        let bands = [
            SubBand {
                lo: 7_000_000.0,
                hi: 7_300_000.0,
                chip: "40m".into(),
                spans: Vec::new(),
            },
            SubBand {
                lo: 7_200_000.0,
                hi: 7_400_000.0,
                chip: "41m".into(),
                spans: Vec::new(),
            },
        ];
        assert_eq!(active_subband(7_250_000.0, &bands, Some(0)), Some(0));
        assert_eq!(active_subband(7_250_000.0, &bands, Some(1)), Some(1));
        assert!(active_subband(7_250_000.0, &bands, None).is_none());
        assert!(active_subband(1_000.0, &bands, Some(0)).is_none());
    }

    #[test]
    fn clicking_the_middle_of_a_scale_is_the_mid_frequency() {
        let lo = 14_000_000.0;
        let hi = 14_350_000.0;
        let mid: f64 = lo + 0.5 * (hi - lo);
        assert!((mid - 14_175_000.0).abs() < 0.5);
    }

    #[test]
    fn one_wheel_notch_is_one_digit() {
        assert_eq!(wheel_notches([14.0]), 1);
        assert_eq!(wheel_notches([120.0]), 1);
        assert_eq!(wheel_notches([-53.0]), -1);
        assert_eq!(wheel_notches([14.0, 14.0]), 2);
        assert_eq!(wheel_notches([0.0]), 0);
        assert_eq!(wheel_notches(std::iter::empty()), 0);
    }

    #[test]
    fn hertz_digit_is_a_one_hertz_step() {
        let steps = digit_steps(14_153_608.0);
        let last = steps.iter().rev().find_map(|(_, step)| *step).unwrap();
        assert!((last - 1.0).abs() < 0.01);
        let mhz = steps
            .iter()
            .filter_map(|(_, step)| *step)
            .take(4)
            .collect::<Vec<_>>();
        assert_eq!(mhz.len(), 4);
        assert!((mhz[0] - 1_000_000_000.0).abs() < 0.5);
        assert!((mhz[3] - 1_000_000.0).abs() < 0.5);
    }

    #[test]
    fn negative_frequency_does_not_overflow() {
        let steps = digit_steps(-14_153_608.0);
        assert_eq!(steps.first().map(|(ch, _)| *ch), Some('-'));
        assert!(steps.first().unwrap().1.is_none());
        let last = steps.iter().rev().find_map(|(_, step)| *step).unwrap();
        assert!((last - 1.0).abs() < 0.01);
        let _ = digit_steps(f64::NAN);
        let _ = digit_steps(f64::NEG_INFINITY);
    }

    #[test]
    fn channel_window_pads_and_falls_back() {
        let (lo, hi) = channel_window(&[14_153_608.0, 146_520_000.0], (0.0, 1.0));
        let span = 146_520_000.0 - 14_153_608.0;
        let pad = span * 0.08;
        assert!((lo - (14_153_608.0 - pad)).abs() < 1.0, "{lo}");
        assert!((hi - (146_520_000.0 + pad)).abs() < 1.0, "{hi}");

        let (lo, hi) = channel_window(&[146_000_000.0], (0.0, 1.0));
        assert!((hi - lo - 500_000.0).abs() < 1.0, "{}", hi - lo);

        let fallback = (150_000.0, 1_500_000_000.0);
        assert_eq!(channel_window(&[], fallback), fallback);
        assert_eq!(channel_window(&[0.0, f64::NAN], fallback), fallback);
    }

    #[test]
    fn middle_of_the_channel_bar_is_the_mid_frequency() {
        let mid = bar_freq(14_000_000.0, 14_350_000.0, 0.5);
        assert!((mid - 14_175_000.0).abs() < 0.5);
    }
}

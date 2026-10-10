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

/// Cream card of the movement. Lamp color is a wash on top of this, not a
/// replacement, so a red bulb still reads as a white face.
pub const METER_FACE: egui::Color32 = egui::Color32::from_rgb(255, 243, 214);

/// Degrees of needle travel, left rest to right stop. Zero degrees is up.
const METER_SWEEP_DEG: f32 = 124.0;

/// Bulb tint. Warm is the incandescent lamp. The others shift that lamp
/// slightly; they do not recolor the ticks or the scale name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lamp {
    Warm,
    Red,
    Blue,
    Green,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeterTick {
    pub frac: f32,
    pub major: bool,
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeterScale {
    pub name: String,
    pub ticks: Vec<MeterTick>,
    /// Start of the red arc, as the same 0..1 fraction as the needle.
    /// `None` leaves the whole scale in the face color.
    pub red_from: Option<f32>,
}

/// A fraction that is not finite parks on the left stop. `clamp` would
/// keep a NaN, and a NaN angle draws the needle off the face.
fn unit_frac(frac: f32) -> f32 {
    if frac.is_finite() {
        frac.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

pub fn needle_deg(frac: f32) -> f32 {
    let t = unit_frac(frac);
    -METER_SWEEP_DEG * 0.5 + t * METER_SWEEP_DEG
}

/// Where the needle is, and how fast it is moving, in fraction of the sweep.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Needle {
    pub frac: f32,
    pub vel: f32,
}

impl Needle {
    pub fn at(frac: f32) -> Self {
        Self {
            frac: unit_frac(frac),
            vel: 0.0,
        }
    }
}

/// One step of a damped movement. `dt` is seconds since the last paint.
/// A full-scale change takes a fraction of a second. One frame does not
/// cross the scale, and a stalled frame does not throw the needle.
pub fn step_needle(needle: Needle, target: f32, dt: f32) -> Needle {
    let target = unit_frac(target);
    let dt = if dt.is_finite() {
        dt.clamp(0.0, 0.05)
    } else {
        0.0
    };
    // Slightly under-damped, so the needle floats up to the new reading
    // and settles instead of snapping or ringing.
    const STIFFNESS: f32 = 46.0;
    const DAMPING: f32 = 10.5;
    let mut vel = needle.vel + (STIFFNESS * (target - needle.frac) - DAMPING * needle.vel) * dt;
    let mut frac = needle.frac + vel * dt;
    if !frac.is_finite() || !vel.is_finite() {
        return Needle::at(target);
    }
    if frac < 0.0 {
        frac = 0.0;
        if vel < 0.0 {
            vel = 0.0;
        }
    } else if frac > 1.0 {
        frac = 1.0;
        if vel > 0.0 {
            vel = 0.0;
        }
    }
    Needle { frac, vel }
}

pub fn needle_settled(needle: Needle, target: f32) -> bool {
    (needle.frac - unit_frac(target)).abs() < 0.004 && needle.vel.abs() < 0.02
}

/// Glow rgb. Alpha is applied when the dome is painted.
pub fn lamp_glow(lamp: Lamp) -> egui::Color32 {
    let warm = [255.0_f32, 176.0, 64.0];
    let toward = match lamp {
        Lamp::Warm => warm,
        Lamp::Red => [255.0, 48.0, 36.0],
        Lamp::Blue => [64.0, 132.0, 255.0],
        Lamp::Green => [48.0, 176.0, 96.0],
    };
    // Warm is the bulb. A mode tint moves less than halfway so the face
    // stays an incandescent meter with a colored lamp, not a colored card.
    let mix = if lamp == Lamp::Warm { 0.0 } else { 0.42 };
    let channel = |i: usize| (warm[i] + (toward[i] - warm[i]) * mix).round() as u8;
    egui::Color32::from_rgb(channel(0), channel(1), channel(2))
}

fn meter_tick(frac: f32, major: bool, label: Option<&str>) -> MeterTick {
    MeterTick {
        frac: unit_frac(frac),
        major,
        label: label.map(str::to_string),
    }
}

fn trim_num(value: f32) -> String {
    if (value - value.round()).abs() < 0.05 {
        format!("{}", value.round() as i32)
    } else {
        format!("{value:.1}")
    }
}

/// S0 at the left stop through S9, then +20 and +60, on the same span as
/// [`strength_frac`]. Every S-unit is a tick. Labels are the marks that fit
/// on the arc: 0, 3, 6, 9, +20, +60. The name on the face is `S`.
pub fn s_scale() -> MeterScale {
    let mut ticks = Vec::new();
    for unit in 0..=9 {
        let db = -54.0 + f64::from(unit) * 6.0;
        let label = matches!(unit, 0 | 3 | 6 | 9).then(|| unit.to_string());
        ticks.push(meter_tick(strength_frac(db), true, label.as_deref()));
    }
    ticks.push(meter_tick(strength_frac(20.0), true, Some("+20")));
    ticks.push(meter_tick(strength_frac(60.0), true, Some("+60")));
    MeterScale {
        name: "S".into(),
        ticks,
        red_from: Some(strength_frac(20.0)),
    }
}

/// 1.0 at rest through 5.0 at the stop. Red begins at 3.0, half of that span.
pub fn swr_scale() -> MeterScale {
    let at = |value: f32| (value - 1.0) / 4.0;
    let mut ticks = vec![
        meter_tick(at(1.0), true, Some("1")),
        meter_tick(at(1.5), true, Some("1.5")),
        meter_tick(at(2.0), true, Some("2")),
        meter_tick(at(3.0), true, Some("3")),
        meter_tick(at(5.0), true, Some("5")),
    ];
    for value in [1.25, 1.75, 2.5, 3.5, 4.0, 4.5] {
        ticks.push(meter_tick(at(value), false, None));
    }
    MeterScale {
        name: "SWR".into(),
        ticks,
        red_from: Some(at(3.0)),
    }
}

fn percent_scale(name: &str) -> MeterScale {
    MeterScale {
        name: name.into(),
        ticks: vec![
            meter_tick(0.0, true, Some("0")),
            meter_tick(0.25, false, None),
            meter_tick(0.5, true, Some("50")),
            meter_tick(0.75, false, None),
            meter_tick(1.0, true, Some("100")),
        ],
        red_from: None,
    }
}

pub fn alc_scale() -> MeterScale {
    percent_scale("ALC")
}

pub fn pwr_scale() -> MeterScale {
    percent_scale("PWR")
}

fn ranged_scale(name: &str, full: f32) -> MeterScale {
    // A non-positive full scale has no tick that can be labeled. The needle
    // fraction is still the caller's, clamped onto the arc.
    if !full.is_finite() || full <= 0.0 {
        return MeterScale {
            name: name.into(),
            ticks: Vec::new(),
            red_from: None,
        };
    }
    MeterScale {
        name: name.into(),
        ticks: vec![
            meter_tick(0.0, true, Some("0")),
            meter_tick(0.25, false, None),
            meter_tick(0.5, true, Some(&trim_num(full * 0.5))),
            meter_tick(0.75, false, None),
            meter_tick(1.0, true, Some(&trim_num(full))),
        ],
        red_from: None,
    }
}

pub fn volts_scale(full: f32) -> MeterScale {
    ranged_scale("V", full)
}

pub fn milliamps_scale(full: f32) -> MeterScale {
    ranged_scale("mA", full)
}

fn arc_point(pivot: egui::Pos2, radius: f32, deg: f32) -> egui::Pos2 {
    let rad = deg.to_radians();
    egui::pos2(pivot.x + rad.sin() * radius, pivot.y - rad.cos() * radius)
}

fn arc_points(pivot: egui::Pos2, radius: f32, from_deg: f32, to_deg: f32) -> Vec<egui::Pos2> {
    let steps = 18;
    (0..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32;
            arc_point(pivot, radius, from_deg + (to_deg - from_deg) * t)
        })
        .collect()
}

pub const METER_W: f32 = 196.0;
pub const METER_H: f32 = 132.0;
pub const METER_GAP: f32 = 8.0;

/// Hamlib read-only meters, from `RIG_LEVEL_READONLY_LIST` in `rig.h`.
/// Strength leads so a new grid matches the old Meters page, then the
/// other readings in header order. `RAWSTR` is last: it is a rig-specific
/// count and is drawn only when the radio gives it a real range.
pub const RADIO_METERS: &[&str] = &[
    "STRENGTH",
    "ALC",
    "SWR",
    "RFPOWER_METER",
    "RFPOWER_METER_WATTS",
    "COMP_METER",
    "VD_METER",
    "ID_METER",
    "TEMP_METER",
    "RAWSTR",
];

pub struct MeterFace {
    pub id: String,
    pub frac: f32,
    pub scale: MeterScale,
    pub lamp: Lamp,
}

/// Faces for the meter levels this radio lists. Settable levels such as `AF`
/// are not faces. `RFPOWER` is used only when the radio has no power meter.
pub fn meter_faces(
    get_levels: &[(&str, f64, f64)],
    values: &[(&str, f64)],
    strength_db: f64,
    ptt: bool,
) -> Vec<MeterFace> {
    let listed = |id: &str| get_levels.iter().any(|(name, _, _)| *name == id);
    let span = |id: &str| {
        get_levels
            .iter()
            .find(|(name, _, _)| *name == id)
            .map(|(_, min, max)| (*min, *max))
    };
    let value = |id: &str| values.iter().find(|(name, _)| *name == id).map(|(_, v)| *v);
    let mut ids: Vec<&str> = RADIO_METERS
        .iter()
        .copied()
        .filter(|id| listed(id))
        .collect();
    if !listed("RFPOWER_METER") && !listed("RFPOWER_METER_WATTS") && listed("RFPOWER") {
        ids.push("RFPOWER");
    }
    ids.into_iter()
        .filter_map(|id| {
            let (min, max) = span(id).unwrap_or((0.0, 0.0));
            face_for(id, value(id), min, max, strength_db, ptt)
        })
        .collect()
}

fn face_for(
    id: &str,
    value: Option<f64>,
    min: f64,
    max: f64,
    strength_db: f64,
    ptt: bool,
) -> Option<MeterFace> {
    let power = matches!(id, "RFPOWER_METER" | "RFPOWER_METER_WATTS" | "RFPOWER");
    let lamp = if power && ptt { Lamp::Red } else { Lamp::Warm };
    let (scale, frac) = match id {
        "STRENGTH" => {
            let db = value.unwrap_or(strength_db);
            (s_scale(), strength_frac(db))
        }
        "SWR" => (swr_scale(), ((value.unwrap_or(1.0) - 1.0) / 4.0) as f32),
        "ALC" => unit_or_range("ALC", value.unwrap_or(0.0), min, max, alc_scale()),
        "RFPOWER_METER" | "RFPOWER" => {
            unit_or_range("PWR", value.unwrap_or(0.0), min, max, pwr_scale())
        }
        "RFPOWER_METER_WATTS" => range_face("W", value, min, max)?,
        "COMP_METER" => range_face("COMP", value, min, max)?,
        "VD_METER" => range_face("VD", value, min, max)?,
        "ID_METER" => range_face("ID", value, min, max)?,
        "TEMP_METER" => range_face("TEMP", value, min, max)?,
        "RAWSTR" => range_face("RAW", value, min, max)?,
        _ => return None,
    };
    Some(MeterFace {
        id: id.to_string(),
        frac,
        scale,
        lamp,
    })
}

/// A 0..1 hamlib meter keeps the percent scale. Any other usable range is
/// labeled with that range. A missing range stays on the percent scale.
fn unit_or_range(
    name: &str,
    value: f64,
    min: f64,
    max: f64,
    percent: MeterScale,
) -> (MeterScale, f32) {
    if (min - 0.0).abs() < 1e-3 && (max - 1.0).abs() < 1e-2 {
        return (percent, value as f32);
    }
    match span_frac(value, min, max) {
        Some(frac) => {
            let mut scale = range_scale(name, min, max);
            if name == "PWR" || name == "ALC" {
                scale.red_from = None;
            }
            (scale, frac)
        }
        None => (percent, value as f32),
    }
}

fn range_face(name: &str, value: Option<f64>, min: f64, max: f64) -> Option<(MeterScale, f32)> {
    let value = value.unwrap_or(min);
    let frac = span_frac(value, min, max)?;
    Some((range_scale(name, min, max), frac))
}

fn span_frac(value: f64, min: f64, max: f64) -> Option<f32> {
    if !value.is_finite() || !min.is_finite() || !max.is_finite() {
        return None;
    }
    let span = max - min;
    if span.abs() < 1e-6 {
        return None;
    }
    Some(((value - min) / span) as f32)
}

fn range_scale(name: &str, min: f64, max: f64) -> MeterScale {
    let at = |frac: f32, label: Option<&str>| meter_tick(frac, label.is_some(), label);
    let mid = (min + max) * 0.5;
    MeterScale {
        name: name.into(),
        ticks: vec![
            at(0.0, Some(&trim_num(min as f32))),
            at(0.25, None),
            at(0.5, Some(&trim_num(mid as f32))),
            at(0.75, None),
            at(1.0, Some(&trim_num(max as f32))),
        ],
        red_from: None,
    }
}

/// Saved ids that this radio still has, then any new meter at the end.
pub fn arrange_meter_ids(saved: &[String], faces: &[MeterFace]) -> Vec<String> {
    let have: Vec<String> = faces.iter().map(|face| face.id.clone()).collect();
    let mut out = Vec::new();
    for id in saved {
        if have.iter().any(|have_id| have_id == id) && !out.iter().any(|kept| kept == id) {
            out.push(id.clone());
        }
    }
    for id in have {
        if !out.iter().any(|kept| kept == &id) {
            out.push(id);
        }
    }
    out
}

pub fn move_meter(order: &[String], from: usize, to: usize) -> Vec<String> {
    if from >= order.len() || to >= order.len() || from == to {
        return order.to_vec();
    }
    let mut next = order.to_vec();
    let id = next.remove(from);
    next.insert(to, id);
    next
}

/// Write `visible` back into `saved`. Meters this radio is not showing keep
/// their places. A name that is not a meter is dropped. A new meter appends.
pub fn merge_meter_order(saved: &[String], visible: &[String]) -> Vec<String> {
    let known = |id: &str| RADIO_METERS.contains(&id) || id == "RFPOWER";
    let mut base: Vec<String> = saved.iter().filter(|id| known(id)).cloned().collect();
    let visible: Vec<String> = visible.iter().filter(|id| known(id)).cloned().collect();
    let showing: std::collections::BTreeSet<&str> = visible.iter().map(String::as_str).collect();
    let slots: Vec<usize> = base
        .iter()
        .enumerate()
        .filter(|(_, id)| showing.contains(id.as_str()))
        .map(|(index, _)| index)
        .collect();
    let mut incoming = visible.into_iter();
    for slot in slots {
        if let Some(id) = incoming.next() {
            base[slot] = id;
        }
    }
    base.extend(incoming);
    base
}

pub fn meter_columns(width: f32) -> usize {
    let pitch = METER_W + METER_GAP;
    if !width.is_finite() || width < METER_W {
        1
    } else {
        ((width + METER_GAP) / pitch).floor().max(1.0) as usize
    }
}

pub fn meter_origin(area_x: f32, area_y: f32, index: usize, cols: usize) -> (f32, f32) {
    let cols = cols.max(1);
    let col = index % cols;
    let row = index / cols;
    (
        area_x + col as f32 * (METER_W + METER_GAP),
        area_y + row as f32 * (METER_H + METER_GAP),
    )
}

/// The meter under a point in the grid, or `None` in the gap or past the last cell.
pub fn meter_hit(rel_x: f32, rel_y: f32, cols: usize, count: usize) -> Option<usize> {
    if rel_x < 0.0 || rel_y < 0.0 || cols == 0 || count == 0 {
        return None;
    }
    let pitch_x = METER_W + METER_GAP;
    let pitch_y = METER_H + METER_GAP;
    let col = (rel_x / pitch_x) as usize;
    let row = (rel_y / pitch_y) as usize;
    if col >= cols {
        return None;
    }
    let x_in = rel_x - col as f32 * pitch_x;
    let y_in = rel_y - row as f32 * pitch_y;
    if x_in > METER_W || y_in > METER_H {
        return None;
    }
    let index = row * cols + col;
    if index < count {
        Some(index)
    } else {
        None
    }
}

/// Tiny lock. Closed when `locked` is true. Returns the click response.
pub fn lock_toggle(ui: &mut egui::Ui, locked: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(3),
            ui.visuals().widgets.hovered.bg_fill,
        );
    }
    let color = if locked {
        egui::Color32::from_rgb(226, 198, 130)
    } else {
        ui.visuals().weak_text_color()
    };
    paint_lock(ui.painter(), rect.shrink(3.0), locked, color);
    response.on_hover_text(if locked {
        "Unlock meters"
    } else {
        "Lock meters"
    })
}

fn paint_lock(painter: &egui::Painter, rect: egui::Rect, locked: bool, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.3, color);
    let body = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 1.0, rect.center().y - 1.0),
        egui::pos2(rect.right() - 1.0, rect.bottom()),
    );
    painter.rect_stroke(
        body,
        egui::CornerRadius::same(1),
        stroke,
        egui::StrokeKind::Inside,
    );
    let radius = body.width() * 0.34;
    let center_y = body.top() - radius + if locked { 1.0 } else { -1.5 };
    let center = egui::pos2(body.center().x + if locked { 0.0 } else { -1.0 }, center_y);
    let mut points = Vec::new();
    let sweep = if locked { 180.0 } else { 140.0 };
    let steps = 8;
    for step in 0..=steps {
        let deg = 180.0 + sweep * (step as f32 / steps as f32);
        let rad = deg.to_radians();
        points.push(egui::pos2(
            center.x + rad.cos() * radius,
            center.y + rad.sin() * radius,
        ));
    }
    for pair in points.windows(2) {
        painter.line_segment([pair[0], pair[1]], stroke);
    }
    painter.line_segment([points[0], egui::pos2(points[0].x, body.top())], stroke);
    if locked {
        let end = *points.last().unwrap_or(&points[0]);
        painter.line_segment([end, egui::pos2(end.x, body.top())], stroke);
    }
}

pub fn paint_analog_meter(
    painter: egui::Painter,
    rect: egui::Rect,
    frac: f32,
    scale: &MeterScale,
    lamp: Lamp,
) {
    let bezel = rect.shrink(1.0);
    painter.rect_filled(
        bezel,
        egui::CornerRadius::same(10),
        egui::Color32::from_rgb(36, 22, 12),
    );
    let face = bezel.shrink(5.0);
    painter.rect_filled(face, egui::CornerRadius::same(7), METER_FACE);

    // The bulb sits inside the glass. A circle larger than the dome stops
    // at the glass edge instead of washing the bezel.
    let glow = lamp_glow(lamp);
    let bulb = egui::pos2(face.center().x, face.top() + face.height() * 0.22);
    let glass = painter.with_clip_rect(face);
    for (radius, alpha) in [(58.0, 26), (36.0, 40), (18.0, 54)] {
        glass.circle_filled(
            bulb,
            radius,
            egui::Color32::from_rgba_unmultiplied(glow.r(), glow.g(), glow.b(), alpha),
        );
    }
    // Glass: a pale lip inside the bezel and a short highlight under the bulb.
    painter.rect_stroke(
        face,
        egui::CornerRadius::same(7),
        egui::Stroke::new(
            1.0,
            egui::Color32::from_rgba_unmultiplied(255, 252, 240, 170),
        ),
        egui::StrokeKind::Inside,
    );
    painter.line_segment(
        [
            egui::pos2(face.left() + 22.0, face.top() + 14.0),
            egui::pos2(face.right() - 36.0, face.top() + 10.0),
        ],
        egui::Stroke::new(
            1.5,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 90),
        ),
    );

    let pivot = egui::pos2(face.center().x, face.bottom() - 8.0);
    let radius = (face.width() * 0.46).min(face.height() * 0.92);
    let right = needle_deg(1.0);

    if let Some(red_from) = scale.red_from {
        let start = needle_deg(red_from);
        let mut wedge = vec![pivot];
        wedge.extend(arc_points(pivot, radius * 0.98, start, right));
        painter.add(egui::Shape::convex_polygon(
            wedge,
            egui::Color32::from_rgba_unmultiplied(186, 32, 28, 70),
            egui::Stroke::NONE,
        ));
    }

    for tick in &scale.ticks {
        let deg = needle_deg(tick.frac);
        let outer = radius * if tick.major { 0.98 } else { 0.90 };
        let inner = radius * if tick.major { 0.78 } else { 0.84 };
        painter.line_segment(
            [arc_point(pivot, inner, deg), arc_point(pivot, outer, deg)],
            egui::Stroke::new(
                if tick.major { 1.4 } else { 1.0 },
                egui::Color32::from_rgb(28, 22, 16),
            ),
        );
        if let Some(label) = &tick.label {
            painter.text(
                arc_point(pivot, radius * 0.64, deg),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::monospace(9.0),
                egui::Color32::from_rgb(28, 22, 16),
            );
        }
    }

    painter.text(
        egui::pos2(pivot.x, pivot.y - radius * 0.30),
        egui::Align2::CENTER_CENTER,
        &scale.name,
        egui::FontId::proportional(18.0),
        egui::Color32::from_rgb(168, 28, 24),
    );

    let tip = arc_point(pivot, radius * 0.94, needle_deg(frac));
    painter.line_segment(
        [pivot, tip],
        egui::Stroke::new(1.6, egui::Color32::from_rgb(146, 24, 18)),
    );
    painter.circle_filled(pivot, 3.5, egui::Color32::from_rgb(48, 32, 24));
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
    fn needle_floats_across_the_scale() {
        let mut needle = Needle::at(0.0);
        needle = step_needle(needle, 1.0, 1.0 / 60.0);
        assert!(needle.frac < 0.12, "one frame jumped to {}", needle.frac);
        assert!(needle.frac > 0.0);
        let mut steps = 1;
        while steps < 180 && !needle_settled(needle, 1.0) {
            needle = step_needle(needle, 1.0, 1.0 / 60.0);
            steps += 1;
        }
        assert!(needle.frac > 0.95, "did not arrive: {}", needle.frac);
        assert!(steps > 20, "settled in {steps} frames");
        assert!(steps < 80, "still moving after {steps} frames");
        let parked = step_needle(Needle::at(0.4), 0.4, f32::NAN);
        assert!((parked.frac - 0.4).abs() < 0.001);
        assert_eq!(step_needle(Needle::at(0.2), 0.2, 0.0).vel, 0.0);
    }

    #[test]
    fn needle_rests_left_and_stops_right() {
        assert!((needle_deg(0.0) + 62.0).abs() < 0.01);
        assert!((needle_deg(0.5) - 0.0).abs() < 0.01);
        assert!((needle_deg(1.0) - 62.0).abs() < 0.01);
        assert!((needle_deg(f32::NAN) + 62.0).abs() < 0.01);
    }

    #[test]
    fn swr_red_zone_starts_at_three() {
        let scale = swr_scale();
        assert_eq!(scale.name, "SWR");
        let three = scale
            .ticks
            .iter()
            .find(|tick| tick.label.as_deref() == Some("3"))
            .unwrap();
        assert!((three.frac - 0.5).abs() < 0.001);
        assert!((scale.red_from.unwrap() - 0.5).abs() < 0.001);
        let five = scale
            .ticks
            .iter()
            .find(|tick| tick.label.as_deref() == Some("5"))
            .unwrap();
        assert!((five.frac - 1.0).abs() < 0.001);
    }

    #[test]
    fn s_scale_matches_the_strength_span() {
        let scale = s_scale();
        assert_eq!(scale.name, "S");
        let nine = scale
            .ticks
            .iter()
            .find(|tick| tick.label.as_deref() == Some("9"))
            .unwrap();
        assert!((nine.frac - strength_frac(0.0)).abs() < 0.001);
        let plus = scale
            .ticks
            .iter()
            .find(|tick| tick.label.as_deref() == Some("+60"))
            .unwrap();
        assert!((plus.frac - 1.0).abs() < 0.001);
        assert!(scale.red_from.unwrap() > nine.frac);
    }

    #[test]
    fn volts_and_milliamps_use_the_full_scale_number() {
        let volts = volts_scale(13.8);
        assert_eq!(volts.name, "V");
        assert_eq!(
            volts
                .ticks
                .iter()
                .filter_map(|tick| tick.label.as_deref())
                .collect::<Vec<_>>(),
            ["0", "6.9", "13.8"]
        );
        let ma = milliamps_scale(100.0);
        assert_eq!(ma.name, "mA");
        assert!(ma
            .ticks
            .iter()
            .any(|tick| tick.label.as_deref() == Some("100")));
        assert!(volts_scale(0.0).ticks.is_empty());
        assert!(milliamps_scale(f32::NAN).ticks.is_empty());
    }

    #[test]
    fn red_lamp_shifts_the_glow_and_the_face_stays_cream() {
        let warm = lamp_glow(Lamp::Warm);
        let red = lamp_glow(Lamp::Red);
        let blue = lamp_glow(Lamp::Blue);
        let green = lamp_glow(Lamp::Green);
        assert!(red.g() < warm.g());
        assert!(red.r() > 240);
        assert!(blue.b() > warm.b());
        assert!(green.g() > red.g());
        assert_eq!(METER_FACE, egui::Color32::from_rgb(255, 243, 214));
        assert_ne!(METER_FACE, red);
    }

    #[test]
    fn meter_faces_are_the_readonly_levels_and_not_the_sliders() {
        let get = [
            ("AF", 0.0, 1.0),
            ("STRENGTH", -54.0, 60.0),
            ("ALC", 0.0, 1.0),
            ("SWR", 1.0, 10.0),
            ("RFPOWER_METER", 0.0, 1.0),
            ("VD_METER", 0.0, 15.0),
            ("RAWSTR", 0.0, 0.0),
        ];
        let values = [
            ("STRENGTH", 0.0),
            ("ALC", 0.5),
            ("SWR", 3.0),
            ("RFPOWER_METER", 0.25),
            ("VD_METER", 12.0),
            ("AF", 0.2),
        ];
        let faces = meter_faces(&get, &values, 0.0, true);
        let ids: Vec<&str> = faces.iter().map(|face| face.id.as_str()).collect();
        assert_eq!(ids, ["STRENGTH", "ALC", "SWR", "RFPOWER_METER", "VD_METER"]);
        assert_eq!(faces[2].scale.name, "SWR");
        assert!((faces[2].frac - 0.5).abs() < 0.001);
        assert_eq!(faces[3].lamp, Lamp::Red);
        assert_eq!(faces[0].lamp, Lamp::Warm);
        assert_eq!(faces[4].scale.name, "VD");
    }

    #[test]
    fn power_falls_back_to_rfpower_when_the_radio_has_no_meter() {
        let get = [("RFPOWER", 0.0, 1.0), ("AF", 0.0, 1.0)];
        let faces = meter_faces(&get, &[("RFPOWER", 0.4)], -54.0, false);
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].id, "RFPOWER");
        assert_eq!(faces[0].scale.name, "PWR");
        assert_eq!(faces[0].lamp, Lamp::Warm);
    }

    #[test]
    fn arrange_keeps_a_saved_order_and_appends_a_new_meter() {
        let faces = meter_faces(
            &[
                ("STRENGTH", -54.0, 60.0),
                ("ALC", 0.0, 1.0),
                ("SWR", 1.0, 5.0),
            ],
            &[],
            -54.0,
            false,
        );
        let saved = ["SWR".into(), "GONE".into(), "STRENGTH".into()];
        assert_eq!(
            arrange_meter_ids(&saved, &faces),
            ["SWR", "STRENGTH", "ALC"]
        );
        let order = ["STRENGTH".into(), "ALC".into(), "SWR".into()];
        assert_eq!(move_meter(&order, 0, 2), ["ALC", "SWR", "STRENGTH"]);
        let stored = [
            "SWR".to_string(),
            "VD_METER".to_string(),
            "STRENGTH".to_string(),
            "GONE".to_string(),
        ];
        let visible = ["STRENGTH".to_string(), "SWR".to_string(), "ALC".to_string()];
        assert_eq!(
            merge_meter_order(&stored, &visible),
            ["STRENGTH", "VD_METER", "SWR", "ALC"]
        );
    }

    #[test]
    fn a_drag_in_the_gap_is_not_a_meter() {
        assert_eq!(meter_hit(10.0, 10.0, 2, 3), Some(0));
        assert_eq!(meter_hit(METER_W + 1.0, 10.0, 2, 3), None);
        assert_eq!(meter_hit(METER_W + METER_GAP + 2.0, 4.0, 2, 3), Some(1));
        assert_eq!(meter_hit(4.0, METER_H + METER_GAP + 4.0, 2, 3), Some(2));
        assert_eq!(meter_columns(METER_W * 2.0 + METER_GAP), 2);
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

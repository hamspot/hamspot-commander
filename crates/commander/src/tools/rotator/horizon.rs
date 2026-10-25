//! Elevation "globe horizon" inside the azimuth dial (issue #84), after an
//! aircraft attitude indicator. Geometry is pure and tested; the painter uses
//! muted tones so the amber azimuth scale stays readable on top.

/// Degrees of elevation from the window center to its edge.
pub const VIEW_DEG: f64 = 45.0;

/// Pixels per degree for a window of radius `r`.
pub fn px_per_deg(r: f64) -> f64 {
    r / VIEW_DEG
}

/// Screen y offset from the window center (down positive) of the line for
/// elevation `pitch` while the antenna is at `el`. Raising the antenna moves
/// the horizon (pitch 0) down, like a climbing aircraft.
pub fn pitch_dy(pitch: f64, el: f64, r: f64) -> f64 {
    (el - pitch) * px_per_deg(r)
}

/// Horizon offset, clamped just outside the window so the fill stays sane.
pub fn horizon_dy(el: f64, r: f64) -> f64 {
    pitch_dy(0.0, el, r).clamp(-r * 1.01, r * 1.01)
}

/// Half width of the window at offset `dy`. None outside the circle.
pub fn half_chord(dy: f64, r: f64) -> Option<f64> {
    (dy.abs() < r).then(|| (r * r - dy * dy).sqrt())
}

/// Ladder rungs: every 5 from -10 to `max_el`. A labelled rung every 15.
pub fn ladder(max_el: f64) -> Vec<(f64, bool)> {
    let top = (max_el / 5.0).floor() as i32;
    (-2..=top)
        .map(|i| {
            let p = i as f64 * 5.0;
            (p, i % 3 == 0 && i != 0)
        })
        .collect()
}

/// Sky polygon (the part of the window above the horizon), as offsets from the
/// center. Empty when the horizon is above the window; a full circle when below.
pub fn sky_polygon(hy: f64, r: f64, steps: usize) -> Vec<(f64, f64)> {
    if hy <= -r {
        return Vec::new();
    }
    let (a, b) = if hy >= r {
        (0.0, std::f64::consts::TAU)
    } else {
        // Screen y = r sin(t). Sky is sin(t) < hy/r, running from pi - asin to 2pi + asin.
        let s = (hy / r).asin();
        (std::f64::consts::PI - s, std::f64::consts::TAU + s)
    };
    (0..=steps)
        .map(|i| {
            let t = a + (b - a) * i as f64 / steps as f64;
            (r * t.cos(), r * t.sin())
        })
        .collect()
}

const SKY: egui::Color32 = egui::Color32::from_rgb(0x3A, 0x36, 0x32);
const GROUND: egui::Color32 = egui::Color32::from_rgb(0x2A, 0x1E, 0x14);
const LINE: egui::Color32 = egui::Color32::from_rgb(0x8C, 0x78, 0x5C);
const HORIZON: egui::Color32 = egui::Color32::from_rgb(0xC8, 0xA8, 0x78);
const WINGS: egui::Color32 = egui::Color32::from_rgb(0xFF, 0xA8, 0x2E);
const PRESET: egui::Color32 = egui::Color32::from_rgb(0x50, 0xD0, 0x70);

/// Draw the sphere centered at `c` with radius `r`. `el` None shows the OFF flag.
pub fn paint(
    painter: &egui::Painter,
    c: egui::Pos2,
    r: f32,
    el: Option<f64>,
    preset_el: Option<f64>,
    max_el: f64,
) {
    let rr = r as f64;
    let e = el.unwrap_or(0.0);
    let at = |x: f64, y: f64| c + egui::vec2(x as f32, y as f32);
    painter.circle_filled(c, r, GROUND);
    let hy = horizon_dy(e, rr);
    let sky: Vec<egui::Pos2> = sky_polygon(hy, rr, 64).into_iter().map(|(x, y)| at(x, y)).collect();
    if sky.len() > 2 {
        painter.add(egui::Shape::convex_polygon(sky, SKY, egui::Stroke::NONE));
    }
    // Meridians: fixed ellipses, a sphere seen face on.
    for frac in [0.35_f64, 0.7] {
        let pts: Vec<egui::Pos2> = (0..=48)
            .map(|i| {
                let t = std::f64::consts::TAU * i as f64 / 48.0;
                at(rr * frac * t.cos(), rr * t.sin())
            })
            .collect();
        painter.add(egui::Shape::closed_line(pts, egui::Stroke::new(0.8, LINE.gamma_multiply(0.5))));
    }
    // Ladder.
    for (pitch, labelled) in ladder(max_el) {
        let dy = pitch_dy(pitch, e, rr);
        let Some(half) = half_chord(dy, rr) else { continue };
        let (len, stroke) = if pitch == 0.0 {
            (half, egui::Stroke::new(2.0, HORIZON))
        } else if labelled {
            ((rr * 0.45).min(half), egui::Stroke::new(1.2, LINE))
        } else {
            ((rr * 0.2).min(half), egui::Stroke::new(1.0, LINE))
        };
        painter.line_segment([at(-len, dy), at(len, dy)], stroke);
        if labelled && len + 4.0 < half {
            let font = egui::FontId::proportional((r * 0.13).max(8.0));
            let text = format!("{pitch:.0}");
            painter.text(at(len + 3.0, dy), egui::Align2::LEFT_CENTER, &text, font.clone(), LINE);
            painter.text(at(-len - 3.0, dy), egui::Align2::RIGHT_CENTER, &text, font, LINE);
        }
    }
    // Rim, and the fixed pointer triangles top and bottom.
    painter.circle_stroke(c, r, egui::Stroke::new(1.5, LINE));
    let tri = r * 0.08;
    for (y, dir) in [(-rr, 1.0_f32), (rr, -1.0)] {
        let tip = at(0.0, y) + egui::vec2(0.0, tri * 1.4 * dir);
        let base = at(0.0, y);
        painter.add(egui::Shape::convex_polygon(
            vec![tip, base + egui::vec2(-tri, 0.0), base + egui::vec2(tri, 0.0)],
            WINGS,
            egui::Stroke::NONE,
        ));
    }
    // EL preset marker on the right edge.
    if let Some(p) = preset_el {
        let dy = pitch_dy(p, e, rr);
        if let Some(half) = half_chord(dy, rr) {
            let tip = at(half - 2.0, dy);
            let m = r * 0.07;
            painter.add(egui::Shape::convex_polygon(
                vec![tip, tip + egui::vec2(m * 1.4, -m), tip + egui::vec2(m * 1.4, m)],
                PRESET,
                egui::Stroke::NONE,
            ));
        }
    }
    // Fixed wings reference.
    let w = r * 0.42;
    let s = egui::Stroke::new(3.0, WINGS);
    painter.line_segment([c + egui::vec2(-w, 0.0), c + egui::vec2(-w * 0.35, 0.0)], s);
    painter.line_segment([c + egui::vec2(w * 0.35, 0.0), c + egui::vec2(w, 0.0)], s);
    painter.line_segment([c + egui::vec2(-w * 0.35, 0.0), c + egui::vec2(0.0, w * 0.18)], s);
    painter.line_segment([c + egui::vec2(w * 0.35, 0.0), c + egui::vec2(0.0, w * 0.18)], s);
    if el.is_none() {
        let flag = egui::Rect::from_center_size(
            c + egui::vec2(-r * 0.45, -r * 0.45),
            egui::vec2(r * 0.34, r * 0.16),
        );
        painter.rect_filled(flag, egui::CornerRadius::same(2), egui::Color32::from_rgb(0xB0, 0x20, 0x18));
        painter.text(
            flag.center(),
            egui::Align2::CENTER_CENTER,
            "OFF",
            egui::FontId::proportional(flag.height() * 0.8),
            egui::Color32::WHITE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizon_moves_down_as_elevation_rises() {
        assert_eq!(pitch_dy(0.0, 0.0, 90.0), 0.0);
        assert_eq!(pitch_dy(0.0, 45.0, 90.0), 90.0);
        assert_eq!(pitch_dy(30.0, 30.0, 90.0), 0.0);
        assert_eq!(pitch_dy(90.0, 45.0, 90.0), -90.0);
        assert!((horizon_dy(90.0, 90.0) - 90.9).abs() < 1e-9);
        assert!((horizon_dy(-90.0, 90.0) + 90.9).abs() < 1e-9);
    }

    #[test]
    fn chords_stay_inside_the_window() {
        assert_eq!(half_chord(0.0, 10.0), Some(10.0));
        assert_eq!(half_chord(6.0, 10.0), Some(8.0));
        assert_eq!(half_chord(10.0, 10.0), None);
    }

    #[test]
    fn ladder_every_five_labelled_every_fifteen_to_zenith() {
        let l = ladder(90.0);
        assert_eq!(l.first(), Some(&(-10.0, false)));
        assert_eq!(l.last(), Some(&(90.0, true)));
        assert_eq!(l.len(), 21);
        assert!(l.contains(&(15.0, true)));
        assert!(l.contains(&(0.0, false)));
        assert!(l.contains(&(10.0, false)));
        assert_eq!(ladder(180.0).last(), Some(&(180.0, true)));
    }

    #[test]
    fn sky_fills_above_the_horizon() {
        assert!(sky_polygon(-11.0, 10.0, 8).is_empty());
        let full = sky_polygon(11.0, 10.0, 8);
        assert_eq!(full.len(), 9);
        let half = sky_polygon(0.0, 10.0, 8);
        assert!(half.iter().all(|(_, y)| *y <= 1e-9));
        assert!((half[0].0 + 10.0).abs() < 1e-9);
        assert!((half[8].0 - 10.0).abs() < 1e-9);
    }
}

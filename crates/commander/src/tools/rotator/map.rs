//! Optional azimuthal-equidistant world map behind the rotator rose
//! (David, 2026-10-30). Implemented directly (no projection crate): from the
//! map centre, a point's great-circle bearing is its screen angle (0 up,
//! clockwise, same as the rose) and its distance is the radius. The rose edge
//! is either the antipode (half the circumference) or a chosen range in km.
//! Coastlines: Natural Earth 1:110m, public domain, bundled as
//! `data/coastline-110m.txt` (see licenses/NaturalEarth.md), parsed once.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

pub const MAP_VERSION: u32 = 1;
pub const EARTH_KM: f64 = 6371.0088;
const DATA: &str = include_str!("../../../data/coastline-110m.txt");
const HEADER: &str = "# hamspot coastline v1";

/// Saved with the rotator settings. Text fields keep what was typed so a bad
/// entry stays visible and is reported, never silently replaced.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MapSettings {
    pub version: u32,
    pub show: bool,
    pub lat: String,
    pub lon: String,
    /// Range at the rose edge in km; blank = whole world (antipode at the edge).
    pub range_km: String,
}

impl Default for MapSettings {
    fn default() -> Self {
        Self { version: MAP_VERSION, show: false, lat: String::new(), lon: String::new(), range_km: String::new() }
    }
}

pub fn parse_deg(text: &str, what: &str, max: f64) -> Result<f64, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err(format!("map {what} is empty"));
    }
    match t.parse::<f64>() {
        Ok(v) if v.is_finite() && v.abs() <= max => Ok(v),
        Ok(_) => Err(format!("map {what} {t} is outside ±{max:.0}°")),
        Err(_) => Err(format!("map {what} \"{t}\" is not decimal degrees")),
    }
}

/// Centre (lat, lon) and edge distance in radians, or the reason it is unusable.
pub fn resolve(s: &MapSettings) -> Result<(f64, f64, f64), String> {
    let lat = parse_deg(&s.lat, "latitude", 90.0)?;
    let lon = parse_deg(&s.lon, "longitude", 180.0)?;
    let edge = match s.range_km.trim() {
        "" => std::f64::consts::PI,
        t => match t.parse::<f64>() {
            Ok(km) if km.is_finite() && km >= 100.0 && km <= std::f64::consts::PI * EARTH_KM => km / EARTH_KM,
            _ => return Err(format!("map range \"{t}\" km must be 100…20015 or blank")),
        },
    };
    Ok((lat, lon, edge))
}

/// Bearing (degrees, 0 north, clockwise) and angular distance (radians) from
/// (lat0, lon0) to (lat, lon). Inputs in degrees. Poles do not panic.
pub fn bearing_dist(lat0: f64, lon0: f64, lat: f64, lon: f64) -> (f64, f64) {
    let (p0, p) = (lat0.to_radians(), lat.to_radians());
    let dl = (lon - lon0).to_radians();
    let y = dl.sin() * p.cos();
    let x = p0.cos() * p.sin() - p0.sin() * p.cos() * dl.cos();
    let bearing = y.atan2(x).to_degrees().rem_euclid(360.0);
    // atan2 form is stable near 0 and near the antipode.
    let c = (p0.sin() * p.sin() + p0.cos() * p.cos() * dl.cos()).clamp(-1.0, 1.0);
    let s = (x * x + y * y).sqrt();
    (bearing, s.atan2(c))
}

/// Unit-disc point (x right, y down; 1.0 = rose edge) or None past the edge.
pub fn project(lat0: f64, lon0: f64, edge: f64, lat: f64, lon: f64) -> Option<(f32, f32)> {
    let (b, d) = bearing_dist(lat0, lon0, lat, lon);
    if !(d.is_finite() && d <= edge) {
        return None;
    }
    let (x, y) = super::geom::dial_xy(b, d / edge);
    Some((x as f32, y as f32))
}

/// Polylines as (lon, lat) degrees.
pub fn coastlines() -> Result<&'static [Vec<(f64, f64)>], String> {
    static LINES: OnceLock<Result<Vec<Vec<(f64, f64)>>, String>> = OnceLock::new();
    LINES.get_or_init(|| parse(DATA)).as_ref().map(|v| v.as_slice()).map_err(|e| e.clone())
}

pub fn parse(text: &str) -> Result<Vec<Vec<(f64, f64)>>, String> {
    if !text.starts_with(HEADER) {
        return Err("map data: unknown coastline format".into());
    }
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate().filter(|(_, l)| !l.starts_with('#') && !l.trim().is_empty()) {
        let mut pts = Vec::new();
        for pair in line.split_whitespace() {
            let (a, b) = pair.split_once(',').ok_or_else(|| format!("map data line {}: bad pair", n + 1))?;
            let (lon, lat) = (a.parse::<f64>(), b.parse::<f64>());
            match (lon, lat) {
                (Ok(lon), Ok(lat)) if lon.abs() <= 180.0 && lat.abs() <= 90.0 => pts.push((lon, lat)),
                _ => return Err(format!("map data line {}: bad coordinate {pair}", n + 1)),
            }
        }
        out.push(pts);
    }
    if out.is_empty() {
        return Err("map data: no coastlines".into());
    }
    Ok(out)
}

/// A jump longer than this (unit-disc) between neighbours is a wrap through
/// the antipode or the edge, so the line is split there.
const MAX_STEP: f32 = 0.35;

/// Re-projected unit-disc polylines, split at the edge and the antipode.
pub fn project_lines(lines: &[Vec<(f64, f64)>], lat0: f64, lon0: f64, edge: f64) -> Vec<Vec<(f32, f32)>> {
    let mut out = Vec::new();
    for line in lines {
        let mut cur: Vec<(f32, f32)> = Vec::new();
        for &(lon, lat) in line {
            match project(lat0, lon0, edge, lat, lon) {
                Some(p) => {
                    if let Some(&(px, py)) = cur.last() {
                        if ((p.0 - px).powi(2) + (p.1 - py).powi(2)).sqrt() > MAX_STEP {
                            if cur.len() >= 2 {
                                out.push(std::mem::take(&mut cur));
                            }
                            cur.clear();
                        }
                    }
                    cur.push(p);
                }
                None => {
                    if cur.len() >= 2 {
                        out.push(std::mem::take(&mut cur));
                    }
                    cur.clear();
                }
            }
        }
        if cur.len() >= 2 {
            out.push(cur);
        }
    }
    out
}

/// Projection cache: re-projects only when the centre or range changes.
#[derive(Default)]
pub struct MapCache {
    key: Option<(f64, f64, f64)>,
    pub lines: Vec<Vec<(f32, f32)>>,
}

impl MapCache {
    /// Ok(lines) for the settings, or the reason the map can't be drawn.
    pub fn get(&mut self, s: &MapSettings) -> Result<&[Vec<(f32, f32)>], String> {
        let key = resolve(s)?;
        let data = coastlines()?;
        if self.key != Some(key) {
            self.lines = project_lines(data, key.0, key.1, key.2);
            self.key = Some(key);
        }
        Ok(&self.lines)
    }
}

/// Faint coastline colour: visible on the dark face, well under the amber rose.
pub const MAP_COLOR: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x3C, 0x6E, 0x78, 0xA0);

pub fn paint(painter: &egui::Painter, center: egui::Pos2, r: f32, lines: &[Vec<(f32, f32)>]) {
    let stroke = egui::Stroke::new(1.0, MAP_COLOR);
    for l in lines {
        let pts: Vec<egui::Pos2> = l.iter().map(|&(x, y)| center + egui::vec2(x * r, y * r)).collect();
        painter.add(egui::Shape::line(pts, stroke));
    }
    // Cheap distance rings at a third and two thirds of the edge.
    for k in [1.0 / 3.0, 2.0 / 3.0] {
        painter.circle_stroke(center, r * k, egui::Stroke::new(1.0, MAP_COLOR.gamma_multiply(0.5)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn centre_maps_to_origin() {
        let p = project(39.77, -86.16, PI, 39.77, -86.16).unwrap();
        assert!(p.0.abs() < 1e-6 && p.1.abs() < 1e-6);
    }

    #[test]
    fn known_bearings_and_distances() {
        // From the equator/prime meridian: 90° east along the equator is a
        // quarter circumference due east.
        let (b, d) = bearing_dist(0.0, 0.0, 0.0, 90.0);
        assert!(close(b, 90.0, 1e-9) && close(d, PI / 2.0, 1e-9));
        let (b, d) = bearing_dist(0.0, 0.0, 45.0, 0.0);
        assert!(close(b, 0.0, 1e-9) && close(d, PI / 4.0, 1e-9));
        // Indianapolis -> London, checked against an independent haversine: 6,420 km.
        let (b, d) = bearing_dist(39.7684, -86.1581, 51.5072, -0.1276);
        assert!(close(d * EARTH_KM, 6_420.2, 1.0), "{}", d * EARTH_KM);
        assert!(close(b, 47.0, 2.0), "{b}");
    }

    #[test]
    fn ninety_east_plots_at_the_rose_east() {
        let (x, y) = project(0.0, 0.0, PI, 0.0, 90.0).unwrap();
        assert!(close(x as f64, 0.5, 1e-6) && y.abs() < 1e-6, "({x},{y})");
        // Same with the rose's own dial helper: east is +x on screen.
        let (ex, ey) = super::super::geom::dial_xy(90.0, 1.0);
        assert!(ex > 0.99 && ey.abs() < 1e-9);
    }

    #[test]
    fn antipode_sits_on_the_edge_and_lines_split_there() {
        let (_, d) = bearing_dist(10.0, 20.0, -10.0, -160.0);
        assert!(close(d, PI, 1e-9));
        let p = project(10.0, 20.0, PI, -10.0, -160.0).unwrap();
        assert!(close(((p.0 * p.0 + p.1 * p.1) as f64).sqrt(), 1.0, 1e-5));
        // A line crossing near the antipode jumps across the disc: split.
        let line = vec![(-161.0, -10.0), (-159.0, -10.0)];
        let out = project_lines(&[line], 10.0, 20.0, PI);
        for l in &out {
            for w in l.windows(2) {
                let s = ((w[0].0 - w[1].0).powi(2) + (w[0].1 - w[1].1).powi(2)).sqrt();
                assert!(s <= MAX_STEP, "wrap-around segment {s}");
            }
        }
    }

    #[test]
    fn poles_do_not_panic_and_stay_in_the_disc() {
        for (lat0, lon0) in [(90.0, 0.0), (-90.0, 0.0), (90.0, 180.0), (0.0, 180.0), (0.0, -180.0)] {
            let lines = project_lines(coastlines().unwrap(), lat0, lon0, PI);
            assert!(!lines.is_empty());
            for l in &lines {
                for &(x, y) in l {
                    assert!(x.is_finite() && y.is_finite() && (x * x + y * y).sqrt() <= 1.0 + 1e-4);
                }
            }
        }
        let (b, d) = bearing_dist(90.0, 0.0, 90.0, 0.0);
        assert!(b.is_finite() && d.abs() < 1e-12);
    }

    #[test]
    fn data_parses_and_input_is_validated() {
        assert!(coastlines().unwrap().len() > 100);
        assert!(parse("junk").is_err());
        let mut s = MapSettings::default();
        assert!(!s.show, "off by default");
        assert!(resolve(&s).unwrap_err().contains("latitude"));
        s.lat = "91".into();
        s.lon = "0".into();
        assert!(resolve(&s).unwrap_err().contains("outside"));
        s.lat = "39.77".into();
        s.lon = "abc".into();
        assert!(resolve(&s).unwrap_err().contains("decimal"));
        s.lon = "-86.16".into();
        assert_eq!(resolve(&s).unwrap().2, PI);
        s.range_km = "5".into();
        assert!(resolve(&s).is_err());
        s.range_km = "5000".into();
        assert!(close(resolve(&s).unwrap().2, 5000.0 / EARTH_KM, 1e-12));
    }

    #[test]
    fn range_clips_far_points() {
        assert!(project(0.0, 0.0, 1000.0 / EARTH_KM, 0.0, 90.0).is_none());
    }
}

//! Optional azimuthal-equidistant world map behind the rotator rose
//! (David, 2026-10-30). Implemented directly (no projection crate): from the
//! map centre, a point's great-circle bearing is its screen angle (0 up,
//! clockwise, same as the rose) and its distance is the radius. The rose edge
//! is either the antipode (half the circumference) or a chosen range in km.
//! Land: Natural Earth 1:110m land polygons, public domain, bundled as
//! `data/land-110m.txt` (see licenses/NaturalEarth.md), parsed and
//! triangulated (earcut, then split to small spherical triangles) once.
//! Ocean is a black disc, land dark-blue triangles, coasts a faint outline.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

pub const MAP_VERSION: u32 = 1;
pub const EARTH_KM: f64 = 6371.0088;
const DATA: &str = include_str!("../../../data/land-110m.txt");
const HEADER: &str = "# hamspot land v1";

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

/// One land polygon: outer ring and holes, (lon, lat) degrees.
#[derive(Clone, Debug, Default)]
pub struct Poly {
    pub outer: Vec<(f64, f64)>,
    pub holes: Vec<Vec<(f64, f64)>>,
}

/// Parsed once: polygons, their coastline outlines, and sphere triangles.
pub struct Land {
    pub polys: Vec<Poly>,
    /// Coast outlines without the artificial ±180° / -90° edges of the data.
    pub coasts: Vec<Vec<(f64, f64)>>,
    pub tris: Vec<[(f64, f64); 3]>,
}

pub fn land() -> Result<&'static Land, String> {
    static LAND: OnceLock<Result<Land, String>> = OnceLock::new();
    LAND.get_or_init(|| parse(DATA).map(build)).as_ref().map_err(|e| e.clone())
}

fn pair(p: &str, n: usize) -> Result<(f64, f64), String> {
    let (a, b) = p.split_once(',').ok_or_else(|| format!("map data line {n}: bad pair"))?;
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(lon), Ok(lat)) if lon.abs() <= 180.0 && lat.abs() <= 90.0 => Ok((lon, lat)),
        _ => Err(format!("map data line {n}: bad coordinate {p}")),
    }
}

pub fn parse(text: &str) -> Result<Vec<Poly>, String> {
    if !text.starts_with(HEADER) {
        return Err("map data: unknown land format".into());
    }
    let mut out: Vec<Poly> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let (tag, rest) = line.split_at(line.len().min(2));
        let ring = rest.split_whitespace().map(|p| pair(p, n)).collect::<Result<Vec<_>, _>>()?;
        if ring.len() < 3 {
            return Err(format!("map data line {n}: ring under 3 points"));
        }
        match tag {
            "P " => out.push(Poly { outer: ring, holes: Vec::new() }),
            "H " => out.last_mut().ok_or(format!("map data line {n}: hole before a polygon"))?.holes.push(ring),
            _ => return Err(format!("map data line {n}: expected P or H")),
        }
    }
    if out.is_empty() {
        return Err("map data: no land".into());
    }
    Ok(out)
}

/// Longest side of a sphere triangle before it is split, so projected
/// straight sides follow the curved map.
const MAX_SIDE: f64 = 10.0 * std::f64::consts::PI / 180.0;

fn side(a: (f64, f64), b: (f64, f64)) -> f64 {
    bearing_dist(a.1, a.0, b.1, b.0).1
}

fn mid(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

fn split_sphere(t: [(f64, f64); 3], out: &mut Vec<[(f64, f64); 3]>, depth: u8) {
    let [a, b, c] = t;
    if depth >= 6 || (side(a, b) <= MAX_SIDE && side(b, c) <= MAX_SIDE && side(c, a) <= MAX_SIDE) {
        out.push(t);
        return;
    }
    let (ab, bc, ca) = (mid(a, b), mid(b, c), mid(c, a));
    for s in [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]] {
        split_sphere(s, out, depth + 1);
    }
}

fn is_artificial(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0.abs() >= 179.99 && b.0.abs() >= 179.99) || (a.1 <= -89.99 && b.1 <= -89.99)
}

fn build(polys: Vec<Poly>) -> Land {
    let mut tris = Vec::new();
    let mut coasts = Vec::new();
    let mut ec = earcut::Earcut::<f64>::new();
    let mut idx: Vec<u32> = Vec::new();
    for p in &polys {
        let mut pts: Vec<[f64; 2]> = p.outer.iter().map(|&(x, y)| [x, y]).collect();
        let mut holes = Vec::new();
        for h in &p.holes {
            holes.push(pts.len() as u32);
            pts.extend(h.iter().map(|&(x, y)| [x, y]));
        }
        ec.earcut(pts.iter().copied(), &holes, &mut idx);
        for t in idx.chunks_exact(3) {
            let v = |i: u32| (pts[i as usize][0], pts[i as usize][1]);
            split_sphere([v(t[0]), v(t[1]), v(t[2])], &mut tris, 0);
        }
        for ring in std::iter::once(&p.outer).chain(&p.holes) {
            let mut cur: Vec<(f64, f64)> = Vec::new();
            for k in 0..=ring.len() {
                let q = ring[k % ring.len()];
                if let Some(&last) = cur.last() {
                    if is_artificial(last, q) {
                        if cur.len() >= 2 {
                            coasts.push(std::mem::take(&mut cur));
                        }
                        cur.clear();
                    }
                }
                cur.push(q);
            }
            if cur.len() >= 2 {
                coasts.push(cur);
            }
        }
    }
    Land { polys, coasts, tris }
}

fn ring_has(ring: &[(f64, f64)], lon: f64, lat: f64) -> bool {
    let mut inside = false;
    let n = ring.len();
    for i in 0..n {
        let (x1, y1) = ring[i];
        let (x2, y2) = ring[(i + 1) % n];
        if (y1 > lat) != (y2 > lat) && lon < x1 + (lat - y1) * (x2 - x1) / (y2 - y1) {
            inside = !inside;
        }
    }
    inside
}

/// Is (lat, lon) on land, from the source polygons (holes are water).
pub fn on_land(land: &Land, lat: f64, lon: f64) -> bool {
    land.polys.iter().any(|p| ring_has(&p.outer, lon, lat) && !p.holes.iter().any(|h| ring_has(h, lon, lat)))
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

/// Angular distance from the antipode inside which triangles are split and,
/// at the last level, dropped: there the projection tears around the rim.
const ANTI_GUARD: f64 = 0.15;
const MAX_DEPTH: u8 = 4;

struct Proj {
    lat0: f64,
    lon0: f64,
    edge: f64,
}

impl Proj {
    /// Unclamped disc point and angular distance.
    fn at(&self, (lon, lat): (f64, f64)) -> (f64, f64, f64) {
        let (b, d) = bearing_dist(self.lat0, self.lon0, lat, lon);
        let (x, y) = super::geom::dial_xy(b, d / self.edge);
        (x, y, d)
    }
}

fn seg_hits_disc(a: (f64, f64), b: (f64, f64)) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (-(a.0 * dx + a.1 * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (x, y) = (a.0 + t * dx, a.1 + t * dy);
    x * x + y * y <= 1.0
}

fn has_origin(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> bool {
    let s = |p: (f64, f64), q: (f64, f64)| p.0 * q.1 - p.1 * q.0;
    let (d1, d2, d3) = (s(a, b), s(b, c), s(c, a));
    (d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0) || (d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0)
}

fn clamp1(p: (f64, f64)) -> [f32; 2] {
    let r = (p.0 * p.0 + p.1 * p.1).sqrt();
    let k = if r > 1.0 { 1.0 / r } else { 1.0 };
    [(p.0 * k) as f32, (p.1 * k) as f32]
}

fn emit(pr: &Proj, t: [(f64, f64); 3], depth: u8, out: &mut Vec<[f32; 2]>) {
    let p = t.map(|v| pr.at(v));
    let xy = p.map(|(x, y, _)| (x, y));
    let near_anti = p.iter().any(|q| q.2 > std::f64::consts::PI - ANTI_GUARD);
    let inside = p.iter().filter(|q| q.2 <= pr.edge).count();
    let split = |out: &mut Vec<[f32; 2]>| {
        let [a, b, c] = t;
        let (ab, bc, ca) = (mid(a, b), mid(b, c), mid(c, a));
        for s in [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]] {
            emit(pr, s, depth + 1, out);
        }
    };
    if near_anti {
        // Only the whole-world map reaches the antipode; keep splitting so
        // the torn triangle shrinks to the rim, then drop it.
        if depth < MAX_DEPTH && pr.edge > std::f64::consts::PI - ANTI_GUARD {
            split(out);
        }
        return;
    }
    if inside == 3 {
        out.extend(xy.map(clamp1));
        return;
    }
    let touches = inside > 0
        || has_origin(xy[0], xy[1], xy[2])
        || seg_hits_disc(xy[0], xy[1])
        || seg_hits_disc(xy[1], xy[2])
        || seg_hits_disc(xy[2], xy[0]);
    if !touches {
        return;
    }
    if depth < MAX_DEPTH {
        split(out);
    } else {
        out.extend(xy.map(clamp1));
    }
}

/// Land triangles in the unit disc (3 points each) for a centre and edge.
pub fn project_land(land: &Land, lat0: f64, lon0: f64, edge: f64) -> Vec<[f32; 2]> {
    let pr = Proj { lat0, lon0, edge };
    let mut out = Vec::new();
    for t in &land.tris {
        emit(&pr, *t, 0, &mut out);
    }
    out
}

pub struct Projected {
    /// Land triangles, three points each, unit disc.
    pub tris: Vec<[f32; 2]>,
    pub coasts: Vec<Vec<(f32, f32)>>,
}

/// Projection cache: re-projects only when the centre or range changes.
#[derive(Default)]
pub struct MapCache {
    key: Option<(f64, f64, f64)>,
    data: Option<Projected>,
}

impl MapCache {
    /// The projected map, or the reason it can't be drawn.
    pub fn get(&mut self, s: &MapSettings) -> Result<&Projected, String> {
        let key = resolve(s)?;
        let land = land()?;
        if self.key != Some(key) || self.data.is_none() {
            self.data = Some(Projected {
                tris: project_land(land, key.0, key.1, key.2),
                coasts: project_lines(&land.coasts, key.0, key.1, key.2),
            });
            self.key = Some(key);
        }
        Ok(self.data.as_ref().expect("set above"))
    }
}

/// Water inside the map circle.
pub const OCEAN_COLOR: egui::Color32 = egui::Color32::BLACK;
/// Land fill: dark blue, well under the amber rose.
pub const LAND_COLOR: egui::Color32 = egui::Color32::from_rgb(0x12, 0x26, 0x5A);
/// Faint coast outline and range rings.
pub const MAP_COLOR: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x3C, 0x6E, 0x78, 0xA0);

/// Ocean disc and land fill (drawn under the elevation globe).
pub fn paint_fill(painter: &egui::Painter, center: egui::Pos2, r: f32, map: &Projected) {
    painter.circle_filled(center, r, OCEAN_COLOR);
    let mut mesh = egui::Mesh::default();
    mesh.reserve_vertices(map.tris.len());
    mesh.reserve_triangles(map.tris.len() / 3);
    for (i, p) in map.tris.iter().enumerate() {
        mesh.colored_vertex(center + egui::vec2(p[0] * r, p[1] * r), LAND_COLOR);
        if i % 3 == 2 {
            let k = i as u32;
            mesh.add_triangle(k - 2, k - 1, k);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Coast outline and range rings (drawn over the globe, under the rose).
pub fn paint(painter: &egui::Painter, center: egui::Pos2, r: f32, map: &Projected) {
    let stroke = egui::Stroke::new(1.0, MAP_COLOR);
    for l in &map.coasts {
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
            let lines = project_lines(&land().unwrap().coasts, lat0, lon0, PI);
            let tris = project_land(land().unwrap(), lat0, lon0, PI);
            assert!(tris.iter().all(|p| p[0].is_finite() && (p[0] * p[0] + p[1] * p[1]).sqrt() <= 1.0 + 1e-4));
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
        assert!(land().unwrap().polys.len() > 100);
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

    fn tri_has(t: &[[f32; 2]], x: f32, y: f32) -> bool {
        t.chunks_exact(3).any(|c| {
            let s = |a: [f32; 2], b: [f32; 2]| (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]);
            let (d1, d2, d3) = (s(c[0], c[1]), s(c[1], c[2]), s(c[2], c[0]));
            (d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0) || (d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0)
        })
    }

    /// Unit-disc point for a place seen from a centre.
    fn spot(lat0: f64, lon0: f64, edge: f64, lat: f64, lon: f64) -> (f32, f32) {
        project(lat0, lon0, edge, lat, lon).unwrap()
    }

    #[test]
    fn indianapolis_centre_is_land_and_mid_pacific_is_water() {
        let l = land().unwrap();
        let (lat0, lon0) = (39.7684, -86.1581);
        assert!(on_land(l, lat0, lon0));
        assert!(!on_land(l, 0.0, -150.0));
        for edge in [PI, 3000.0 / EARTH_KM] {
            let t = project_land(l, lat0, lon0, edge);
            assert!(tri_has(&t, 0.0, 0.0), "centre is filled land at edge {edge}");
            if edge == PI {
                let (x, y) = spot(lat0, lon0, edge, 0.0, -150.0);
                assert!(!tri_has(&t, x, y), "mid-Pacific is water");
                // Far land: Australia (Alice Springs) is filled too.
                let (x, y) = spot(lat0, lon0, edge, -23.7, 133.9);
                assert!(tri_has(&t, x, y), "Australia filled");
            }
        }
    }

    #[test]
    fn no_triangle_outside_and_count_in_budget() {
        let l = land().unwrap();
        assert!(l.tris.len() < 40_000, "sphere triangles {}", l.tris.len());
        for (lat0, lon0, edge) in [(39.77, -86.16, PI), (0.0, 0.0, PI), (-90.0, 0.0, PI), (90.0, 0.0, PI), (39.77, -86.16, 2000.0 / EARTH_KM), (-77.85, 166.67, 5000.0 / EARTH_KM)] {
            let t = project_land(l, lat0, lon0, edge);
            eprintln!("map triangles: sphere {} projected {} at {lat0},{lon0} edge {edge:.3}", l.tris.len(), t.len() / 3);
            assert_eq!(t.len() % 3, 0);
            assert!(t.len() / 3 < 40_000, "{} triangles at {lat0},{lon0}", t.len() / 3);
            assert!(t.iter().all(|p| p[0].is_finite() && p[1].is_finite() && (p[0] * p[0] + p[1] * p[1]).sqrt() <= 1.0 + 1e-4));
        }
    }

    #[test]
    fn south_pole_centre_is_antarctica_and_north_pole_is_ocean() {
        let l = land().unwrap();
        let t = project_land(l, -90.0, 0.0, PI);
        assert!(tri_has(&t, 0.0, 0.0), "south pole on Antarctica");
        let t = project_land(l, 90.0, 0.0, PI);
        assert!(!tri_has(&t, 0.0, 0.0), "north pole is Arctic Ocean");
        // Seen from the north pole, Antarctica sits at the rim, not as a
        // spurious fill across the disc: half radius is mostly water there.
        let (x, y) = spot(90.0, 0.0, PI, 0.0, -150.0);
        assert!(!tri_has(&t, x, y));
    }
}

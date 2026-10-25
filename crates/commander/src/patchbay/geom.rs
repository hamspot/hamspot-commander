//! Panel geometry for the RF patch bay (#93). Pure functions in panel points.

use super::model::{Device, Dir, Jack, PatchBay};

pub const BOX_W: f32 = 168.0;
pub const TITLE_H: f32 = 22.0;
pub const ROW_H: f32 = 20.0;
pub const JACK_R: f32 = 7.0;
/// Snap radius for dropping a cable end on a jack.
pub const SNAP: f32 = 22.0;

pub fn box_size(dev: &Device) -> [f32; 2] {
    let ports = dev.ports();
    let ins = ports.iter().filter(|p| p.dir == Dir::In).count();
    let outs = ports.len() - ins;
    let rows = ins.max(outs).max(1) + if dev.is_bound_amp() { super::bind::STATUS_ROWS } else { 0 };
    [BOX_W, TITLE_H + ROW_H * rows as f32 + 8.0]
}

/// Jack centres: inputs on the left edge, outputs on the right edge.
pub fn jacks(dev: &Device) -> Vec<(String, Dir, [f32; 2])> {
    let (mut i, mut o) = (0, 0);
    dev.ports()
        .into_iter()
        .map(|p| {
            let row = if p.dir == Dir::In { &mut i } else { &mut o };
            let y = dev.pos[1] + TITLE_H + ROW_H * (*row as f32 + 0.5) + 4.0;
            *row += 1;
            let x = if p.dir == Dir::In { dev.pos[0] } else { dev.pos[0] + BOX_W };
            (p.name, p.dir, [x, y])
        })
        .collect()
}

pub fn jack_pos(bay: &PatchBay, jack: &Jack) -> Option<[f32; 2]> {
    let dev = bay.device(jack.device)?;
    jacks(dev).into_iter().find(|(n, _, _)| *n == jack.port).map(|(_, _, p)| p)
}

/// Nearest jack within `radius` for which `ok` holds.
pub fn nearest_jack(bay: &PatchBay, at: [f32; 2], radius: f32, ok: impl Fn(&Jack) -> bool) -> Option<Jack> {
    let mut best: Option<(f32, Jack)> = None;
    for dev in &bay.devices {
        for (name, _, p) in jacks(dev) {
            let d = dist(p, at);
            let jack = Jack::new(dev.id, &name);
            if d <= radius && best.as_ref().map_or(true, |(b, _)| d < *b) && ok(&jack) {
                best = Some((d, jack));
            }
        }
    }
    best.map(|(_, j)| j)
}

pub fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Bezier control points for a hanging cable: leaves each jack sideways and
/// sags under its own weight, more for a longer cable.
pub fn cable_curve(from: [f32; 2], to: [f32; 2]) -> [[f32; 2]; 4] {
    let span = dist(from, to);
    let sag = 18.0 + span * 0.18;
    let lead = (span * 0.35).clamp(30.0, 140.0);
    [
        from,
        [from[0] + lead, from[1] + sag],
        [to[0] - lead, to[1] + sag],
        to,
    ]
}

pub fn bezier(c: [[f32; 2]; 4], t: f32) -> [f32; 2] {
    let u = 1.0 - t;
    let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
    [
        w[0] * c[0][0] + w[1] * c[1][0] + w[2] * c[2][0] + w[3] * c[3][0],
        w[0] * c[0][1] + w[1] * c[1][1] + w[2] * c[2][1] + w[3] * c[3][1],
    ]
}

pub fn sample(c: [[f32; 2]; 4], n: usize) -> Vec<[f32; 2]> {
    (0..=n).map(|i| bezier(c, i as f32 / n as f32)).collect()
}

/// Distance from a point to the sampled cable.
pub fn cable_distance(c: [[f32; 2]; 4], at: [f32; 2]) -> f32 {
    let pts = sample(c, 32);
    pts.windows(2).map(|w| seg_dist(w[0], w[1], at)).fold(f32::MAX, f32::min)
}

fn seg_dist(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    dist([a[0] + t * dx, a[1] + t * dy], p)
}

/// Index of the cable under the pointer, if any.
pub fn cable_at(bay: &PatchBay, at: [f32; 2], radius: f32) -> Option<usize> {
    let mut best: Option<(f32, usize)> = None;
    for (i, c) in bay.cables.iter().enumerate() {
        let (Some(a), Some(b)) = (jack_pos(bay, &c.from), jack_pos(bay, &c.to)) else { continue };
        let d = cable_distance(cable_curve(a, b), at);
        if d <= radius && best.map_or(true, |(bd, _)| d < bd) {
            best = Some((d, i));
        }
    }
    best.map(|(_, i)| i)
}

/// Device whose title bar is under the pointer (for dragging a box).
pub fn title_at(bay: &PatchBay, at: [f32; 2]) -> Option<u32> {
    bay.devices.iter().rev().find(|d| {
        at[0] >= d.pos[0] && at[0] <= d.pos[0] + BOX_W && at[1] >= d.pos[1] && at[1] <= d.pos[1] + TITLE_H
    }).map(|d| d.id)
}

pub fn box_at(bay: &PatchBay, at: [f32; 2]) -> Option<u32> {
    bay.devices.iter().rev().find(|d| {
        let s = box_size(d);
        at[0] >= d.pos[0] && at[0] <= d.pos[0] + s[0] && at[1] >= d.pos[1] && at[1] <= d.pos[1] + s[1]
    }).map(|d| d.id)
}

#[cfg(test)]
mod tests {
    use super::super::model::Kind;
    use super::*;

    #[test]
    fn jacks_sit_on_box_edges() {
        let d = Device::new(1, Kind::Amplifier, "A", [100.0, 50.0]);
        let j = jacks(&d);
        let input = j.iter().find(|(n, _, _)| n == "INPUT").unwrap().2;
        let output = j.iter().find(|(n, _, _)| n == "OUTPUT").unwrap().2;
        assert_eq!(input[0], 100.0);
        assert_eq!(output[0], 100.0 + BOX_W);
        assert_eq!(input[1], output[1]);
        assert!(box_size(&d)[1] > TITLE_H + 2.0 * ROW_H);
    }

    #[test]
    fn snaps_to_the_nearest_allowed_jack() {
        let mut bay = PatchBay::default();
        let a = bay.add(Kind::Amplifier, "A", [0.0, 0.0]);
        let p = jack_pos(&bay, &Jack::new(a, "INPUT")).unwrap();
        let near = [p[0] + 10.0, p[1] + 5.0];
        assert_eq!(nearest_jack(&bay, near, SNAP, |_| true), Some(Jack::new(a, "INPUT")));
        assert_eq!(nearest_jack(&bay, near, SNAP, |j| j.port != "INPUT").map(|j| j.port), Some("PTT IN".into()));
        assert_eq!(nearest_jack(&bay, [500.0, 500.0], SNAP, |_| true), None);
    }

    #[test]
    fn cable_sags_and_is_hit_near_its_curve() {
        let c = cable_curve([0.0, 0.0], [200.0, 0.0]);
        let mid = bezier(c, 0.5);
        assert!(mid[1] > 10.0, "sag {mid:?}");
        assert!(cable_distance(c, mid) < 0.5);
        assert!(cable_distance(c, [100.0, -40.0]) > 30.0);
        assert_eq!(bezier(c, 0.0), [0.0, 0.0]);
        assert_eq!(bezier(c, 1.0), [200.0, 0.0]);
    }

    #[test]
    fn finds_cable_and_title() {
        let mut bay = PatchBay::default();
        let a = bay.add(Kind::Radio, "R", [0.0, 0.0]);
        let b = bay.add(Kind::Antenna, "Ant", [400.0, 0.0]);
        bay.connect(&Jack::new(a, "ANT1"), &Jack::new(b, "FEED")).unwrap();
        let (p, q) = (jack_pos(&bay, &bay.cables[0].from).unwrap(), jack_pos(&bay, &bay.cables[0].to).unwrap());
        let mid = bezier(cable_curve(p, q), 0.5);
        assert_eq!(cable_at(&bay, mid, 6.0), Some(0));
        assert_eq!(title_at(&bay, [410.0, 5.0]), Some(b));
        assert_eq!(title_at(&bay, [410.0, 40.0]), None);
        assert_eq!(box_at(&bay, [410.0, 40.0]), Some(b));
    }
}

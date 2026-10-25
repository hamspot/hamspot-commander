//! Smith chart maths: complex numbers, impedance <-> reflection coefficient, line rotation,
//! grid circles clipped to the unit disk and single-stub matching. No egui, unit tested.
use std::f64::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Cx {
    pub re: f64,
    pub im: f64,
}

pub const fn cx(re: f64, im: f64) -> Cx {
    Cx { re, im }
}

impl Cx {
    pub fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }
    pub fn arg(self) -> f64 {
        self.im.atan2(self.re)
    }
    pub fn polar(r: f64, a: f64) -> Cx {
        cx(r * a.cos(), r * a.sin())
    }
    pub fn add(self, o: Cx) -> Cx {
        cx(self.re + o.re, self.im + o.im)
    }
    pub fn sub(self, o: Cx) -> Cx {
        cx(self.re - o.re, self.im - o.im)
    }
    pub fn mul(self, o: Cx) -> Cx {
        cx(
            self.re * o.re - self.im * o.im,
            self.re * o.im + self.im * o.re,
        )
    }
    pub fn div(self, o: Cx) -> Cx {
        let d = o.re * o.re + o.im * o.im;
        cx(
            (self.re * o.re + self.im * o.im) / d,
            (self.im * o.re - self.re * o.im) / d,
        )
    }
    pub fn scale(self, k: f64) -> Cx {
        cx(self.re * k, self.im * k)
    }
    pub fn inv(self) -> Cx {
        cx(1.0, 0.0).div(self)
    }
}

const ONE: Cx = cx(1.0, 0.0);

/// Reflection coefficient of a normalized impedance: (z - 1) / (z + 1).
pub fn gamma(z: Cx) -> Cx {
    z.sub(ONE).div(z.add(ONE))
}

/// Normalized impedance at a point of the chart: (1 + g) / (1 - g). Infinite at g = 1.
pub fn z_of(g: Cx) -> Cx {
    ONE.add(g).div(ONE.sub(g))
}

/// Moves `g` toward the generator by `wavelengths` of lossless line (clockwise on the chart).
pub fn toward_generator(g: Cx, wavelengths: f64) -> Cx {
    g.mul(Cx::polar(1.0, -2.0 * TAU * wavelengths))
}

/// (Test oracle) Input impedance of a lossless line by the textbook formula, in ohms:
/// Z0 (ZL + j Z0 tan bl) / (Z0 + j ZL tan bl).
#[cfg(test)]
pub fn zin_formula(zl: Cx, z0: f64, wavelengths: f64) -> Cx {
    let t = (TAU * wavelengths).tan();
    let j_z0t = cx(0.0, z0 * t);
    zl.add(j_z0t)
        .div(cx(z0, 0.0).add(zl.mul(cx(0.0, t))))
        .scale(z0)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LenUnit {
    Wavelengths,
    Degrees,
    Meters,
}

/// Electrical length in wavelengths. Meters needs the frequency (MHz) and velocity factor.
pub fn to_wavelengths(v: f64, unit: LenUnit, f_mhz: f64, vf: f64) -> f64 {
    match unit {
        LenUnit::Wavelengths => v,
        LenUnit::Degrees => v / 360.0,
        LenUnit::Meters => v / (299.792458 / f_mhz * vf),
    }
}

/// A circle (centre, radius) in the Gamma plane, sampled and clipped to the unit disk.
/// Returns the visible runs as polylines.
pub fn clipped_circle(c: Cx, r: f64, n: usize) -> Vec<Vec<Cx>> {
    let mut runs: Vec<Vec<Cx>> = vec![];
    let mut cur: Vec<Cx> = vec![];
    for i in 0..=n {
        let p = c.add(Cx::polar(r, TAU * i as f64 / n as f64));
        if p.abs() <= 1.0 + 1e-9 {
            cur.push(p);
        } else if !cur.is_empty() {
            runs.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        runs.push(cur);
    }
    // join the run that wraps through angle 0
    if runs.len() > 1 {
        let first = runs[0][0];
        let last = *runs.last().unwrap().last().unwrap();
        if (first.sub(last)).abs() < 1e-9 {
            let head = runs.remove(0);
            runs.last_mut().unwrap().extend(head);
        }
    }
    runs
}

/// Constant-resistance circle: centre r/(1+r), radius 1/(1+r).
pub fn r_circle(r: f64) -> (Cx, f64) {
    (cx(r / (1.0 + r), 0.0), 1.0 / (1.0 + r))
}

/// Constant-reactance circle: centre (1, 1/x), radius 1/|x|.
pub fn x_circle(x: f64) -> (Cx, f64) {
    (cx(1.0, 1.0 / x), 1.0 / x.abs())
}

/// Grid values drawn on the chart.
pub const GRID_R: [f64; 6] = [0.0, 0.2, 0.5, 1.0, 2.0, 5.0];
pub const GRID_X: [f64; 5] = [0.2, 0.5, 1.0, 2.0, 5.0];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Stub {
    Short,
    Open,
}

/// Single shunt-stub match.
#[derive(Clone, Copy, Debug)]
pub struct StubMatch {
    /// Line from the load to the stub, wavelengths.
    pub d: f64,
    /// Normalized admittance at the stub point, 1 + jb.
    pub y_at_d: Cx,
    /// Stub length, wavelengths.
    pub stub_len: f64,
}

/// Finds the shortest distance toward the generator where the normalized admittance has
/// real part 1, then the stub length that cancels its susceptance. `None` for a matched or
/// purely reactive load.
pub fn stub_match(zl_norm: Cx, kind: Stub) -> Option<StubMatch> {
    let g0 = gamma(zl_norm);
    let m = g0.abs();
    if m < 1e-6 || m > 1.0 - 1e-9 {
        return None;
    }
    let g_re = |d: f64| z_of(toward_generator(g0, d)).inv().re - 1.0;
    // scan half a wavelength for a sign change, then bisect
    let n = 2000;
    let mut prev = g_re(0.0);
    if prev.abs() < 1e-12 {
        return finish(g0, 0.0, kind);
    }
    for i in 1..=n {
        let d1 = 0.5 * i as f64 / n as f64;
        let v = g_re(d1);
        if v.signum() != prev.signum() {
            let (mut a, mut b) = (0.5 * (i - 1) as f64 / n as f64, d1);
            for _ in 0..60 {
                let mid = (a + b) / 2.0;
                if g_re(mid).signum() == g_re(a).signum() {
                    a = mid
                } else {
                    b = mid
                }
            }
            return finish(g0, (a + b) / 2.0, kind);
        }
        prev = v;
    }
    None
}

fn finish(g0: Cx, d: f64, kind: Stub) -> Option<StubMatch> {
    let y = z_of(toward_generator(g0, d)).inv();
    let b = y.im;
    // the stub must add -jb
    let bl = match kind {
        Stub::Short => (1.0f64).atan2(b), // y = -j cot(bl) = -jb  -> cot(bl) = b
        Stub::Open => (-b).atan().rem_euclid(PI), // y = j tan(bl) = -jb
    };
    Some(StubMatch {
        d,
        y_at_d: y,
        stub_len: bl / TAU,
    })
}

/// Input admittance of a stub of `len` wavelengths (normalized).
#[cfg(test)]
pub fn stub_admittance(len: f64, kind: Stub) -> Cx {
    let bl = TAU * len;
    match kind {
        Stub::Short => cx(0.0, -1.0 / bl.tan()),
        Stub::Open => cx(0.0, bl.tan()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "{a} vs {b}");
    }
    fn close_c(a: Cx, b: Cx, tol: f64) {
        assert!(a.sub(b).abs() <= tol, "{a:?} vs {b:?}");
    }

    #[test]
    fn complex_ops() {
        close_c(cx(1.0, 2.0).mul(cx(3.0, -1.0)), cx(5.0, 5.0), 1e-12);
        close_c(cx(5.0, 5.0).div(cx(3.0, -1.0)), cx(1.0, 2.0), 1e-12);
        close_c(cx(0.0, 1.0).inv(), cx(0.0, -1.0), 1e-12);
        close(cx(3.0, 4.0).abs(), 5.0, 1e-12);
    }

    #[test]
    fn gamma_known_values() {
        close_c(gamma(cx(1.0, 0.0)), cx(0.0, 0.0), 1e-12); // matched: centre
        close_c(gamma(cx(0.0, 0.0)), cx(-1.0, 0.0), 1e-12); // short: left edge
        close_c(gamma(cx(2.0, 0.0)), cx(1.0 / 3.0, 0.0), 1e-12); // 100 ohm on 50: SWR 2
        close_c(gamma(cx(0.0, 1.0)), cx(0.0, 1.0), 1e-12); // +j1: top of the chart
        // 100 + j50 on 50 ohm: z = 2 + j1, gamma = (1 + j)/(3 + j) = 0.4 + j0.2
        let g = gamma(cx(2.0, 1.0));
        close_c(g, cx(0.4, 0.2), 1e-12);
        close(g.abs(), 0.44721, 1e-5);
        close(g.arg().to_degrees(), 26.565, 1e-3);
        // round trip
        close_c(z_of(g), cx(2.0, 1.0), 1e-12);
    }

    #[test]
    fn line_rotation_matches_formula() {
        let z0 = 50.0;
        let zl = cx(100.0, 50.0);
        for d in [0.0, 0.05, 0.125, 0.2, 0.3, 0.37] {
            let by_chart = z_of(toward_generator(gamma(zl.scale(1.0 / z0)), d)).scale(z0);
            close_c(by_chart, zin_formula(zl, z0, d), 1e-9);
        }
        // quarter-wave transformer: Zin = Z0^2 / ZL
        let zin = z_of(toward_generator(gamma(cx(2.0, 0.0)), 0.25));
        close_c(zin, cx(0.5, 0.0), 1e-12);
        // half wave repeats
        close_c(toward_generator(cx(0.3, 0.1), 0.5), cx(0.3, 0.1), 1e-12);
        // toward the generator is clockwise: +j1 moves to the right half's lower angles
        let g = toward_generator(cx(0.0, 1.0), 0.0625);
        close(g.arg().to_degrees(), 45.0, 1e-9);
    }

    #[test]
    fn length_units() {
        close(
            to_wavelengths(90.0, LenUnit::Degrees, 0.0, 1.0),
            0.25,
            1e-12,
        );
        // 1 m at 149.896 MHz in air is half a wavelength; with VF 0.66 more
        close(
            to_wavelengths(1.0, LenUnit::Meters, 149.896229, 1.0),
            0.5,
            1e-9,
        );
        close(
            to_wavelengths(1.0, LenUnit::Meters, 149.896229, 0.66),
            0.5 / 0.66,
            1e-9,
        );
    }

    #[test]
    fn grid_circles() {
        // r circles pass through gamma = 1 and through (r-1)/(r+1)
        for r in GRID_R {
            let (c, rad) = r_circle(r);
            close(c.re + rad, 1.0, 1e-12);
            close(c.re - rad, (r - 1.0) / (r + 1.0), 1e-12);
        }
        // x circles pass through gamma = 1 and the point of z = jx on the rim
        for x in GRID_X {
            for x in [x, -x] {
                let (c, rad) = x_circle(x);
                close(c.sub(cx(1.0, 0.0)).abs(), rad, 1e-12);
                close(c.sub(gamma(cx(0.0, x))).abs(), rad, 1e-9);
            }
        }
        // clipping: everything visible is inside the disk, the r=0 rim is one closed run
        let runs = clipped_circle(x_circle(1.0).0, 1.0, 360);
        assert!(!runs.is_empty());
        assert!(runs.iter().flatten().all(|p| p.abs() <= 1.0 + 1e-9));
        let rim = clipped_circle(cx(0.0, 0.0), 1.0, 360);
        assert_eq!(rim.len(), 1);
        // points on a constant-r circle all have that resistance
        let (c, rad) = r_circle(0.5);
        for k in 1..8 {
            let p = c.add(Cx::polar(rad, k as f64));
            close(z_of(p).re, 0.5, 1e-9);
        }
    }

    #[test]
    fn single_stub_match() {
        // Classic textbook case: ZL = 100 + j50 on 50 ohm (z = 2 + j1).
        for kind in [Stub::Short, Stub::Open] {
            let m = stub_match(cx(2.0, 1.0), kind).unwrap();
            close(m.y_at_d.re, 1.0, 1e-9);
            assert!(m.d > 0.0 && m.d < 0.5);
            assert!(m.stub_len >= 0.0 && m.stub_len < 0.5);
            // stub plus line admittance is a perfect match
            let total = m.y_at_d.add(stub_admittance(m.stub_len, kind));
            close_c(total, cx(1.0, 0.0), 1e-6);
        }
        // known value: first solution d = 0.0322 wavelength, y = 1 - j1.0 ... check |b|
        let m = stub_match(cx(2.0, 1.0), Stub::Short).unwrap();
        close(m.y_at_d.im.abs(), 1.0, 1e-6); // |b| = (z-1 radius) for g=1 crossing at SWR 2.62
        assert!(
            stub_match(cx(1.0, 0.0), Stub::Short).is_none(),
            "already matched"
        );
        assert!(
            stub_match(cx(0.0, 1.0), Stub::Short).is_none(),
            "pure reactance"
        );
    }
}

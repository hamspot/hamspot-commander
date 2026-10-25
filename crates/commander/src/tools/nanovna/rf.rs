//! Host math for a NanoVNA V2 sweep. The device reports S11 and S21.
//! S12 and S22 are not measured.

use std::f64::consts::PI;

const C_MPS: f64 = 299_792_458.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cx {
    pub re: f64,
    pub im: f64,
}

impl Cx {
    pub const ZERO: Self = Self { re: 0.0, im: 0.0 };

    pub fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }

    pub fn norm(self) -> f64 {
        self.re * self.re + self.im * self.im
    }

    pub fn abs(self) -> f64 {
        self.norm().sqrt()
    }

    pub fn arg(self) -> f64 {
        self.im.atan2(self.re)
    }

    pub fn add(self, other: Self) -> Self {
        Self {
            re: self.re + other.re,
            im: self.im + other.im,
        }
    }

    pub fn sub(self, other: Self) -> Self {
        Self {
            re: self.re - other.re,
            im: self.im - other.im,
        }
    }

    pub fn mul(self, other: Self) -> Self {
        Self {
            re: self.re * other.re - self.im * other.im,
            im: self.re * other.im + self.im * other.re,
        }
    }

    pub fn scale(self, k: f64) -> Self {
        Self {
            re: self.re * k,
            im: self.im * k,
        }
    }

    pub fn div(self, other: Self) -> Option<Self> {
        let den = other.norm();
        if den == 0.0 {
            return None;
        }
        Some(Self {
            re: (self.re * other.re + self.im * other.im) / den,
            im: (self.im * other.re - self.re * other.im) / den,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub hz: f64,
    pub s11: Cx,
    pub s21: Cx,
}

pub fn from_ratio(hz: f64, s11_re: f64, s11_im: f64, s21_re: f64, s21_im: f64) -> Sample {
    Sample {
        hz,
        s11: Cx::new(s11_re, s11_im),
        s21: Cx::new(s21_re, s21_im),
    }
}

/// Γ = 0 is Z0. A magnitude of 1 or more is left as an open reading.
pub fn impedance(gamma: Cx, z0: f64) -> Option<Cx> {
    let den = Cx::new(1.0 - gamma.re, -gamma.im);
    let num = Cx::new(1.0 + gamma.re, gamma.im);
    num.div(den).map(|z| z.scale(z0))
}

pub fn swr(gamma: Cx) -> f64 {
    let mag = gamma.abs();
    if mag >= 1.0 {
        f64::INFINITY
    } else {
        (1.0 + mag) / (1.0 - mag)
    }
}

pub fn return_loss_db(gamma: Cx) -> f64 {
    let mag = gamma.abs();
    if mag <= 0.0 {
        f64::INFINITY
    } else {
        -20.0 * mag.log10()
    }
}

pub fn db(value: Cx) -> f64 {
    let mag = value.abs();
    if mag <= 0.0 {
        f64::NEG_INFINITY
    } else {
        20.0 * mag.log10()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Derived {
    pub z_abs: f64,
    pub r: f64,
    pub x: f64,
    pub q: f64,
    pub series_l_h: f64,
    pub series_c_f: f64,
    pub parallel_l_h: f64,
    pub parallel_c_f: f64,
    pub parallel_r: f64,
    pub phase_deg: f64,
}

pub fn derive(sample: &Sample, z0: f64) -> Derived {
    let z = impedance(sample.s11, z0).unwrap_or(Cx::new(f64::INFINITY, f64::INFINITY));
    let omega = 2.0 * PI * sample.hz;
    let r = z.re;
    let x = z.im;
    let q = if r.abs() < 1e-15 {
        f64::INFINITY
    } else {
        x.abs() / r.abs()
    };
    let (series_l_h, series_c_f) = if !x.is_finite() || omega == 0.0 {
        (f64::NAN, f64::NAN)
    } else if x >= 0.0 {
        (x / omega, f64::NAN)
    } else {
        (f64::NAN, -1.0 / (omega * x))
    };
    let rpx = r * r + x * x;
    let rp = if r.abs() < 1e-15 {
        f64::INFINITY
    } else {
        rpx / r
    };
    let xp = if x.abs() < 1e-15 { f64::INFINITY } else { rpx / x };
    let (parallel_l_h, parallel_c_f) = if !xp.is_finite() || omega == 0.0 {
        (f64::NAN, f64::NAN)
    } else if xp >= 0.0 {
        (xp / omega, f64::NAN)
    } else {
        (f64::NAN, -1.0 / (omega * xp))
    };
    Derived {
        z_abs: if r.is_finite() && x.is_finite() {
            r.hypot(x)
        } else {
            f64::INFINITY
        },
        r,
        x,
        q,
        series_l_h,
        series_c_f,
        parallel_l_h,
        parallel_c_f,
        parallel_r: rp,
        phase_deg: sample.s11.arg().to_degrees(),
    }
}

/// Boxcar over `width` points. Width 1 leaves the sweep alone.
pub fn smooth_samples(samples: &[Sample], width: u32) -> Vec<Sample> {
    if width <= 1 || samples.len() < 2 {
        return samples.to_vec();
    }
    let half = ((width as usize) - 1) / 2;
    samples
        .iter()
        .enumerate()
        .map(|(index, sample)| {
            let start = index.saturating_sub(half);
            let end = (index + half + 1).min(samples.len());
            let mut s11 = Cx::ZERO;
            let mut s21 = Cx::ZERO;
            let mut count = 0.0;
            for other in &samples[start..end] {
                s11 = s11.add(other.s11);
                s21 = s21.add(other.s21);
                count += 1.0;
            }
            Sample {
                hz: sample.hz,
                s11: s11.scale(1.0 / count),
                s21: s21.scale(1.0 / count),
            }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct ErrorTerms {
    pub directivity: Vec<Cx>,
    pub source_match: Vec<Cx>,
    pub reflection_tracking: Vec<Cx>,
}

/// Ideal open, short, and load. A standard that did not divide is left out
/// of that frequency and the point stays uncalibrated.
pub fn solve_osl(open: &[Cx], short: &[Cx], load: &[Cx]) -> Result<ErrorTerms, String> {
    if open.len() != short.len() || short.len() != load.len() || open.is_empty() {
        return Err("open, short, and load must be the same non-empty sweep".into());
    }
    let mut directivity = Vec::with_capacity(open.len());
    let mut source_match = Vec::with_capacity(open.len());
    let mut reflection_tracking = Vec::with_capacity(open.len());
    for index in 0..open.len() {
        let ed = load[index];
        let a = open[index].sub(ed);
        let b = ed.sub(short[index]);
        let sum = a.add(b);
        let diff = a.sub(b);
        let Some(es) = diff.div(sum) else {
            directivity.push(Cx::ZERO);
            source_match.push(Cx::ZERO);
            reflection_tracking.push(Cx::ZERO);
            continue;
        };
        let er = a.mul(Cx::new(1.0, 0.0).sub(es));
        directivity.push(ed);
        source_match.push(es);
        reflection_tracking.push(er);
    }
    Ok(ErrorTerms {
        directivity,
        source_match,
        reflection_tracking,
    })
}

pub fn apply_osl(measured: Cx, terms: &ErrorTerms, index: usize) -> Option<Cx> {
    let ed = terms.directivity.get(index).copied()?;
    let es = terms.source_match.get(index).copied()?;
    let er = terms.reflection_tracking.get(index).copied()?;
    if er.norm() == 0.0 && es.norm() == 0.0 && ed.norm() == 0.0 {
        return None;
    }
    let num = measured.sub(ed);
    let den = er.add(es.mul(num));
    num.div(den)
}

pub fn apply_thru(measured: Cx, thru: Cx, isolation: Option<Cx>) -> Option<Cx> {
    let (meas, track) = if let Some(iso) = isolation {
        (measured.sub(iso), thru.sub(iso))
    } else {
        (measured, thru)
    };
    meas.div(track)
}

/// Remove a length of line. Reflection is a round trip, transmission is one way.
pub fn port_extend(sample: Sample, metres: f64, velocity: f64) -> Sample {
    if metres == 0.0 || !(velocity > 0.0) || !sample.hz.is_finite() {
        return sample;
    }
    let beta = 2.0 * PI * sample.hz * metres / (C_MPS * velocity);
    let rot11 = Cx::new((2.0 * beta).cos(), (2.0 * beta).sin());
    let rot21 = Cx::new(beta.cos(), beta.sin());
    Sample {
        hz: sample.hz,
        s11: sample.s11.mul(rot11),
        s21: sample.s21.mul(rot21),
    }
}

pub fn unwrap_phase(values: &[Cx]) -> Vec<f64> {
    let mut out = Vec::with_capacity(values.len());
    let mut prev = 0.0;
    for (index, value) in values.iter().enumerate() {
        let angle = value.arg();
        if index == 0 {
            prev = angle;
            out.push(angle);
            continue;
        }
        let mut delta = angle - prev;
        while delta > PI {
            delta -= 2.0 * PI;
        }
        while delta < -PI {
            delta += 2.0 * PI;
        }
        let next = out[index - 1] + delta;
        // Keep `prev` as the raw angle so the wrap is against the principal value.
        prev = angle;
        out.push(next);
    }
    out
}

/// Group delay from the slope of unwrapped phase, averaged over up to two
/// points on each side. τ = −Δφ / (2π Δf), with φ in radians.
pub fn group_delay(hz: &[f64], unwrapped: &[f64]) -> Vec<f64> {
    let n = hz.len().min(unwrapped.len());
    let mut out = vec![f64::NAN; n];
    for index in 0..n {
        let lo = index.saturating_sub(2);
        let hi = (index + 2).min(n - 1);
        if hi <= lo || !(hz[hi] > hz[lo]) {
            continue;
        }
        let slope = (unwrapped[hi] - unwrapped[lo]) / (hz[hi] - hz[lo]);
        out[index] = -slope / (2.0 * PI);
    }
    out
}

pub fn mean_complex(rows: &[Vec<Cx>]) -> Vec<Cx> {
    if rows.is_empty() {
        return Vec::new();
    }
    let n = rows[0].len();
    let mut out = vec![Cx::ZERO; n];
    let mut count = vec![0u32; n];
    for row in rows {
        for (index, value) in row.iter().enumerate().take(n) {
            if value.re.is_finite() && value.im.is_finite() {
                out[index] = out[index].add(*value);
                count[index] += 1;
            }
        }
    }
    for (value, seen) in out.iter_mut().zip(count) {
        if seen > 0 {
            *value = value.scale(1.0 / f64::from(seen));
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct Resonance {
    pub kind: &'static str,
    pub hz: f64,
    pub bw3_hz: f64,
    pub bw6_hz: f64,
    pub q: f64,
}

/// Series resonance is X going from capacitive to inductive.
/// Parallel resonance is the other crossing.
pub fn x_resonances(samples: &[Sample], z0: f64) -> Vec<Resonance> {
    let mut out = Vec::new();
    let xs: Vec<(f64, f64)> = samples
        .iter()
        .map(|sample| (sample.hz, derive(sample, z0).x))
        .collect();
    for index in 1..xs.len() {
        let (f0, x0) = xs[index - 1];
        let (f1, x1) = xs[index];
        if !x0.is_finite() || !x1.is_finite() || x0 == 0.0 || x0.signum() == x1.signum() {
            continue;
        }
        let frac = x0.abs() / (x0.abs() + x1.abs());
        let hz = f0 + (f1 - f0) * frac;
        let kind = if x0 < 0.0 && x1 > 0.0 {
            "series"
        } else {
            "parallel"
        };
        out.push(Resonance {
            kind,
            hz,
            bw3_hz: f64::NAN,
            bw6_hz: f64::NAN,
            q: f64::NAN,
        });
    }
    out
}

/// Peak of |S21|, with 3 dB and 6 dB bandwidth around that peak.
pub fn s21_peak(samples: &[Sample]) -> Option<Resonance> {
    if samples.len() < 3 {
        return None;
    }
    let mut best = 0usize;
    let mut best_db = f64::NEG_INFINITY;
    for (index, sample) in samples.iter().enumerate() {
        let mag = db(sample.s21);
        if mag > best_db {
            best_db = mag;
            best = index;
        }
    }
    if !best_db.is_finite() {
        return None;
    }
    let bw3 = bandwidth(samples, best, best_db - 3.0);
    let bw6 = bandwidth(samples, best, best_db - 6.0);
    let q = if bw3 > 0.0 {
        samples[best].hz / bw3
    } else {
        f64::NAN
    };
    Some(Resonance {
        kind: "s21 peak",
        hz: samples[best].hz,
        bw3_hz: bw3,
        bw6_hz: bw6,
        q,
    })
}

fn bandwidth(samples: &[Sample], peak: usize, floor_db: f64) -> f64 {
    let mut lo = peak;
    while lo > 0 && db(samples[lo].s21) >= floor_db {
        lo -= 1;
    }
    let mut hi = peak;
    while hi + 1 < samples.len() && db(samples[hi].s21) >= floor_db {
        hi += 1;
    }
    if lo == peak || hi == peak {
        return f64::NAN;
    }
    let left = cross(samples, lo, lo + 1, floor_db);
    let right = cross(samples, hi - 1, hi, floor_db);
    (right - left).abs()
}

fn cross(samples: &[Sample], a: usize, b: usize, floor_db: f64) -> f64 {
    let ya = db(samples[a].s21);
    let yb = db(samples[b].s21);
    let span = yb - ya;
    if span.abs() < 1e-12 {
        return samples[a].hz;
    }
    let frac = ((floor_db - ya) / span).clamp(0.0, 1.0);
    samples[a].hz + (samples[b].hz - samples[a].hz) * frac
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fault {
    pub metres: f64,
    pub magnitude: f64,
}

/// Window, zero-pad, and inverse FFT. The range limit is `c·vf / (2 Δf)`.
pub fn distance_to_fault(samples: &[Sample], velocity: f64) -> Result<(f64, Vec<Fault>), String> {
    if samples.len() < 4 {
        return Err("a distance trace needs at least 4 points".into());
    }
    if !(velocity > 0.0) || velocity > 1.0 {
        return Err("velocity factor must be between 0 and 1".into());
    }
    let df = samples[1].hz - samples[0].hz;
    if !(df > 0.0) {
        return Err("the sweep step is not positive".into());
    }
    for pair in samples.windows(2) {
        if (pair[1].hz - pair[0].hz - df).abs() > df * 1e-6 + 1.0 {
            return Err("distance to fault needs a uniform frequency step".into());
        }
    }
    let mut n = 1usize;
    while n < samples.len() * 2 {
        n *= 2;
    }
    let mut buf = vec![Cx::ZERO; n];
    for (index, sample) in samples.iter().enumerate() {
        let window = 0.5 - 0.5 * (2.0 * PI * index as f64 / (samples.len() as f64 - 1.0)).cos();
        buf[index] = sample.s11.scale(window);
    }
    ifft(&mut buf);
    let limit = C_MPS * velocity / (2.0 * df);
    let mut trace = Vec::with_capacity(n);
    for (index, value) in buf.iter().enumerate() {
        let time = index as f64 / (n as f64 * df);
        let metres = C_MPS * velocity * time / 2.0;
        if metres > limit {
            break;
        }
        trace.push(Fault {
            metres,
            magnitude: value.abs(),
        });
    }
    Ok((limit, trace))
}

fn fft(data: &mut [Cx]) {
    let n = data.len();
    if n <= 1 {
        return;
    }
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            data.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * PI / len as f64;
        let wlen = Cx::new(ang.cos(), ang.sin());
        let mut start = 0;
        while start < n {
            let mut w = Cx::new(1.0, 0.0);
            for offset in 0..len / 2 {
                let u = data[start + offset];
                let v = data[start + offset + len / 2].mul(w);
                data[start + offset] = u.add(v);
                data[start + offset + len / 2] = u.sub(v);
                w = w.mul(wlen);
            }
            start += len;
        }
        len *= 2;
    }
}

fn ifft(data: &mut [Cx]) {
    for value in data.iter_mut() {
        value.im = -value.im;
    }
    fft(data);
    let scale = 1.0 / data.len() as f64;
    for value in data.iter_mut() {
        value.im = -value.im;
        *value = value.scale(scale);
    }
}

pub fn touchstone_s1p(samples: &[Sample], z0: f64) -> String {
    let mut out = String::from("! S11 only. S21 is not in an s1p file.\n");
    out.push_str(&format!("# Hz S RI R {z0}\n"));
    for sample in samples {
        out.push_str(&format!(
            "{:.0} {:.8e} {:.8e}\n",
            sample.hz, sample.s11.re, sample.s11.im
        ));
    }
    out
}

pub fn touchstone_s2p(samples: &[Sample], z0: f64) -> String {
    let mut out = String::from(
        "! S12 and S22 were not measured. Flip the part by hand and say so on that file.\n",
    );
    out.push_str(&format!("# Hz S RI R {z0}\n"));
    for sample in samples {
        out.push_str(&format!(
            "{:.0} {:.8e} {:.8e} {:.8e} {:.8e} 0 0 0 0\n",
            sample.hz, sample.s11.re, sample.s11.im, sample.s21.re, sample.s21.im
        ));
    }
    out
}

pub fn csv_table(samples: &[Sample], z0: f64, calibrated: bool) -> String {
    let phases = unwrap_phase(&samples.iter().map(|s| s.s21).collect::<Vec<_>>());
    let hz: Vec<f64> = samples.iter().map(|s| s.hz).collect();
    let delay = group_delay(&hz, &phases);
    let mut out = String::from(
        "hz,s11_re,s11_im,s21_re,s21_im,swr,return_loss_db,z_abs,r,x,q,series_l_h,series_c_f,parallel_l_h,parallel_c_f,s21_db,s21_phase_deg,s21_phase_unwrap_deg,group_delay_s,calibrated\n",
    );
    for (index, sample) in samples.iter().enumerate() {
        let d = derive(sample, z0);
        let cell = |value: f64| {
            if value.is_finite() {
                format!("{value:.8e}")
            } else {
                String::new()
            }
        };
        out.push_str(&format!(
            "{:.0},{:.8e},{:.8e},{:.8e},{:.8e},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            sample.hz,
            sample.s11.re,
            sample.s11.im,
            sample.s21.re,
            sample.s21.im,
            cell(swr(sample.s11)),
            cell(return_loss_db(sample.s11)),
            cell(d.z_abs),
            cell(d.r),
            cell(d.x),
            cell(d.q),
            cell(d.series_l_h),
            cell(d.series_c_f),
            cell(d.parallel_l_h),
            cell(d.parallel_c_f),
            cell(db(sample.s21)),
            cell(sample.s21.arg().to_degrees()),
            cell(phases.get(index).copied().unwrap_or(f64::NAN).to_degrees()),
            cell(delay.get(index).copied().unwrap_or(f64::NAN)),
            if calibrated { "yes" } else { "no" },
        ));
    }
    out
}

/// A dark chart. The first series is tan. This is the exported picture, not a window grab.
pub fn chart_png_traces(title: &str, xs: &[f64], traces: &[&[f64]]) -> Result<Vec<u8>, String> {
    if traces.is_empty() || traces.iter().any(|ys| ys.len() != xs.len()) || xs.is_empty() {
        return Err("the chart has no points".into());
    }
    let w = 900u32;
    let h = 520u32;
    let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([12, 16, 22]));
    let left = 70.0;
    let right = w as f64 - 20.0;
    let top = 36.0;
    let bottom = h as f64 - 40.0;
    let xmin = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let xmax = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let ymin = traces
        .iter()
        .flat_map(|ys| ys.iter())
        .copied()
        .filter(|v| v.is_finite())
        .fold(f64::INFINITY, f64::min);
    let ymax = traces
        .iter()
        .flat_map(|ys| ys.iter())
        .copied()
        .filter(|v| v.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);
    if !xmin.is_finite() || !ymin.is_finite() || xmax <= xmin || ymax <= ymin {
        return Err("the chart axes are empty".into());
    }
    let map = |x: f64, y: f64| -> (i32, i32) {
        let px = left + (x - xmin) / (xmax - xmin) * (right - left);
        let py = bottom - (y - ymin) / (ymax - ymin) * (bottom - top);
        (px.round() as i32, py.round() as i32)
    };
    for x in [left as i32, right as i32] {
        vline(&mut img, x, top as i32, bottom as i32, image::Rgb([90, 98, 110]));
    }
    hline(
        &mut img,
        left as i32,
        right as i32,
        bottom as i32,
        image::Rgb([90, 98, 110]),
    );
    let colors = [
        image::Rgb([214, 176, 112]),
        image::Rgb([120, 170, 210]),
        image::Rgb([180, 90, 70]),
    ];
    for (index, ys) in traces.iter().enumerate() {
        let color = colors[index % colors.len()];
        let mut prev: Option<(i32, i32)> = None;
        for (x, y) in xs.iter().zip(*ys) {
            if !y.is_finite() {
                prev = None;
                continue;
            }
            let pt = map(*x, *y);
            if let Some(was) = prev {
                line(&mut img, was, pt, color);
            }
            prev = Some(pt);
        }
    }
    let _ = title;
    let mut bytes = Vec::new();
    let dyn_img = image::DynamicImage::ImageRgb8(img);
    dyn_img
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .map_err(|err| err.to_string())?;
    Ok(bytes)
}

fn hline(img: &mut image::RgbImage, x0: i32, x1: i32, y: i32, color: image::Rgb<u8>) {
    for x in x0.min(x1)..=x0.max(x1) {
        put(img, x, y, color);
    }
}

fn vline(img: &mut image::RgbImage, x: i32, y0: i32, y1: i32, color: image::Rgb<u8>) {
    for y in y0.min(y1)..=y0.max(y1) {
        put(img, x, y, color);
    }
}

fn line(img: &mut image::RgbImage, a: (i32, i32), b: (i32, i32), color: image::Rgb<u8>) {
    let mut x0 = a.0;
    let mut y0 = a.1;
    let x1 = b.0;
    let y1 = b.1;
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        put(img, x0, y0, color);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

fn put(img: &mut image::RgbImage, x: i32, y: i32, color: image::Rgb<u8>) {
    if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
        img.put_pixel(x as u32, y as u32, color);
    }
}

/// The official capture example reads the 16-bit pixels as big-endian RGB565.
pub fn rgb565_png(width: u16, height: u16, pixels: &[u8]) -> Result<Vec<u8>, String> {
    let need = usize::from(width) * usize::from(height) * 2;
    if pixels.len() != need {
        return Err(format!("capture is {} bytes, expected {need}", pixels.len()));
    }
    let mut img = image::RgbImage::new(u32::from(width), u32::from(height));
    for (index, chunk) in pixels.chunks(2).enumerate() {
        let px = u16::from_be_bytes([chunk[0], chunk[1]]);
        let r = ((px >> 11) & 0x1f) as u8;
        let g = ((px >> 5) & 0x3f) as u8;
        let b = (px & 0x1f) as u8;
        let x = (index as u32) % u32::from(width);
        let y = (index as u32) / u32::from(width);
        img.put_pixel(
            x,
            y,
            image::Rgb([r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2]),
        );
    }
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .map_err(|err| err.to_string())?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gamma_box(g: Cx, ed: Cx, es: Cx, er: Cx) -> Cx {
        // gm = ed + er*g / (1 - es*g)
        let num = er.mul(g);
        let den = Cx::new(1.0, 0.0).sub(es.mul(g));
        ed.add(num.div(den).unwrap())
    }

    #[test]
    fn osl_recovers_a_known_load() {
        let ed = Cx::new(0.02, -0.01);
        let es = Cx::new(0.05, 0.02);
        let er = Cx::new(0.9, 0.1);
        let actual = Cx::new(0.3, -0.2);
        let open = gamma_box(Cx::new(1.0, 0.0), ed, es, er);
        let short = gamma_box(Cx::new(-1.0, 0.0), ed, es, er);
        let load = gamma_box(Cx::ZERO, ed, es, er);
        let terms = solve_osl(&[open], &[short], &[load]).unwrap();
        let measured = gamma_box(actual, ed, es, er);
        let back = apply_osl(measured, &terms, 0).unwrap();
        assert!((back.re - actual.re).abs() < 1e-9);
        assert!((back.im - actual.im).abs() < 1e-9);
    }

    #[test]
    fn a_fifty_ohm_load_is_about_one_to_one() {
        let sample = Sample {
            hz: 14e6,
            s11: Cx::new(0.0, 0.0),
            s21: Cx::ZERO,
        };
        let d = derive(&sample, 50.0);
        assert!((d.r - 50.0).abs() < 1e-9);
        assert!(d.x.abs() < 1e-9);
        assert!((swr(sample.s11) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn group_delay_of_a_fixed_line_is_that_delay() {
        let tau = 12e-9;
        let mut samples = Vec::new();
        let mut hz = Vec::new();
        let mut s21 = Vec::new();
        for i in 0..21 {
            let f = 100e6 + i as f64 * 1e6;
            let phase = -2.0 * PI * f * tau;
            let value = Cx::new(phase.cos(), phase.sin());
            hz.push(f);
            s21.push(value);
            samples.push(Sample {
                hz: f,
                s11: Cx::ZERO,
                s21: value,
            });
        }
        let delay = group_delay(&hz, &unwrap_phase(&s21));
        assert!((delay[10] - tau).abs() < 1e-12);
        let _ = samples;
    }

    #[test]
    fn distance_to_fault_peaks_near_the_programmed_length() {
        let vf = 1.0;
        let df = 1e6;
        let nfft = 128.0;
        // Bin 4 of the zero-padded length. A tone that is not on a bin
        // lands on a neighbour, and one bin here is more than a metre.
        let metres = 4.0 * C_MPS * vf / (2.0 * nfft * df);
        let tau = 2.0 * metres / (C_MPS * vf);
        let mut samples = Vec::new();
        for i in 0..64 {
            let f = 1e6 + i as f64 * df;
            let phase = -2.0 * PI * f * tau;
            samples.push(Sample {
                hz: f,
                s11: Cx::new(phase.cos(), phase.sin()),
                s21: Cx::ZERO,
            });
        }
        let (limit, trace) = distance_to_fault(&samples, vf).unwrap();
        let peak = trace
            .iter()
            .max_by(|a, b| a.magnitude.partial_cmp(&b.magnitude).unwrap())
            .unwrap();
        assert!((peak.metres - metres).abs() < 0.2, "{}", peak.metres);
        let expect = C_MPS * vf / (2.0 * df);
        assert!((limit - expect).abs() < 1e-6);
    }

    #[test]
    fn an_s2p_file_says_s12_and_s22_were_not_measured() {
        let text = touchstone_s2p(
            &[Sample {
                hz: 1e6,
                s11: Cx::new(0.1, 0.0),
                s21: Cx::new(0.9, 0.0),
            }],
            50.0,
        );
        assert!(text.contains("S12 and S22 were not measured"));
        assert!(text.contains("0 0 0 0"));
    }

    #[test]
    fn a_chart_png_starts_with_the_png_signature() {
        let ys = [1.2, 1.5, 1.1];
        let bytes = chart_png_traces("swr", &[1.0, 2.0, 3.0], &[&ys]).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }
}

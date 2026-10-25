//! Memory traces, complex trace math and Touchstone / CSV import. Issues 70
//! and 53. Nothing here touches egui.

use super::rf::{Cx, Sample};

/// Number of memory slots, MEM1..MEM4.
pub const MEMS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Math {
    Off,
    /// data − memory, complex, per point.
    Sub,
    /// data ÷ memory, complex, per point (normalize).
    Div,
}

/// Point-by-point complex math on S11 and S21. Memory must share the grid
/// (same length; frequencies within 1 Hz) or None is returned.
pub fn trace_math(data: &[Sample], memory: &[Sample], op: Math) -> Option<Vec<Sample>> {
    if op == Math::Off || data.len() != memory.len() || data.is_empty() {
        return None;
    }
    if data.iter().zip(memory).any(|(a, b)| (a.hz - b.hz).abs() > 1.0) {
        return None;
    }
    let nan = Cx::new(f64::NAN, f64::NAN);
    let f = |a: Cx, b: Cx| match op {
        Math::Sub => a.sub(b),
        Math::Div => a.div(b).unwrap_or(nan),
        Math::Off => a,
    };
    Some(
        data.iter()
            .zip(memory)
            .map(|(a, b)| Sample { hz: a.hz, s11: f(a.s11, b.s11), s21: f(a.s21, b.s21) })
            .collect(),
    )
}

/// A parsed Touchstone file: samples (S21 is zero for an s1p) and its Z0.
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    pub samples: Vec<Sample>,
    pub z0: f64,
    pub has_s21: bool,
}

/// Read a v1 Touchstone .s1p or .s2p: `# <Hz|kHz|MHz|GHz> S <RI|MA|DB> R <z0>`.
/// S12 and S22 in an s2p are ignored (the V2.2 does not measure them).
pub fn parse_touchstone(text: &str) -> Result<Imported, String> {
    let mut unit = 1e9; // Touchstone default is GHz.
    let mut fmt = "MA".to_string();
    let mut z0 = 50.0;
    let mut nums: Vec<f64> = Vec::new();
    let mut seen_option = false;
    for raw in text.lines() {
        let line = raw.split('!').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(opt) = line.strip_prefix('#') {
            if seen_option {
                continue;
            }
            seen_option = true;
            let words: Vec<String> = opt.split_whitespace().map(|w| w.to_ascii_uppercase()).collect();
            let mut i = 0;
            while i < words.len() {
                match words[i].as_str() {
                    "HZ" => unit = 1.0,
                    "KHZ" => unit = 1e3,
                    "MHZ" => unit = 1e6,
                    "GHZ" => unit = 1e9,
                    "RI" | "MA" | "DB" => fmt = words[i].clone(),
                    "S" => {}
                    "Y" | "Z" | "H" | "G" => return Err(format!("{} parameters are not supported, only S", words[i])),
                    "R" => {
                        let v = words.get(i + 1).and_then(|w| w.parse::<f64>().ok()).ok_or("R needs a value")?;
                        if !(v > 0.0) {
                            return Err("Z0 must be above zero".into());
                        }
                        z0 = v;
                        i += 1;
                    }
                    other => return Err(format!("unknown option word {other}")),
                }
                i += 1;
            }
            continue;
        }
        if line.starts_with('[') {
            return Err("Touchstone v2 keywords are not supported".into());
        }
        for w in line.split_whitespace() {
            nums.push(w.parse::<f64>().map_err(|_| format!("not a number: {w}"))?);
        }
    }
    let pair = |a: f64, b: f64| -> Cx {
        match fmt.as_str() {
            "RI" => Cx::new(a, b),
            "DB" => {
                let m = 10f64.powf(a / 20.0);
                let r = b.to_radians();
                Cx::new(m * r.cos(), m * r.sin())
            }
            _ => {
                let r = b.to_radians();
                Cx::new(a * r.cos(), a * r.sin())
            }
        }
    };
    // s1p rows hold 3 numbers, s2p rows 9. Decide by which divides cleanly
    // and keeps frequencies rising.
    let rising = |w: usize| nums.len() % w == 0 && nums.chunks(w).map(|c| c[0]).collect::<Vec<_>>().windows(2).all(|p| p[1] > p[0]);
    let width = if rising(9) && !rising(3) {
        9
    } else if rising(3) {
        3
    } else if rising(9) {
        9
    } else {
        return Err("rows are neither 3 (s1p) nor 9 (s2p) numbers with rising frequency".into());
    };
    let samples: Vec<Sample> = nums
        .chunks(width)
        .map(|c| Sample {
            hz: c[0] * unit,
            s11: pair(c[1], c[2]),
            s21: if width == 9 { pair(c[3], c[4]) } else { Cx::ZERO },
        })
        .collect();
    if samples.len() < 2 {
        return Err("a Touchstone file needs at least two points".into());
    }
    Ok(Imported { samples, z0, has_s21: width == 9 })
}

/// Move S11 from the file's reference impedance to `z0`. S21 is left as is.
pub fn renormalize(samples: &mut [Sample], from: f64, to: f64) {
    if (from - to).abs() < 1e-9 {
        return;
    }
    for s in samples {
        let one = Cx::new(1.0, 0.0);
        if let Some(z) = one.add(s.s11).div(one.sub(s.s11)).map(|z| z.scale(from)) {
            if let Some(g) = z.sub(Cx::new(to, 0.0)).div(z.add(Cx::new(to, 0.0))) {
                s.s11 = g;
            }
        }
    }
}

/// Read a CSV sweep. A header row names the columns. It needs a frequency
/// column (`hz`, `freq_hz`, `freq_mhz` or `mhz`) and either `s11_re`/`s11_im`,
/// `r`/`x` (ohms), or `swr` alone (stored as a real Γ, angle unknown).
/// Commander's own CSV export reads back exactly. Issue 53.
pub fn parse_csv(text: &str, z0: f64) -> Result<Imported, String> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
    let head: Vec<String> = lines
        .next()
        .ok_or("the CSV is empty")?
        .split(',')
        .map(|h| h.trim().trim_matches('"').to_ascii_lowercase())
        .collect();
    let col = |names: &[&str]| head.iter().position(|h| names.contains(&h.as_str()));
    let (fcol, fk) = if let Some(i) = col(&["hz", "freq_hz", "frequency_hz", "frequency"]) {
        (i, 1.0)
    } else if let Some(i) = col(&["mhz", "freq_mhz", "frequency_mhz"]) {
        (i, 1e6)
    } else {
        return Err("the CSV needs a hz or mhz column".into());
    };
    let re = col(&["s11_re"]);
    let im = col(&["s11_im"]);
    let r = col(&["r", "r_ohm"]);
    let x = col(&["x", "x_ohm"]);
    let swr = col(&["swr"]);
    let s21re = col(&["s21_re"]);
    let s21im = col(&["s21_im"]);
    if !(re.is_some() && im.is_some()) && !(r.is_some() && x.is_some()) && swr.is_none() {
        return Err("the CSV needs s11_re and s11_im, r and x, or swr".into());
    }
    let mut out = Vec::new();
    for (n, line) in lines.enumerate() {
        let cells: Vec<&str> = line.split(',').map(str::trim).collect();
        let get = |i: Option<usize>| -> Option<f64> { i.and_then(|i| cells.get(i)).and_then(|c| c.parse().ok()) };
        let hz = get(Some(fcol)).ok_or(format!("row {} has no frequency", n + 2))? * fk;
        let s11 = if let (Some(a), Some(b)) = (get(re), get(im)) {
            Cx::new(a, b)
        } else if let (Some(a), Some(b)) = (get(r), get(x)) {
            let z = Cx::new(a, b);
            z.sub(Cx::new(z0, 0.0)).div(z.add(Cx::new(z0, 0.0))).unwrap_or(Cx::new(1.0, 0.0))
        } else if let Some(s) = get(swr) {
            Cx::new(if s >= 1.0 { (s - 1.0) / (s + 1.0) } else { 0.0 }, 0.0)
        } else {
            return Err(format!("row {} has no S11 value", n + 2));
        };
        let s21 = match (get(s21re), get(s21im)) {
            (Some(a), Some(b)) => Cx::new(a, b),
            _ => Cx::ZERO,
        };
        out.push(Sample { hz, s11, s21 });
    }
    if out.len() < 2 {
        return Err("the CSV needs at least two rows".into());
    }
    let has_s21 = s21re.is_some() && s21im.is_some();
    Ok(Imported { samples: out, z0, has_s21 })
}

/// Linear resample of (xs, ys) onto `at`. Points outside the source range are NaN.
pub fn resample(xs: &[f64], ys: &[f64], at: &[f64]) -> Vec<f64> {
    at.iter()
        .map(|&x| {
            if xs.len() < 2 || ys.len() != xs.len() || x < xs[0] || x > xs[xs.len() - 1] {
                return f64::NAN;
            }
            let i = xs.partition_point(|v| *v < x).clamp(1, xs.len() - 1);
            let (x0, x1, y0, y1) = (xs[i - 1], xs[i], ys[i - 1], ys[i]);
            if x1 == x0 { y0 } else { y0 + (y1 - y0) * (x - x0) / (x1 - x0) }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::rf;
    use super::*;

    fn sweep(k: f64) -> Vec<Sample> {
        (0..5)
            .map(|i| Sample {
                hz: 1e6 + i as f64 * 1e6,
                s11: Cx::new(0.1 * k * i as f64, -0.05 * k),
                s21: Cx::new(0.5 * k, 0.1 * i as f64),
            })
            .collect()
    }

    #[test]
    fn math_trace_is_correct_for_test_data() {
        let d = sweep(2.0);
        let m = sweep(1.0);
        let sub = trace_math(&d, &m, Math::Sub).unwrap();
        assert!((sub[3].s11.re - 0.3).abs() < 1e-12 && (sub[3].s11.im + 0.05).abs() < 1e-12);
        let div = trace_math(&d, &d, Math::Div).unwrap();
        for s in &div[1..] {
            assert!((s.s11.re - 1.0).abs() < 1e-12 && s.s11.im.abs() < 1e-12);
            assert!((rf::db(s.s21)).abs() < 1e-9);
        }
        // Normalising by a -6 dB memory lifts the trace by 6.02 dB.
        let half: Vec<Sample> = d.iter().map(|s| Sample { s21: s.s21.scale(0.5), ..*s }).collect();
        let n = trace_math(&d, &half, Math::Div).unwrap();
        assert!((rf::db(n[2].s21) - 6.0206).abs() < 1e-3);
        assert!(trace_math(&d, &m[..3], Math::Sub).is_none());
        assert!(trace_math(&d, &m, Math::Off).is_none());
    }

    #[test]
    fn touchstone_round_trips_s1p_and_s2p() {
        let d = sweep(1.0);
        let one = parse_touchstone(&rf::touchstone_s1p(&d, 50.0)).unwrap();
        assert!(!one.has_s21);
        let two = parse_touchstone(&rf::touchstone_s2p(&d, 75.0)).unwrap();
        assert!(two.has_s21);
        assert_eq!(two.z0, 75.0);
        for (a, b) in d.iter().zip(&two.samples) {
            assert!((a.hz - b.hz).abs() < 1e-6);
            assert!((a.s11.re - b.s11.re).abs() < 1e-7 && (a.s21.im - b.s21.im).abs() < 1e-7);
        }
        for (a, b) in d.iter().zip(&one.samples) {
            assert!((a.s11.im - b.s11.im).abs() < 1e-7);
        }
    }

    #[test]
    fn touchstone_reads_ma_db_and_units() {
        let ma = parse_touchstone("! x\n# MHz S MA R 50\n1 0.5 90\n2 1 180\n").unwrap();
        assert_eq!(ma.samples[0].hz, 1e6);
        assert!(ma.samples[0].s11.re.abs() < 1e-12 && (ma.samples[0].s11.im - 0.5).abs() < 1e-12);
        let db = parse_touchstone("# kHz S DB R 50\n100 -6.0206 0\n200 0 -90\n").unwrap();
        assert_eq!(db.samples[1].hz, 200e3);
        assert!((db.samples[0].s11.re - 0.5).abs() < 1e-4);
        let ghz = parse_touchstone("#\n1 0.1 0\n2 0.2 0\n").unwrap();
        assert_eq!(ghz.samples[1].hz, 2e9);
        assert!(parse_touchstone("# Hz Z RI R 50\n1 1 1\n2 1 1\n").is_err());
    }

    #[test]
    fn resampling_interpolates_and_blanks_outside() {
        let r = resample(&[0.0, 10.0], &[0.0, 100.0], &[-1.0, 0.0, 2.5, 10.0, 11.0]);
        assert!(r[0].is_nan() && r[4].is_nan());
        assert_eq!(&r[1..4], &[0.0, 25.0, 100.0]);
    }

    #[test]
    fn renormalising_moves_a_matched_load() {
        // 75 Ω load in a 75 Ω file is Γ 0; on 50 Ω it is 0.2.
        let mut s = vec![Sample { hz: 1.0, s11: Cx::ZERO, s21: Cx::ZERO }];
        renormalize(&mut s, 75.0, 50.0);
        assert!((s[0].s11.re - 0.2).abs() < 1e-12);
    }

    #[test]
    fn csv_import_reads_commander_csv_and_analyzer_columns() {
        let d = sweep(1.0);
        let back = parse_csv(&rf::csv_table(&d, 50.0, true), 50.0).unwrap();
        assert!(back.has_s21);
        assert!((back.samples[4].s11.re - d[4].s11.re).abs() < 1e-7);
        let rx = parse_csv("mhz,r,x\n14.0,50,0\n14.1,100,50\n", 50.0).unwrap();
        assert_eq!(rx.samples[0].hz, 14.0e6);
        assert!((rf::swr(rx.samples[1].s11) - 2.618).abs() < 1e-3);
        let swr = parse_csv("Freq_MHz,SWR\n7.0,1.5\n7.1,3\n", 50.0).unwrap();
        assert!((rf::swr(swr.samples[1].s11) - 3.0).abs() < 1e-9);
        assert!(parse_csv("a,b\n1,2\n", 50.0).is_err());
    }
}

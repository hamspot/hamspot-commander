//! Pure RF math for the ham_rf example. No UI, no I/O, so every formula is unit-tested.
//! Frequencies are MHz, lengths are feet or metres as named, power is watts unless named.
//! These are rules of thumb for the workshop bench: real antennas need trimming with an
//! analyzer, and cable data is approximate.

/// Speed of light in vacuum, m/s.
pub const C: f64 = 299_792_458.0;
pub const FT_PER_M: f64 = 1.0 / 0.3048;

// ------------------------------------------------------------ units
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FreqUnit {
    Mega,
    Kilo,
    Giga,
}
impl FreqUnit {
    pub const ALL: [FreqUnit; 3] = [FreqUnit::Mega, FreqUnit::Kilo, FreqUnit::Giga];
    pub fn label(self) -> &'static str {
        match self {
            FreqUnit::Mega => "MHz",
            FreqUnit::Kilo => "kHz",
            FreqUnit::Giga => "GHz",
        }
    }
    pub fn to_mhz(self, v: f64) -> f64 {
        match self {
            FreqUnit::Mega => v,
            FreqUnit::Kilo => v / 1e3,
            FreqUnit::Giga => v * 1e3,
        }
    }
    pub fn mhz_to(self, mhz: f64) -> f64 {
        match self {
            FreqUnit::Mega => mhz,
            FreqUnit::Kilo => mhz * 1e3,
            FreqUnit::Giga => mhz / 1e3,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LenUnit {
    Ft,
    M,
    In,
}
impl LenUnit {
    pub const ALL: [LenUnit; 3] = [LenUnit::Ft, LenUnit::M, LenUnit::In];
    pub fn label(self) -> &'static str {
        match self {
            LenUnit::Ft => "ft",
            LenUnit::M => "m",
            LenUnit::In => "in",
        }
    }
    pub fn ft_to(self, ft: f64) -> f64 {
        match self {
            LenUnit::Ft => ft,
            LenUnit::M => ft * 0.3048,
            LenUnit::In => ft * 12.0,
        }
    }
}

// ------------------------------------------------------------ frequency <-> wavelength
pub fn wavelength_m(f_mhz: f64) -> f64 {
    C / 1e6 / f_mhz
}
pub fn freq_mhz(wavelength_m: f64) -> f64 {
    C / 1e6 / wavelength_m
}

// ------------------------------------------------------------ antenna elements
#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub name: &'static str,
    pub ft: f64,
    pub note: &'static str,
}

/// Half-wave dipole, total length in feet: the classic 468/f rule (includes ~5% end effect).
pub fn dipole_ft(f_mhz: f64) -> f64 {
    468.0 / f_mhz
}

/// Starting lengths for common wire/rod antennas (feet), all approximate.
pub fn antenna_elements(f_mhz: f64) -> Vec<Element> {
    let dip = dipole_ft(f_mhz);
    vec![
        Element {
            name: "Half-wave dipole (total)",
            ft: dip,
            note: "468/f",
        },
        Element {
            name: "Dipole, each leg",
            ft: dip / 2.0,
            note: "half of total",
        },
        Element {
            name: "Quarter-wave vertical",
            ft: 234.0 / f_mhz,
            note: "234/f, needs radials",
        },
        Element {
            name: "Radial, each",
            ft: 234.0 / f_mhz * 1.05,
            note: "about 5% over quarter wave",
        },
        Element {
            name: "Full-wave loop (perimeter)",
            ft: 1005.0 / f_mhz,
            note: "1005/f",
        },
        Element {
            name: "J-pole, total height",
            ft: 702.0 / f_mhz,
            note: "3/4 wave, 702/f",
        },
        Element {
            name: "J-pole, stub section",
            ft: 234.0 / f_mhz,
            note: "quarter-wave matching stub",
        },
        Element {
            name: "3-el Yagi, reflector",
            ft: dip * 1.05,
            note: "5% over driven",
        },
        Element {
            name: "3-el Yagi, driven",
            ft: dip,
            note: "468/f, trim for SWR",
        },
        Element {
            name: "3-el Yagi, director",
            ft: dip * 0.95,
            note: "5% under driven",
        },
        Element {
            name: "3-el Yagi, spacing",
            ft: 0.2 * wavelength_m(f_mhz) * FT_PER_M,
            note: "0.2 wavelength, each gap",
        },
    ]
}

// ------------------------------------------------------------ coax
pub struct Cable {
    pub name: &'static str,
    pub vf: f64,
    /// (MHz, dB per 100 ft, matched). Typical datasheet values, rounded.
    pub pts: &'static [(f64, f64)],
}

pub const CABLES: [Cable; 7] = [
    Cable {
        name: "RG-174",
        vf: 0.66,
        pts: &[
            (1.0, 1.9),
            (10.0, 3.2),
            (50.0, 6.2),
            (100.0, 8.9),
            (200.0, 12.6),
            (400.0, 17.3),
            (900.0, 27.0),
        ],
    },
    Cable {
        name: "RG-58",
        vf: 0.66,
        pts: &[
            (1.0, 0.4),
            (10.0, 1.4),
            (50.0, 3.3),
            (100.0, 4.9),
            (200.0, 7.3),
            (400.0, 11.2),
            (900.0, 17.0),
        ],
    },
    Cable {
        name: "RG-8X",
        vf: 0.78,
        pts: &[
            (1.0, 0.3),
            (10.0, 1.0),
            (50.0, 2.5),
            (100.0, 3.7),
            (200.0, 5.4),
            (400.0, 8.0),
            (900.0, 12.6),
        ],
    },
    Cable {
        name: "RG-213",
        vf: 0.66,
        pts: &[
            (1.0, 0.2),
            (10.0, 0.6),
            (50.0, 1.3),
            (100.0, 1.9),
            (200.0, 2.8),
            (400.0, 4.5),
            (900.0, 7.2),
        ],
    },
    Cable {
        name: "LMR-400",
        vf: 0.85,
        pts: &[
            (1.0, 0.1),
            (10.0, 0.3),
            (50.0, 0.7),
            (100.0, 0.9),
            (200.0, 1.4),
            (400.0, 2.1),
            (900.0, 3.2),
        ],
    },
    Cable {
        name: "LMR-600",
        vf: 0.87,
        pts: &[
            (1.0, 0.05),
            (10.0, 0.17),
            (50.0, 0.4),
            (100.0, 0.5),
            (200.0, 0.8),
            (400.0, 1.2),
            (900.0, 1.8),
        ],
    },
    Cable {
        name: "1/2 in hardline",
        vf: 0.88,
        pts: &[
            (1.0, 0.02),
            (10.0, 0.08),
            (50.0, 0.3),
            (100.0, 0.4),
            (200.0, 0.6),
            (400.0, 0.9),
            (900.0, 1.3),
        ],
    },
];

/// Matched loss in dB per 100 ft at `f_mhz`, log-log interpolated between table points.
/// Returns `(loss, extrapolated)`; outside the table the loss follows sqrt(f) from the nearest point.
pub fn loss_per_100ft(c: &Cable, f_mhz: f64) -> (f64, bool) {
    let p = c.pts;
    let (first, last) = (p[0], p[p.len() - 1]);
    if f_mhz <= first.0 {
        return (first.1 * (f_mhz / first.0).sqrt(), f_mhz < first.0);
    }
    if f_mhz >= last.0 {
        return (last.1 * (f_mhz / last.0).sqrt(), f_mhz > last.0);
    }
    for w in p.windows(2) {
        let ((f0, l0), (f1, l1)) = (w[0], w[1]);
        if f_mhz >= f0 && f_mhz <= f1 {
            let t = (f_mhz / f0).ln() / (f1 / f0).ln();
            return ((l0.ln() + t * (l1.ln() - l0.ln())).exp(), false);
        }
    }
    (last.1, false)
}

/// Total line loss (dB) with a mismatched load, from the matched loss `ml_db` and the load's
/// SWR: `10 log10[(a^2 - g^2) / (a (1 - g^2))]`, a = 10^(ml/10), g = reflection coefficient at the load.
pub fn loss_with_swr_db(ml_db: f64, load_swr: f64) -> f64 {
    let a = 10f64.powf(ml_db / 10.0);
    let g = gamma_from_swr(load_swr);
    10.0 * ((a * a - g * g) / (a * (1.0 - g * g))).log10()
}

// ------------------------------------------------------------ SWR family
#[derive(Clone, Copy, Debug)]
pub struct Reflection {
    pub swr: f64,
    pub gamma: f64,
    pub return_loss_db: f64,
    pub mismatch_loss_db: f64,
    pub reflected_pct: f64,
}

pub fn gamma_from_swr(swr: f64) -> f64 {
    (swr - 1.0) / (swr + 1.0)
}
pub fn swr_from_gamma(g: f64) -> f64 {
    if g >= 1.0 {
        f64::INFINITY
    } else {
        (1.0 + g) / (1.0 - g)
    }
}

impl Reflection {
    /// From |gamma| in `0..=1`.
    pub fn from_gamma(g: f64) -> Result<Self, &'static str> {
        if !(0.0..=1.0).contains(&g) || g.is_nan() {
            return Err("reflection coefficient must be 0 to 1");
        }
        Ok(Self {
            swr: swr_from_gamma(g),
            gamma: g,
            return_loss_db: -20.0 * g.log10(),
            mismatch_loss_db: -10.0 * (1.0 - g * g).log10(),
            reflected_pct: g * g * 100.0,
        })
    }
    pub fn from_swr(swr: f64) -> Result<Self, &'static str> {
        if swr.is_nan() || swr < 1.0 {
            return Err("SWR must be 1 or more");
        }
        Self::from_gamma(gamma_from_swr(swr))
    }
    pub fn from_return_loss(rl_db: f64) -> Result<Self, &'static str> {
        if rl_db.is_nan() || rl_db < 0.0 {
            return Err("return loss must be 0 dB or more");
        }
        Self::from_gamma(10f64.powf(-rl_db / 20.0))
    }
}

// ------------------------------------------------------------ dB and power
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PowerUnit {
    W,
    MW,
    Dbm,
    Dbw,
    Vrms,
}
impl PowerUnit {
    pub const ALL: [PowerUnit; 5] = [
        PowerUnit::W,
        PowerUnit::MW,
        PowerUnit::Dbm,
        PowerUnit::Dbw,
        PowerUnit::Vrms,
    ];
    pub fn label(self) -> &'static str {
        match self {
            PowerUnit::W => "W",
            PowerUnit::MW => "mW",
            PowerUnit::Dbm => "dBm",
            PowerUnit::Dbw => "dBW",
            PowerUnit::Vrms => "Vrms",
        }
    }
    pub fn to_watts(self, v: f64, r_ohm: f64) -> f64 {
        match self {
            PowerUnit::W => v,
            PowerUnit::MW => v / 1e3,
            PowerUnit::Dbm => 10f64.powf(v / 10.0) / 1e3,
            PowerUnit::Dbw => 10f64.powf(v / 10.0),
            PowerUnit::Vrms => v * v / r_ohm,
        }
    }
    pub fn watts_to(self, w: f64, r_ohm: f64) -> f64 {
        match self {
            PowerUnit::W => w,
            PowerUnit::MW => w * 1e3,
            PowerUnit::Dbm => 10.0 * (w * 1e3).log10(),
            PowerUnit::Dbw => 10.0 * w.log10(),
            PowerUnit::Vrms => (w * r_ohm).sqrt(),
        }
    }
}

pub fn db_to_power_ratio(db: f64) -> f64 {
    10f64.powf(db / 10.0)
}
pub fn db_to_voltage_ratio(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}
pub fn power_ratio_to_db(r: f64) -> f64 {
    10.0 * r.log10()
}

/// Peak-to-peak volts of a sine of `vrms`.
pub fn vpp(vrms: f64) -> f64 {
    vrms * 2.0 * std::f64::consts::SQRT_2
}

/// S-meter reading for a level in dBm, using the IARU convention
/// (S9 = -73 dBm below 30 MHz, -93 dBm above, 6 dB per S-unit). Example: "S7", "S9+20 dB".
pub fn s_meter(dbm: f64, vhf: bool) -> String {
    let s9 = if vhf { -93.0 } else { -73.0 };
    if dbm >= s9 {
        let over = (dbm - s9).round() as i64;
        if over == 0 {
            "S9".into()
        } else {
            format!("S9+{over} dB")
        }
    } else {
        let s = 9.0 - (s9 - dbm) / 6.0;
        if s < 1.0 {
            "below S1".into()
        } else {
            format!("S{}", s.round() as i64)
        }
    }
}

// ------------------------------------------------------------ Ohm's law
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Q {
    V,
    I,
    R,
    P,
}
impl Q {
    pub const ALL: [Q; 4] = [Q::V, Q::I, Q::R, Q::P];
    pub fn label(self) -> &'static str {
        match self {
            Q::V => "Voltage (V)",
            Q::I => "Current (A)",
            Q::R => "Resistance (ohm)",
            Q::P => "Power (W)",
        }
    }
    pub fn index(self) -> usize {
        self as usize
    }
}

/// Given exactly two different known quantities, returns `[V, I, R, P]`.
pub fn ohms_law(a: (Q, f64), b: (Q, f64)) -> Result<[f64; 4], &'static str> {
    if a.0 == b.0 {
        return Err("enter two different quantities");
    }
    for v in [a.1, b.1] {
        if !v.is_finite() || v <= 0.0 {
            return Err("values must be positive numbers");
        }
    }
    let get = |q: Q| {
        if a.0 == q {
            Some(a.1)
        } else if b.0 == q {
            Some(b.1)
        } else {
            None
        }
    };
    let (mut v, mut i, mut r, mut p) = (get(Q::V), get(Q::I), get(Q::R), get(Q::P));
    // each pass fills what two knowns determine
    for _ in 0..2 {
        if v.is_none() {
            if let (Some(i_v), Some(r_v)) = (i, r) {
                v = Some(i_v * r_v);
            }
        }
        if v.is_none() {
            if let (Some(p_v), Some(i_v)) = (p, i) {
                v = Some(p_v / i_v);
            }
        }
        if v.is_none() {
            if let (Some(p_v), Some(r_v)) = (p, r) {
                v = Some((p_v * r_v).sqrt());
            }
        }
        if i.is_none() {
            if let (Some(v_v), Some(r_v)) = (v, r) {
                i = Some(v_v / r_v);
            }
        }
        if i.is_none() {
            if let (Some(p_v), Some(v_v)) = (p, v) {
                i = Some(p_v / v_v);
            }
        }
        if i.is_none() {
            if let (Some(p_v), Some(r_v)) = (p, r) {
                i = Some((p_v / r_v).sqrt());
            }
        }
        if r.is_none() {
            if let (Some(v_v), Some(i_v)) = (v, i) {
                r = Some(v_v / i_v);
            }
        }
        if p.is_none() {
            if let (Some(v_v), Some(i_v)) = (v, i) {
                p = Some(v_v * i_v);
            }
        }
    }
    match (v, i, r, p) {
        (Some(v), Some(i), Some(r), Some(p)) => Ok([v, i, r, p]),
        _ => Err("cannot solve"),
    }
}

// ------------------------------------------------------------ LC resonance
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LcSolve {
    Freq,
    L,
    C,
}

/// f in MHz, L in uH, C in pF. Solves for the quantity named; the other two are inputs.
pub fn lc_resonance(solve: LcSolve, f_mhz: f64, l_uh: f64, c_pf: f64) -> Result<f64, &'static str> {
    let need: &[f64] = match solve {
        LcSolve::Freq => &[l_uh, c_pf],
        LcSolve::L => &[f_mhz, c_pf],
        LcSolve::C => &[f_mhz, l_uh],
    };
    if need.iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return Err("inputs must be positive numbers");
    }
    let tau = 2.0 * std::f64::consts::PI;
    Ok(match solve {
        LcSolve::Freq => 1.0 / (tau * (l_uh * 1e-6 * c_pf * 1e-12).sqrt()) / 1e6,
        LcSolve::L => 1.0 / ((tau * f_mhz * 1e6).powi(2) * c_pf * 1e-12) * 1e6,
        LcSolve::C => 1.0 / ((tau * f_mhz * 1e6).powi(2) * l_uh * 1e-6) * 1e12,
    })
}

/// Characteristic reactance of the resonant pair, sqrt(L/C), in ohms.
pub fn lc_reactance_ohm(l_uh: f64, c_pf: f64) -> f64 {
    (l_uh * 1e-6 / (c_pf * 1e-12)).sqrt()
}

// ------------------------------------------------------------ L-network
#[derive(Clone, Copy, Debug)]
pub struct LNetwork {
    pub q: f64,
    /// Reactance magnitudes, ohms.
    pub x_series: f64,
    pub x_shunt: f64,
    /// True when the shunt element sits on the source side (source R is the higher one).
    pub shunt_at_source: bool,
    /// Low-pass form: series L (nH) and shunt C (pF).
    pub lp_series_l_nh: f64,
    pub lp_shunt_c_pf: f64,
    /// High-pass form: series C (pF) and shunt L (nH).
    pub hp_series_c_pf: f64,
    pub hp_shunt_l_nh: f64,
}

/// Matches resistive `r_source` to `r_load` at `f_mhz`. The shunt element goes across the
/// higher resistance; the series element is on the lower-resistance side.
pub fn l_network(r_source: f64, r_load: f64, f_mhz: f64) -> Result<LNetwork, &'static str> {
    for v in [r_source, r_load, f_mhz] {
        if !v.is_finite() || v <= 0.0 {
            return Err("inputs must be positive numbers");
        }
    }
    if (r_source - r_load).abs() < 1e-9 {
        return Err("resistances are equal: no matching network needed");
    }
    let (hi, lo) = (r_source.max(r_load), r_source.min(r_load));
    let q = (hi / lo - 1.0).sqrt();
    let (xs, xp) = (q * lo, hi / q);
    let w = 2.0 * std::f64::consts::PI * f_mhz * 1e6;
    Ok(LNetwork {
        q,
        x_series: xs,
        x_shunt: xp,
        shunt_at_source: r_source > r_load,
        lp_series_l_nh: xs / w * 1e9,
        lp_shunt_c_pf: 1.0 / (w * xp) * 1e12,
        hp_series_c_pf: 1.0 / (w * xs) * 1e12,
        hp_shunt_l_nh: xp / w * 1e9,
    })
}

// ------------------------------------------------------------ path loss
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DistUnit {
    Km,
    Mi,
    Nmi,
}
impl DistUnit {
    pub const ALL: [DistUnit; 3] = [DistUnit::Km, DistUnit::Mi, DistUnit::Nmi];
    pub fn label(self) -> &'static str {
        match self {
            DistUnit::Km => "km",
            DistUnit::Mi => "mi",
            DistUnit::Nmi => "nmi",
        }
    }
    pub fn to_km(self, d: f64) -> f64 {
        match self {
            DistUnit::Km => d,
            DistUnit::Mi => d * 1.609344,
            DistUnit::Nmi => d * 1.852,
        }
    }
}

/// Free-space path loss: `20 log10(d_km) + 20 log10(f_MHz) + 32.44`.
pub fn fspl_db(d_km: f64, f_mhz: f64) -> f64 {
    20.0 * d_km.log10() + 20.0 * f_mhz.log10() + 32.44
}

/// Received level in dBm for a simple free-space link.
pub fn link_rx_dbm(
    tx_w: f64,
    tx_gain_dbi: f64,
    rx_gain_dbi: f64,
    line_loss_db: f64,
    path_db: f64,
) -> f64 {
    10.0 * (tx_w * 1e3).log10() + tx_gain_dbi + rx_gain_dbi - line_loss_db - path_db
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "{a} vs {b} (tol {tol})");
    }

    #[test]
    fn wavelength_known_values() {
        close(wavelength_m(300.0), 0.999_308, 1e-5);
        close(wavelength_m(14.2), 21.112, 1e-3); // "20 meters"
        close(wavelength_m(146.0), 2.053, 1e-3);
        close(freq_mhz(wavelength_m(7.15)), 7.15, 1e-9);
    }

    #[test]
    fn unit_round_trips() {
        for u in FreqUnit::ALL {
            close(u.to_mhz(u.mhz_to(14.2)), 14.2, 1e-9);
        }
        close(FreqUnit::Kilo.to_mhz(7150.0), 7.15, 1e-12);
        close(FreqUnit::Giga.to_mhz(1.296), 1296.0, 1e-9);
        close(LenUnit::M.ft_to(10.0), 3.048, 1e-12);
        close(LenUnit::In.ft_to(2.0), 24.0, 1e-12);
    }

    #[test]
    fn dipole_rule() {
        close(dipole_ft(14.2), 32.958, 1e-3); // 468/14.2
        close(dipole_ft(7.1) * 0.3048, 20.09, 0.01); // 40 m dipole about 20.1 m
        let e = antenna_elements(146.0);
        let get = |n: &str| e.iter().find(|x| x.name == n).unwrap().ft;
        close(get("Half-wave dipole (total)"), 468.0 / 146.0, 1e-12);
        close(
            get("Quarter-wave vertical") * 2.0,
            get("Half-wave dipole (total)"),
            1e-9,
        );
        close(get("J-pole, total height"), 702.0 / 146.0, 1e-12);
        assert!(get("3-el Yagi, reflector") > get("3-el Yagi, driven"));
        assert!(get("3-el Yagi, driven") > get("3-el Yagi, director"));
        close(get("3-el Yagi, spacing"), 0.2 * 2.0534 * FT_PER_M, 0.01);
    }

    #[test]
    fn coax_loss_interpolation() {
        let rg58 = &CABLES[1];
        assert_eq!(rg58.name, "RG-58");
        // exact table points
        close(loss_per_100ft(rg58, 100.0).0, 4.9, 1e-9);
        close(loss_per_100ft(rg58, 10.0).0, 1.4, 1e-9);
        // between points: monotonic and bracketed
        let (l, ex) = loss_per_100ft(rg58, 144.0);
        assert!(!ex && l > 4.9 && l < 7.3, "{l}");
        // sqrt(f) between 50 and 100 MHz: 3.3 -> 4.9 is ratio 1.485 (sqrt 2 = 1.414)
        let (l, _) = loss_per_100ft(rg58, 70.0);
        close(l, 4.0, 0.15);
        // extrapolation flagged, follows sqrt(f)
        let (l, ex) = loss_per_100ft(rg58, 3600.0);
        assert!(ex);
        close(l, 17.0 * 2.0, 1e-9);
        let (_, ex) = loss_per_100ft(rg58, 900.0);
        assert!(!ex);
        // every cable is monotonic with frequency and ordered by frequency
        for c in &CABLES {
            for w in c.pts.windows(2) {
                assert!(w[1].0 > w[0].0 && w[1].1 > w[0].1, "{}", c.name);
            }
            assert!(c.vf > 0.5 && c.vf < 1.0);
        }
        // better cable, less loss at 146 MHz
        assert!(loss_per_100ft(&CABLES[4], 146.0).0 < loss_per_100ft(&CABLES[1], 146.0).0);
    }

    #[test]
    fn swr_loss_adds_to_matched_loss() {
        close(loss_with_swr_db(1.0, 1.0), 1.0, 1e-9); // matched load: unchanged
        let t = loss_with_swr_db(1.0, 3.0);
        assert!(t > 1.4 && t < 1.6, "{t}"); // a 1 dB line at SWR 3 loses about 1.5 dB
        close(loss_with_swr_db(0.0, 5.0), 0.0, 1e-9); // lossless line: no extra loss
    }

    #[test]
    fn reflection_known_values() {
        let r = Reflection::from_swr(2.0).unwrap();
        close(r.gamma, 1.0 / 3.0, 1e-12);
        close(r.return_loss_db, 9.5424, 1e-3);
        close(r.mismatch_loss_db, 0.5115, 1e-3);
        close(r.reflected_pct, 11.111, 1e-2);
        let m = Reflection::from_swr(1.0).unwrap();
        assert!(m.return_loss_db.is_infinite());
        close(m.mismatch_loss_db, 0.0, 1e-12);
        // round trips
        let b = Reflection::from_return_loss(r.return_loss_db).unwrap();
        close(b.swr, 2.0, 1e-9);
        let c = Reflection::from_gamma(0.5).unwrap();
        close(c.swr, 3.0, 1e-12);
        close(c.return_loss_db, 6.0206, 1e-3);
        // 20 dB return loss is SWR 1.222
        close(
            Reflection::from_return_loss(20.0).unwrap().swr,
            1.2222,
            1e-3,
        );
        assert!(Reflection::from_swr(0.5).is_err());
        assert!(Reflection::from_gamma(1.5).is_err());
        assert!(Reflection::from_return_loss(-1.0).is_err());
        assert!(Reflection::from_gamma(1.0).unwrap().swr.is_infinite());
    }

    #[test]
    fn power_units() {
        for u in PowerUnit::ALL {
            close(u.watts_to(u.to_watts(0.25, 50.0), 50.0), 0.25, 1e-12);
        }
        close(PowerUnit::W.to_watts(1.0, 50.0), 1.0, 1e-12);
        close(PowerUnit::Dbm.watts_to(1.0, 50.0), 30.0, 1e-9);
        close(PowerUnit::Dbw.watts_to(1.0, 50.0), 0.0, 1e-9);
        close(PowerUnit::Dbm.to_watts(10.0, 50.0), 0.01, 1e-12);
        close(PowerUnit::Dbw.watts_to(100.0, 50.0), 20.0, 1e-9);
        close(PowerUnit::Vrms.watts_to(0.001, 50.0), 0.223_607, 1e-6); // 0 dBm across 50 ohm
        close(PowerUnit::Vrms.watts_to(100.0, 50.0), 70.7107, 1e-3);
        close(vpp(70.7107), 200.0, 1e-3);
        close(PowerUnit::MW.watts_to(0.5, 50.0), 500.0, 1e-9);
    }

    #[test]
    fn ratios_and_s_meter() {
        close(db_to_power_ratio(3.0), 1.9953, 1e-3);
        close(db_to_power_ratio(10.0), 10.0, 1e-12);
        close(db_to_voltage_ratio(6.0), 1.9953, 1e-3);
        close(power_ratio_to_db(100.0), 20.0, 1e-12);
        assert_eq!(s_meter(-73.0, false), "S9");
        assert_eq!(s_meter(-53.0, false), "S9+20 dB");
        assert_eq!(s_meter(-97.0, false), "S5"); // 9 - 24/6
        assert_eq!(s_meter(-93.0, true), "S9");
        assert_eq!(s_meter(-140.0, false), "below S1");
    }

    #[test]
    fn ohms_law_all_pairs() {
        let want = [12.0, 2.0, 6.0, 24.0];
        let known = [(Q::V, 12.0), (Q::I, 2.0), (Q::R, 6.0), (Q::P, 24.0)];
        for a in 0..4 {
            for b in 0..4 {
                if a == b {
                    assert!(ohms_law(known[a], known[b]).is_err());
                    continue;
                }
                let r = ohms_law(known[a], known[b]).unwrap();
                for k in 0..4 {
                    close(r[k], want[k], 1e-9);
                }
            }
        }
        let r = ohms_law((Q::P, 100.0), (Q::R, 50.0)).unwrap();
        close(r[0], 70.7107, 1e-3);
        close(r[1], std::f64::consts::SQRT_2, 1e-4);
        assert!(ohms_law((Q::V, -1.0), (Q::R, 5.0)).is_err());
        assert!(ohms_law((Q::V, f64::NAN), (Q::R, 5.0)).is_err());
    }

    #[test]
    fn lc_resonance_values() {
        close(
            lc_resonance(LcSolve::Freq, 0.0, 10.0, 100.0).unwrap(),
            5.0329,
            1e-3,
        );
        // same L*C product, same frequency
        close(
            lc_resonance(LcSolve::Freq, 0.0, 1.0, 1000.0).unwrap(),
            5.0329,
            1e-3,
        );
        // round trip: solve L and C back from f
        let f = lc_resonance(LcSolve::Freq, 0.0, 4.7, 56.0).unwrap();
        close(lc_resonance(LcSolve::L, f, 0.0, 56.0).unwrap(), 4.7, 1e-9);
        close(lc_resonance(LcSolve::C, f, 4.7, 0.0).unwrap(), 56.0, 1e-9);
        // 7.15 MHz with 10 uH needs 49.55 pF
        close(
            lc_resonance(LcSolve::C, 7.15, 10.0, 0.0).unwrap(),
            49.55,
            0.01,
        );
        close(lc_reactance_ohm(10.0, 100.0), 316.228, 1e-3);
        assert!(lc_resonance(LcSolve::Freq, 0.0, 0.0, 10.0).is_err());
        assert!(lc_resonance(LcSolve::L, 7.0, 0.0, -3.0).is_err());
    }

    #[test]
    fn l_network_known_values() {
        let n = l_network(50.0, 200.0, 14.2).unwrap();
        close(n.q, 3f64.sqrt(), 1e-12);
        close(n.x_series, 86.603, 1e-3);
        close(n.x_shunt, 115.470, 1e-3);
        assert!(!n.shunt_at_source); // 200 ohm load is the high side
        // reactance -> component at 14.2 MHz: L = X/w, C = 1/(wX)
        close(
            n.lp_series_l_nh,
            86.603 / (2.0 * std::f64::consts::PI * 14.2e6) * 1e9,
            0.01,
        );
        close(n.lp_shunt_c_pf, 97.07, 0.05);
        close(n.hp_series_c_pf, 129.4, 0.1);
        close(n.hp_shunt_l_nh, 1294.1, 0.5);
        // symmetric in direction: swapping source and load keeps the same parts
        let m = l_network(200.0, 50.0, 14.2).unwrap();
        assert!(m.shunt_at_source);
        close(m.q, n.q, 1e-12);
        close(m.lp_series_l_nh, n.lp_series_l_nh, 1e-9);
        // 50 -> 12.5 ohm: Q = sqrt(3)
        close(l_network(50.0, 12.5, 7.0).unwrap().q, 3f64.sqrt(), 1e-12);
        assert!(l_network(50.0, 50.0, 7.0).is_err());
        assert!(l_network(0.0, 50.0, 7.0).is_err());
    }

    #[test]
    fn path_loss() {
        close(fspl_db(1.0, 100.0), 72.44, 1e-9);
        close(fspl_db(10.0, 146.0), 32.44 + 20.0 + 43.2871, 1e-3);
        // doubling the distance adds 6.02 dB
        close(fspl_db(2.0, 435.0) - fspl_db(1.0, 435.0), 6.0206, 1e-3);
        close(DistUnit::Mi.to_km(10.0), 16.09344, 1e-9);
        close(DistUnit::Nmi.to_km(1.0), 1.852, 1e-12);
        // 5 W (37 dBm), 0 dBi each end, 1 dB line loss, 10 km at 146 MHz
        let rx = link_rx_dbm(5.0, 0.0, 0.0, 1.0, fspl_db(10.0, 146.0));
        close(rx, 36.9897 - 1.0 - 95.7271, 1e-3);
    }
}

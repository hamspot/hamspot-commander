//! US amateur band plan lookup (FCC Part 97). **Informational only**: privileges depend on
//! licence class and change over time. Always check the current rules (47 CFR 97.301-97.307).

#[derive(Debug, PartialEq)]
pub struct Band {
    pub name: &'static str,
    pub lo_mhz: f64,
    pub hi_mhz: f64,
    /// General notes about the band.
    pub note: &'static str,
    /// Sub-band segments inside the band: (lo, hi, who and what).
    pub segments: &'static [(f64, f64, &'static str)],
}

impl Band {
    pub fn center_mhz(&self) -> f64 {
        (self.lo_mhz + self.hi_mhz) / 2.0
    }
    pub fn contains(&self, mhz: f64) -> bool {
        mhz >= self.lo_mhz && mhz <= self.hi_mhz
    }
}

const ALL_MODES: &str = "All amateur modes, all classes";

pub const BANDS: &[Band] = &[
    Band {
        name: "2200 m",
        lo_mhz: 0.1357,
        hi_mhz: 0.1378,
        note: "LF, 1 W EIRP, CW and data",
        segments: &[],
    },
    Band {
        name: "630 m",
        lo_mhz: 0.472,
        hi_mhz: 0.479,
        note: "MF, 5 W EIRP, CW and data",
        segments: &[],
    },
    Band {
        name: "160 m",
        lo_mhz: 1.8,
        hi_mhz: 2.0,
        note: "MF, General and above",
        segments: &[],
    },
    Band {
        name: "80 m",
        lo_mhz: 3.5,
        hi_mhz: 4.0,
        note: "HF, night band",
        segments: &[
            (
                3.5,
                3.6,
                "CW and data (Extra, General, Tech/Novice CW 3.525-3.6)",
            ),
            (3.6, 4.0, "Phone: Extra from 3.600, General from 3.800"),
        ],
    },
    Band {
        name: "60 m",
        lo_mhz: 5.3305,
        hi_mhz: 5.4035,
        note: "HF, channelized, USB, General+",
        segments: &[],
    },
    Band {
        name: "40 m",
        lo_mhz: 7.0,
        hi_mhz: 7.3,
        note: "HF, day and night",
        segments: &[
            (
                7.0,
                7.125,
                "CW and data (Extra, General, Tech/Novice CW 7.025-7.125)",
            ),
            (7.125, 7.3, "Phone: Extra from 7.125, General from 7.175"),
        ],
    },
    Band {
        name: "30 m",
        lo_mhz: 10.1,
        hi_mhz: 10.15,
        note: "HF, CW and data only, 200 W",
        segments: &[],
    },
    Band {
        name: "20 m",
        lo_mhz: 14.0,
        hi_mhz: 14.35,
        note: "HF, daytime DX",
        segments: &[
            (14.0, 14.15, "CW and data (Extra, General)"),
            (
                14.15,
                14.35,
                "Phone: Extra from 14.150, General from 14.225",
            ),
        ],
    },
    Band {
        name: "17 m",
        lo_mhz: 18.068,
        hi_mhz: 18.168,
        note: "HF, General and above",
        segments: &[
            (18.068, 18.11, "CW and data"),
            (18.11, 18.168, "Phone and CW"),
        ],
    },
    Band {
        name: "15 m",
        lo_mhz: 21.0,
        hi_mhz: 21.45,
        note: "HF, daytime DX",
        segments: &[
            (
                21.0,
                21.2,
                "CW and data (Extra, General, Tech/Novice CW 21.025-21.2)",
            ),
            (21.2, 21.45, "Phone: Extra from 21.200, General from 21.275"),
        ],
    },
    Band {
        name: "12 m",
        lo_mhz: 24.89,
        hi_mhz: 24.99,
        note: "HF, General and above",
        segments: &[
            (24.89, 24.93, "CW and data"),
            (24.93, 24.99, "Phone and CW"),
        ],
    },
    Band {
        name: "10 m",
        lo_mhz: 28.0,
        hi_mhz: 29.7,
        note: "HF, open to Technician on CW/SSB",
        segments: &[
            (28.0, 28.3, "CW and data (all classes)"),
            (28.3, 28.5, "SSB phone (all classes, Technician included)"),
            (28.5, 29.7, "Phone and FM (General and above)"),
        ],
    },
    Band {
        name: "6 m",
        lo_mhz: 50.0,
        hi_mhz: 54.0,
        note: "VHF, the magic band",
        segments: &[(50.0, 50.1, "CW only"), (50.1, 54.0, ALL_MODES)],
    },
    Band {
        name: "2 m",
        lo_mhz: 144.0,
        hi_mhz: 148.0,
        note: "VHF, FM repeaters and weak-signal",
        segments: &[(144.0, 144.1, "CW only"), (144.1, 148.0, ALL_MODES)],
    },
    Band {
        name: "1.25 m",
        lo_mhz: 222.0,
        hi_mhz: 225.0,
        note: "VHF, FM and repeaters",
        segments: &[],
    },
    Band {
        name: "70 cm",
        lo_mhz: 420.0,
        hi_mhz: 450.0,
        note: "UHF, secondary to radiolocation",
        segments: &[],
    },
    Band {
        name: "33 cm",
        lo_mhz: 902.0,
        hi_mhz: 928.0,
        note: "UHF, shared with ISM and others",
        segments: &[],
    },
    Band {
        name: "23 cm",
        lo_mhz: 1240.0,
        hi_mhz: 1300.0,
        note: "UHF, shared use",
        segments: &[],
    },
];

/// The band that contains `mhz`, edges included.
pub fn find_band(mhz: f64) -> Option<&'static Band> {
    BANDS.iter().find(|b| b.contains(mhz))
}

/// The segment line that applies at `mhz` within `band`, if the band has segments.
pub fn segment_at(band: &Band, mhz: f64) -> Option<&'static str> {
    band.segments
        .iter()
        .find(|(lo, hi, _)| (mhz >= *lo && mhz < *hi) || (mhz == *hi && *hi == band.hi_mhz))
        .map(|s| s.2)
}

/// The closest band below and above `mhz`, for "not in an amateur band" messages.
pub fn neighbours(mhz: f64) -> (Option<&'static Band>, Option<&'static Band>) {
    let below = BANDS.iter().rev().find(|b| b.hi_mhz < mhz);
    let above = BANDS.iter().find(|b| b.lo_mhz > mhz);
    (below, above)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_known_frequencies() {
        assert_eq!(find_band(14.2).unwrap().name, "20 m");
        assert_eq!(find_band(7.074).unwrap().name, "40 m");
        assert_eq!(find_band(146.52).unwrap().name, "2 m");
        assert_eq!(find_band(446.0).unwrap().name, "70 cm");
        assert_eq!(find_band(1296.1).unwrap().name, "23 cm");
        assert_eq!(find_band(0.1370).unwrap().name, "2200 m");
    }

    #[test]
    fn edges_are_inclusive_and_gaps_are_none() {
        assert_eq!(find_band(14.0).unwrap().name, "20 m");
        assert_eq!(find_band(14.35).unwrap().name, "20 m");
        assert!(find_band(14.351).is_none());
        assert!(find_band(13.999).is_none());
        assert!(find_band(27.185).is_none()); // CB
        assert!(find_band(100.0).is_none()); // FM broadcast
        assert!(find_band(0.0).is_none());
    }

    #[test]
    fn table_is_sorted_and_sane() {
        for w in BANDS.windows(2) {
            assert!(
                w[0].hi_mhz < w[1].lo_mhz,
                "{} overlaps {}",
                w[0].name,
                w[1].name
            );
        }
        for b in BANDS {
            assert!(b.lo_mhz < b.hi_mhz);
            // segments sit inside the band, are contiguous
            for s in b.segments {
                assert!(s.0 >= b.lo_mhz && s.1 <= b.hi_mhz, "{}", b.name);
            }
            for w in b.segments.windows(2) {
                assert_eq!(w[0].1, w[1].0, "{}", b.name);
            }
            assert!((b.center_mhz() - (b.lo_mhz + b.hi_mhz) / 2.0).abs() < 1e-12);
        }
    }

    #[test]
    fn segments_and_neighbours() {
        let b = find_band(14.2).unwrap();
        assert!(segment_at(b, 14.2).unwrap().starts_with("Phone"));
        assert!(segment_at(b, 14.05).unwrap().starts_with("CW"));
        assert!(segment_at(b, 14.35).unwrap().starts_with("Phone")); // top edge
        assert!(segment_at(find_band(5.35).unwrap(), 5.35).is_none());
        let (lo, hi) = neighbours(100.0);
        assert_eq!(lo.unwrap().name, "6 m");
        assert_eq!(hi.unwrap().name, "2 m");
        assert!(neighbours(1.0).0.is_some() && neighbours(1.0).1.is_some());
        assert!(neighbours(5000.0).1.is_none());
    }
}

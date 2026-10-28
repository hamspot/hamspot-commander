use super::*;
use crate::patchbay::model::Jack;
use crate::tx_lockout::TxLockout;

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn step(a: &str, b: Option<&str>, loads: &[&str]) -> PairStep {
    PairStep { ch0: a.into(), ch1: b.map(Into::into), loads: names(loads) }
}

#[test]
fn every_table_kind_loads_and_has_ports() {
    for d in kinds::table() {
        if d.key != kinds::CUSTOM {
            assert!(!d.ports.is_empty(), "{}", d.key);
        }
        for (a, b) in &d.through {
            assert!(d.ports.iter().any(|p| &p.0 == a) && d.ports.iter().any(|p| &p.0 == b), "{}", d.key);
        }
    }
    assert!(kinds::def("duplexer").is_some());
}

#[test]
fn duplexer_plan_is_three_pairs_then_common_s11() {
    let mut bay = PatchBay::default();
    let id = bay.add(Kind::Rf, "Dup", [0.0, 0.0]);
    bay.devices[0].rf = "duplexer".into();
    let p = plan_for(bay.device(id).unwrap(), None);
    assert_eq!(
        p,
        vec![
            step("TX", Some("RX"), &["COMMON"]),
            step("TX", Some("COMMON"), &["RX"]),
            step("RX", Some("COMMON"), &["TX"]),
            step("COMMON", None, &["TX", "RX"]),
        ]
    );
    assert!(p[0].text().contains("terminate COMMON with 50 Ω loads"));
    assert!(p[0].text().contains("VNA PORT 1 (CH0)"));
}

#[test]
fn coupler_plan_covers_all_six_pairs() {
    let p = plan(&names(&["IN", "OUT", "COUPLED", "ISOLATED"]), PlanKind::AllPairs);
    assert_eq!(p.len(), 6 + 1); // six pairs plus S11 at ISOLATED
    assert_eq!(p[1], step("IN", Some("COUPLED"), &["OUT", "ISOLATED"]));
    assert_eq!(p[6], step("ISOLATED", None, &["IN", "OUT", "COUPLED"]));
}

#[test]
fn circulator_is_measured_in_every_direction() {
    let p = plan(&names(&["1", "2", "3"]), PlanKind::AllOrdered);
    assert_eq!(p.len(), 6);
    assert!(p.contains(&step("2", Some("1"), &["3"])));
}

#[test]
fn custom_n_port_and_one_port_plans() {
    let mut d = Device::new(1, Kind::Rf, "X", [0.0, 0.0]);
    d.rf = kinds::CUSTOM.into();
    d.nports = 5;
    assert_eq!(rf_ports(&d), names(&["P1", "P2", "P3", "P4", "P5"]));
    assert_eq!(plan_for(&d, None).len(), 10 + 1);
    let a = Device::new(2, Kind::Antenna, "Yagi", [0.0, 0.0]);
    assert_eq!(plan_for(&a, None), vec![step("FEED", None, &[])]);
}

#[test]
fn a_radio_jack_is_a_one_port_scan_and_needs_confirmation() {
    let r = Device::new(1, Kind::Radio, "IC-9700", [0.0, 0.0]);
    assert_eq!(plan_for(&r, Some("ANT2")), vec![step("ANT2", None, &[])]);
    assert!(needs_off_confirm(&r));
    assert!(!needs_off_confirm(&Device::new(2, Kind::Antenna, "A", [0.0, 0.0])));
}

#[test]
fn a_feedline_on_a_transmitter_is_in_the_tx_path_and_a_loose_one_is_not() {
    let mut bay = PatchBay::default();
    bay.sync_radios(&[("IC-9700".into(), true)]);
    let radio = bay.devices[0].id;
    let feed = bay.add(Kind::Rf, "Feed", [200.0, 0.0]);
    let loose = bay.add(Kind::Rf, "Spare", [200.0, 200.0]);
    let ant = bay.add(Kind::Antenna, "Ant", [400.0, 0.0]);
    for d in &mut bay.devices {
        if d.id == feed || d.id == loose {
            d.rf = "feedline".into();
        }
    }
    bay.connect(&Jack::new(radio, "ANT1"), &Jack::new(feed, "IN")).unwrap();
    bay.connect(&Jack::new(feed, "OUT"), &Jack::new(ant, "FEED")).unwrap();
    assert!(in_tx_path(&bay, feed));
    assert!(in_tx_path(&bay, ant));
    assert!(!in_tx_path(&bay, loose));
}

#[test]
fn lockout_is_engaged_before_the_sweep_and_refuses_ptt() {
    let mut l = TxLockout::default();
    assert!(arm_sweep(&mut l, true, "Feed", false).is_err());
    assert!(!l.engaged(), "no VNA: nothing armed");
    arm_sweep(&mut l, true, "Feed", true).unwrap();
    assert!(l.holds(LOCK_KEY));
    assert_eq!(l.refuse("PTT").unwrap_err(), "PTT refused. TX LOCKOUT: VNA scan in progress on Feed");
    let mut free = TxLockout::default();
    arm_sweep(&mut free, false, "Filter", true).unwrap();
    assert!(!free.engaged());
}

#[test]
fn lockout_releases_on_finish_or_cancel_and_holds_on_disconnect() {
    let mut l = TxLockout::default();
    arm_sweep(&mut l, true, "Feed", true).unwrap();
    assert_eq!(poll(5, true, 4, true), Poll::Wait);
    assert_eq!(poll(5, true, 5, true), Poll::Done);
    assert!(l.release(LOCK_KEY));
    assert!(!l.engaged());
    arm_sweep(&mut l, true, "Feed", true).unwrap();
    assert_eq!(poll(5, false, 4, false), Poll::Lost);
    l.make_manual(LOCK_KEY, "state unknown");
    assert!(!l.release(LOCK_KEY), "a cancel does not clear an unknown state");
    assert!(l.refuse("PTT").is_err());
    l.release_by_user(LOCK_KEY);
    assert!(l.refuse("PTT").is_ok());
}

#[test]
fn triple_click_opens_the_vna_and_double_click_only_selects() {
    assert_eq!(box_click(true, false, false), BoxClick::Select);
    assert_eq!(box_click(false, true, false), BoxClick::Select);
    assert_eq!(box_click(false, false, true), BoxClick::OpenVna);
    assert_eq!(box_click(false, false, false), BoxClick::None);
}

#[test]
fn vna_folder_is_derived_from_the_station_name() {
    let st = Path::new("cfg/SAFE-T.HSCommander");
    assert_eq!(vna_dir(st), Path::new("cfg/SAFE-T.HSData/VNA"));
    assert_eq!(device_dir(st, "abc123").unwrap(), Path::new("cfg/SAFE-T.HSData/VNA/abc123"));
    assert!(device_dir(st, "../x").is_err());
    assert!(device_dir(st, "").is_err());
}

fn sweep() -> Vec<Sample> {
    // |Γ| dips to 0 at 146 MHz: SWR 1.0 there, under 2 for 145–147 MHz.
    (0..=40)
        .map(|i| {
            let hz = 140e6 + i as f64 * 0.25e6 * 1.2;
            let off = ((hz - 146e6) / 1e6).abs();
            let g = (off / 3.0).min(0.95); // |Γ| = 1/3 (SWR 2) at ±1 MHz
            Sample { hz, s11: Cx::new(g, 0.0), s21: Cx::new(1.0 - g, 0.0) }
        })
        .collect()
}

fn set(at: &str, uid: &str) -> ScanSet {
    let raw = stored(&sweep(), "dev1").unwrap();
    ScanSet {
        format: FORMAT.into(),
        version: VERSION,
        device_uid: uid.into(),
        device_name: "Yagi".into(),
        device_kind: "antenna".into(),
        ports: names(&["FEED"]),
        taken_at: at.into(),
        start_hz: 140e6,
        stop_hz: 152e6,
        points: 41,
        vna_id: "dev1".into(),
        cal_note: "UNCALIBRATED".into(),
        by: "test".into(),
        note: String::new(),
        length_m: 0.0,
        vf: 0.0,
        steps: vec![StepResult { step: step("FEED", None, &[]), raw, corrected: None }],
    }
}

#[test]
fn summary_finds_resonance_min_swr_and_the_two_to_one_span() {
    let st = s11_stats(&sweep()).unwrap();
    assert!((st.at_hz - 146e6).abs() < 0.2e6);
    assert!(st.min_swr < 1.05);
    let (lo, hi) = st.under2.unwrap();
    assert!((lo - 145.0e6).abs() < 0.31e6 && (hi - 147.0e6).abs() < 0.31e6, "{lo} {hi}");
    let line = &step_summary(&set("2026-10-27T10:00:00Z", "u1").steps[0])[0];
    assert!(line.contains("UNCALIBRATED") && line.contains("SWR<2"), "{line}");
}

#[test]
fn files_round_trip_never_overwrite_and_list_newest_first() {
    let dir = std::env::temp_dir().join(format!("hsvna-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let st = dir.join("City.HSCommander");
    let a = save_new(&st, &set("2026-10-27T10:00:00Z", "u1")).unwrap();
    let b = save_new(&st, &set("2026-10-27T10:00:00Z", "u1")).unwrap();
    assert_ne!(a, b, "same second gets a new name");
    assert!(b.to_string_lossy().ends_with("-2.HSVNAScan"));
    save_new(&st, &set("2026-10-28T09:00:00Z", "u1")).unwrap();
    let text = fs::read_to_string(&a).unwrap();
    assert!(!text.contains(dir.to_string_lossy().as_ref()), "no absolute paths in the file");
    // Same data back (text form: JSON floats can differ in the last bit).
    let back = parse(&text).unwrap();
    assert_eq!(serde_json::to_string_pretty(&back).unwrap() + "\n", text);
    assert_eq!(back.steps.len(), 1);
    assert_eq!(back.device_uid, "u1");
    let h = history(&st, "u1").unwrap();
    assert_eq!(h.len(), 3);
    assert_eq!(h[0].set.as_ref().unwrap().taken_at, "2026-10-28T09:00:00Z");
    // A newer version is refused and left alone.
    let newer = text.replace("\"version\": 1", "\"version\": 2");
    let p = device_dir(&st, "u1").unwrap().join("z.HSVNAScan");
    fs::write(&p, &newer).unwrap();
    let h = history(&st, "u1").unwrap();
    assert!(h.last().unwrap().set.as_ref().unwrap_err().contains("newer Commander"));
    assert_eq!(fs::read_to_string(&p).unwrap(), newer);
    assert!(delete_scan(&st, "u1", "../x.HSVNAScan").is_err());
    delete_scan(&st, "u1", "z.HSVNAScan").unwrap();
    assert!(history(&st, "other").unwrap().is_empty());
    // Restart marker.
    write_marker(&st, "Feed").unwrap();
    assert_eq!(read_marker(&st).as_deref(), Some("Feed"));
    clear_marker(&st);
    assert!(read_marker(&st).is_none());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn device_records_round_trip_with_uid_and_rf_kind() {
    let mut d = Device::new(3, Kind::Rf, "Dup", [1.0, 2.0]);
    d.rf = "duplexer".into();
    d.notes = "tuned 146.94/146.34".into();
    let t = serde_json::to_string(&d).unwrap();
    let back: Device = serde_json::from_str(&t).unwrap();
    assert_eq!(back, d);
    assert!(!back.uid.is_empty());
    // Older devices without uid load and get one at prune.
    let mut bay = PatchBay::default();
    bay.devices.push(serde_json::from_str(r#"{"id":1,"kind":"antenna","name":"A","pos":[0,0]}"#).unwrap());
    assert!(bay.devices[0].uid.is_empty());
    bay.prune();
    assert!(!bay.devices[0].uid.is_empty());
}

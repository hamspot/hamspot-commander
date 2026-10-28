//! RF device kinds for the patch panel (#219), read from `rf_kinds.json`.
//! A new kind is a new table row: ports (name, true = left/input side),
//! scan plan, transmit through-paths and whether it can transmit. All ports
//! are 50 Ω coax. `custom` takes its ports from the device's port count.

use super::PlanKind;
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KindDef {
    pub key: String,
    pub label: String,
    pub ports: Vec<(String, bool)>,
    pub plan: String,
    pub through: Vec<(String, String)>,
    #[serde(default)]
    pub tx: bool,
    #[serde(default)]
    pub note: String,
}

pub const CUSTOM: &str = "custom";
pub const Z0_OHMS: f64 = 50.0;

pub fn table() -> &'static [KindDef] {
    static T: OnceLock<Vec<KindDef>> = OnceLock::new();
    T.get_or_init(|| serde_json::from_str(include_str!("rf_kinds.json")).expect("rf_kinds.json"))
}

pub fn def(key: &str) -> Option<&'static KindDef> {
    table().iter().find(|d| d.key == key)
}

pub fn label_of(key: &str) -> String {
    def(key).map(|d| d.label.clone()).unwrap_or_else(|| format!("Unknown RF kind {key:?}"))
}

pub fn ports_of(key: &str, n: u8) -> Vec<(String, bool)> {
    if key == CUSTOM {
        let n = if n == 0 { 2 } else { n.clamp(1, 16) };
        return (1..=n).map(|i| (format!("P{i}"), i == 1)).collect();
    }
    def(key).map(|d| d.ports.clone()).unwrap_or_default()
}

pub fn through_of(key: &str, input: &str) -> Vec<String> {
    def(key).map(|d| d.through.iter().filter(|(a, _)| a == input).map(|(_, b)| b.clone()).collect()).unwrap_or_default()
}

pub fn plan_of(key: &str) -> PlanKind {
    def(key).map(|d| PlanKind::parse(&d.plan)).unwrap_or(PlanKind::AllPairs)
}

pub fn is_tx(key: &str) -> bool {
    def(key).is_some_and(|d| d.tx)
}

pub fn note_of(key: &str) -> &'static str {
    def(key).map(|d| d.note.as_str()).unwrap_or("")
}

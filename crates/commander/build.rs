//! Copy band plans next to the built binary, under `bsp/`.
//!
//! `OUT_DIR` is `target/<profile>/build/<pkg>/out`, so three parents up is
//! the directory that holds `commander`.
//!
//! A local build also installs p25voice when this machine opts in. The
//! default is off. `HAMSPOT_WITH_P25VOICE=1` or `yes` turns it on, and `0`
//! or `no` turns it off. Otherwise the marker file `with-p25voice` in the
//! Hamspot Commander config directory turns it on.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let source = manifest.join("../../bsp");
    println!("cargo:rerun-if-changed={}", source.display());

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let dest = out.join("../../../bsp");
    fs::create_dir_all(&dest)
        .unwrap_or_else(|err| panic!("could not create {}: {err}", dest.display()));

    let entries = fs::read_dir(&source)
        .unwrap_or_else(|err| panic!("could not read {}: {err}", source.display()));
    let mut copied = 0;
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("could not read a band plan: {err}"));
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        println!("cargo:rerun-if-changed={}", path.display());
        let to = dest.join(entry.file_name());
        fs::copy(&path, &to).unwrap_or_else(|err| {
            panic!(
                "could not copy {} to {}: {err}",
                path.display(),
                to.display()
            )
        });
        copied += 1;
    }
    if copied == 0 {
        panic!("no band plans in {}", source.display());
    }

    install_p25voice_if_opted_in(&manifest);
}

/// Marker path even when the file is absent, so creating it reruns this script.
fn p25voice_marker() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
        })?;
    Some(config.join("hamspot-commander").join("with-p25voice"))
}

fn p25voice_opted_in(marker: &Path) -> bool {
    if let Ok(value) = std::env::var("HAMSPOT_WITH_P25VOICE") {
        return match value.trim().to_ascii_lowercase().as_str() {
            "1" | "yes" | "true" => true,
            "0" | "no" | "false" | "" => false,
            other => panic!("HAMSPOT_WITH_P25VOICE must be 1, yes, 0, or no (got {other})"),
        };
    }
    marker.is_file()
}

fn install_p25voice_if_opted_in(manifest_dir: &Path) {
    println!("cargo:rerun-if-env-changed=HAMSPOT_WITH_P25VOICE");
    println!("cargo:rerun-if-env-changed=P25VOICE_SRC");
    println!("cargo:rerun-if-env-changed=XDG_CONFIG_HOME");
    println!("cargo:rerun-if-env-changed=HOME");

    let marker = p25voice_marker();
    if let Some(path) = &marker {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let script = manifest_dir.join("../../scripts/install-p25voice.sh");
    println!("cargo:rerun-if-changed={}", script.display());

    let opted_in = marker
        .as_deref()
        .is_some_and(|path| p25voice_opted_in(path));
    if !opted_in {
        return;
    }
    if !script.is_file() {
        panic!("p25voice is opted in, but {} is missing", script.display());
    }
    let status = Command::new(&script).status().unwrap_or_else(|err| {
        panic!("could not run {}: {err}", script.display())
    });
    if !status.success() {
        panic!("p25voice install failed ({status})");
    }
}

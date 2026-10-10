//! Copy band plans next to the built binary, under `bsp/`.
//!
//! `OUT_DIR` is `target/<profile>/build/<pkg>/out`, so three parents up is
//! the directory that holds `commander`.

use std::fs;
use std::path::PathBuf;

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
}

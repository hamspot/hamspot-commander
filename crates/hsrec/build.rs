//! Stamp the git commit into recordings (sw_version), best effort.
fn main() {
    let sha = std::process::Command::new("git")
        .args(["rev-parse", "--short=10", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=HSREC_GIT_SHA={sha}");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
}

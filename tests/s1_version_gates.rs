//! S1b gate: `klang --version` reports the real release version.
//!
//! The binary prints `env!("CARGO_PKG_VERSION")`; this pins that code path
//! so the release-workflow stamping (tag -> Cargo.toml before build) is
//! covered without needing a real release run.

#[test]
fn s1_version_matches_cargo_pkg_version() {
    let bin = env!("CARGO_BIN_EXE_klang");
    let out = std::process::Command::new(bin)
        .arg("--version")
        .output()
        .expect("klang --version runs");
    assert!(out.status.success(), "exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(
        stdout,
        format!("klang {}", env!("CARGO_PKG_VERSION")),
        "printed version must equal CARGO_PKG_VERSION"
    );
    // Short flag behaves the same.
    let out2 = std::process::Command::new(bin)
        .arg("-V")
        .output()
        .expect("klang -V runs");
    assert!(out2.status.success());
    let stdout2 = String::from_utf8_lossy(&out2.stdout).trim().to_string();
    assert_eq!(stdout2, stdout);
}

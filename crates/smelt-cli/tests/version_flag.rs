//! Phase 5 of `docs/plans/20260502-smelt-loop-findings.md` (TB-4):
//! `smelt --version` must print the package version and exit 0.
//! Spec: `docs/specs/cli.md` §"`smelt --version`".

use std::path::PathBuf;
use std::process::Command;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

#[test]
fn test_version_long_flag_prints_package_version() {
    let output = Command::new(smelt_bin())
        .arg("--version")
        .output()
        .expect("smelt binary should be runnable");

    assert!(
        output.status.success(),
        "smelt --version exit code: {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected_version = env!("CARGO_PKG_VERSION");
    assert!(
        stdout.contains(expected_version),
        "expected stdout to contain package version `{expected_version}`, got: {stdout}"
    );
}

#[test]
fn test_version_short_flag_prints_package_version() {
    let output = Command::new(smelt_bin())
        .arg("-V")
        .output()
        .expect("smelt binary should be runnable");

    assert!(output.status.success(), "smelt -V should exit 0");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected_version = env!("CARGO_PKG_VERSION");
    assert!(
        stdout.contains(expected_version),
        "expected stdout to contain `{expected_version}`, got: {stdout}"
    );
}

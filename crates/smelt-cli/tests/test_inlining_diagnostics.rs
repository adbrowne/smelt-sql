#![cfg(feature = "duckdb")]
//! Integration tests for Phase 5: anchored `AmbiguousTestModel` and
//! `NonStandaloneTestModel` diagnostics emitted during `smelt.test` whole-query
//! inlining.

use std::path::{Path, PathBuf};
use std::process::Command;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
}

fn run_smelt_test(project_dir: &Path) -> std::process::Output {
    Command::new(smelt_bin())
        .arg("test")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt test`: {e}"))
}

fn run_smelt_test_show_all(project_dir: &Path) -> std::process::Output {
    Command::new(smelt_bin())
        .arg("test")
        .arg("--show-all")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt test`: {e}"))
}

/// A `smelt.test` whose body contains a single-segment `smelt.<leaf>` ref where
/// two models share that leaf name must fail with an anchored `AmbiguousTestModel`
/// diagnostic (code in brackets), naming both candidates and advising the full
/// dotted address.
///
/// Fixture: `examples/test_inlining_ambiguous/` — two models `a/users` and
/// `b/users` share the leaf "users"; the test body contains `FROM smelt.users`.
#[test]
fn ambiguous_single_segment_ref_diagnoses() {
    let dir = examples_dir().join("test_inlining_ambiguous");
    let output = run_smelt_test(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !output.status.success(),
        "smelt test must exit non-zero when a single-segment ref is ambiguous;\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // The diagnostic must be emitted in bracketed code format, e.g.
    // `error[AmbiguousTestModel]:` — this distinguishes the anchored diagnostic
    // from the former raw-string `AmbiguousTestModel: ...` error.
    assert!(
        combined.contains("error[AmbiguousTestModel]"),
        "output must contain 'error[AmbiguousTestModel]' (anchored diagnostic format);\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // The two candidates must be listed.
    assert!(
        combined.contains("a.users") && combined.contains("b.users"),
        "output must list both candidates (a.users, b.users);\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // Must advise using a full dotted address.
    assert!(
        combined.contains("full") || combined.contains("dotted") || combined.contains("address"),
        "output must advise using the full dotted address;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // Must be anchored: output must contain a file path to the test file.
    assert!(
        combined.contains("test_ambiguous.sql"),
        "output must name the test file for anchoring;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// A `smelt.test` whole-query test that inlines an upstream model whose body
/// contains `smelt.config.var(...)` (a per-model construct requiring a build
/// context) and does NOT mock it via `PASSING` must fail with an anchored
/// `NonStandaloneTestModel` diagnostic advising a `PASSING` mock.
///
/// Fixture: `examples/test_inlining_non_standalone/` — `upstream` uses
/// `smelt.config.var('region')`; `tests/test_no_mock.sql` references it
/// without mocking.
#[test]
fn non_standalone_upstream_diagnoses() {
    let dir = examples_dir().join("test_inlining_non_standalone");
    let output = run_smelt_test(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !output.status.success(),
        "smelt test must exit non-zero when an inlined upstream is non-standalone;\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        combined.contains("error[NonStandaloneTestModel]"),
        "output must contain 'error[NonStandaloneTestModel]' (anchored diagnostic format);\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // Must name the offending ref.
    assert!(
        combined.contains("upstream"),
        "output must name the offending upstream ref;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // Must advise PASSING.
    assert!(
        combined.contains("PASSING"),
        "output must advise mocking via PASSING;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // Must be anchored: output must contain the test file name.
    assert!(
        combined.contains("test_no_mock.sql"),
        "output must name the test file for anchoring;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// Same workspace as `non_standalone_upstream_diagnoses` but the `test_with_mock`
/// test mocks `upstream` via `PASSING`. That test must report PASS even though the
/// no-mock test in the same workspace fails.
///
/// Verifies the hint in `NonStandaloneTestModel` is actionable: mocking the dep
/// resolves the problem.
#[test]
fn mocking_the_dep_resolves_non_standalone() {
    let dir = examples_dir().join("test_inlining_non_standalone");
    let output = run_smelt_test_show_all(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // The test with mock must appear as PASS.
    assert!(
        stdout.contains("PASS") && stdout.contains("upstream_with_mock"),
        "test 'upstream_with_mock' (which mocks upstream via PASSING) must show PASS;\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

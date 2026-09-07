use std::fs;
use std::process::Command;

use crate::{repo_root, script_path};

/// Test-file blind spot (`docs/outcomes/20260906-scd2-keyed-succession/
/// phases/03b-plan.md`): a `#[cfg(test)] mod tests { .. }` block split into
/// its own file (`src/m/mod.rs` declaring `#[cfg(test)] mod helper_tests;`)
/// is test-only even though nothing inside `helper_tests.rs` itself carries
/// the attribute — the existing `tests.rs`/`tests/`-directory name heuristic
/// cannot see it. `mod.rs`'s plain `mod real;` declaration keeps `real.rs`
/// counted as production.
#[test]
fn cfg_test_declared_module_files_are_not_counted() {
    let tempdir = tempfile::tempdir().unwrap();
    let fake_root = tempdir.path();

    let write = |rel: &str, body: &str| {
        let path = fake_root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    };

    write(
        "crates/declared-module-probe/Cargo.toml",
        "[package]\nname = \"declared-module-probe\"\n",
    );
    write("crates/declared-module-probe/src/lib.rs", "mod m;\n");
    write(
        "crates/declared-module-probe/src/m/mod.rs",
        "#[cfg(test)]\nmod helper_tests;\nmod real;\n",
    );
    write(
        "crates/declared-module-probe/src/m/helper_tests.rs",
        "pub fn probe() -> i32 { let x: Option<i32> = None; x.unwrap() }\n",
    );
    write(
        "crates/declared-module-probe/src/m/real.rs",
        "pub fn probe() -> i32 { let x: Option<i32> = None; x.unwrap() }\n",
    );

    // If `helper_tests.rs` were (wrongly) counted, the true count would be 2
    // and this baseline of 1 would register as a REGRESSION. If it correctly
    // stays invisible, the tree matches baseline (only `real.rs`'s unwrap
    // counts) and the gate exits 0.
    write(
        ".claude/hardening-baseline.txt",
        "declared-module-probe unwrap 1\ndeclared-module-probe expect 0\n\
         declared-module-probe println 0\n",
    );

    let output = Command::new("bash")
        .arg(script_path())
        .env("REPO_ROOT", fake_root)
        .current_dir(repo_root())
        .output()
        .expect("failed to run hardening-budget.sh on fake tree");

    assert!(
        output.status.success(),
        "gate should exclude helper_tests.rs (declared #[cfg(test)] in mod.rs) and count only \
         real.rs's unwrap, but failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

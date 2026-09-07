use std::fs;
use std::process::Command;

use crate::{repo_root, script_path};

/// A crate used only as a `[dev-dependencies]` entry is test-support, not
/// production, and its `unwrap`/`expect` debt must not enter the baseline.
///
/// The rule is *derived*, not declared: a crate counts as test-support only
/// when some crate names it under `[dev-dependencies]` AND no crate names it
/// under `[dependencies]`. That shape matters — "nothing depends on it" alone
/// would wrongly exclude every top-level binary crate (`smelt-cli`,
/// `smelt-ui`, …), which nothing depends on either. Deriving it means that
/// promoting a testkit to a real dependency silently re-enters its debt into
/// the gate, where a hand-maintained exclusion list would have gone stale.
#[test]
fn dev_only_crates_are_excluded_from_the_budget() {
    let tempdir = tempfile::tempdir().unwrap();
    let fake_root = tempdir.path();

    let write = |rel: &str, body: &str| {
        let path = fake_root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    };

    // `app` — a top-level crate nothing depends on. Still production.
    write(
        "crates/app/Cargo.toml",
        "[package]\nname = \"app\"\n\n[dependencies]\nprod-lib = { path = \"../prod-lib\" }\n\n\
         [dev-dependencies]\ntest-kit = { path = \"../test-kit\" }\n",
    );
    write("crates/app/src/lib.rs", "pub fn ok() {}\n");

    // `prod-lib` — a real dependency of `app`. Its one unwrap is production debt.
    write(
        "crates/prod-lib/Cargo.toml",
        "[package]\nname = \"prod-lib\"\n",
    );
    write(
        "crates/prod-lib/src/lib.rs",
        "pub fn probe() -> i32 { let x: Option<i32> = None; x.unwrap() }\n",
    );

    // `tool` — a shipped binary that some crate dev-depends on via a test-only
    // back edge (`smelt-runtime` does exactly this to `smelt-cli`). Nothing
    // depends on it normally, so the dev-dep half of the rule alone would
    // wrongly excuse it. A crate with a binary target ships to users and stays
    // production; its debt must still be counted.
    write(
        "crates/tool/Cargo.toml",
        "[package]\nname = \"tool\"\n\n[dev-dependencies]\ntest-kit = { path = \"../test-kit\" }\n",
    );
    write(
        "crates/tool/src/main.rs",
        "fn main() { println!(\"hi\"); }\n",
    );
    write(
        "crates/tool/src/lib.rs",
        "pub fn probe() -> i32 { let x: Option<i32> = None; x.unwrap() }\n",
    );
    write(
        "crates/app-that-dev-deps-tool/Cargo.toml",
        "[package]\nname = \"app-that-dev-deps-tool\"\n\n\
         [dev-dependencies]\ntool = { path = \"../tool\" }\n",
    );
    write(
        "crates/app-that-dev-deps-tool/src/lib.rs",
        "pub fn ok() {}\n",
    );

    // `test-kit` — dev-dependency of `app`, regular dependency of nobody,
    // and no binary target. Its debt must be invisible to the gate.
    write(
        "crates/test-kit/Cargo.toml",
        "[package]\nname = \"test-kit\"\n",
    );
    write(
        "crates/test-kit/src/lib.rs",
        "pub fn a() -> i32 { let x: Option<i32> = None; x.unwrap() }\n\
         pub fn b() -> i32 { let x: Option<i32> = None; x.unwrap() }\n\
         pub fn c() -> i32 { let x: Option<i32> = None; x.expect(\"c\") }\n",
    );

    // Baseline registers every production crate — `tool` included, with the
    // debt its binary target keeps in scope — but not `test-kit`. The gate is
    // a two-sided ratchet, so an over-broad exclusion shows up here as a
    // "unregistered crate" error and an under-broad one as a stale-baseline
    // error; either way this assertion fails.
    write(
        ".claude/hardening-baseline.txt",
        "app unwrap 0\napp expect 0\napp println 0\n\
         app-that-dev-deps-tool unwrap 0\napp-that-dev-deps-tool expect 0\n\
         app-that-dev-deps-tool println 0\n\
         prod-lib unwrap 1\nprod-lib expect 0\nprod-lib println 0\n\
         tool unwrap 1\ntool expect 0\ntool println 1\n",
    );

    let output = Command::new("bash")
        .arg(script_path())
        .env("REPO_ROOT", fake_root)
        .current_dir(repo_root())
        .output()
        .expect("failed to run hardening-budget.sh on fake tree");

    assert!(
        output.status.success(),
        "gate should ignore the dev-only `test-kit` crate, but failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

//! Fail-loud regression for the masked `table_exists` check
//! (`docs/outcomes/20260913-trino-incremental/phases/06d-plan.md`): a
//! `BackendError` from `Backend::table_exists` must never be silently turned
//! into "the table does not exist" — that shape re-runs a first-run
//! `CREATE TABLE … AS` over a target the driver never actually checked,
//! which is exactly the live-Trino conditional-write parity failure this
//! phase measured (a genuine backend error from a freshly constructed
//! `TrinoBackend`'s `table_exists` query, masked to `false`, made run 2
//! retake the bootstrap-create route instead of the membership recompute).
//!
//! Source-scan census over all of `src/`: no `table_exists(…).unwrap_or(`
//! call site survives, so the class cannot regress silently.

#[test]
fn no_table_exists_call_swallows_its_error() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offending = Vec::new();
    visit(&root, &mut offending);
    assert!(
        offending.is_empty(),
        "table_exists(...) results must propagate their error rather than collapsing to \
         `false` via unwrap_or — found:\n  {}",
        offending.join("\n  ")
    );
}

fn visit(dir: &std::path::Path, offending: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            visit(&path, offending);
            continue;
        }
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read source");
        let rel = path
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap_or(&path)
            .display()
            .to_string();
        for (i, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with("///") {
                continue;
            }
            if line.contains("table_exists(") && line.contains("unwrap_or(false)") {
                offending.push(format!("{rel}:{}: {line}", i + 1));
                continue;
            }
            // The three-line-split spelling (`.table_exists(...)` /
            // `.await` / `.unwrap_or(false);`) used throughout
            // `execute/project/mod.rs` and `cumulative.rs`.
            if trimmed == ".unwrap_or(false);"
                && i >= 2
                && text
                    .lines()
                    .nth(i - 2)
                    .is_some_and(|l| l.contains("table_exists("))
            {
                offending.push(format!("{rel}:{}: {line}", i + 1));
            }
        }
    }
}

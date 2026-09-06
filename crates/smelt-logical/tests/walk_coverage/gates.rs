use std::fs;
use std::path::PathBuf;

use super::classify::{load_production_source, unclassified_raw_scans};
use super::repo_root;
use super::scan::scanned_files;

/// Regression lock for the test-file blind spot: `maintenance/choice/tests.rs`
/// and `maintenance/choice/write_suppression_tests.rs` are both declared
/// `#[cfg(test)] mod <stem>;` in `maintenance/choice/mod.rs`, so both must be
/// excluded from the scanned set, while `maintenance/choice/mod.rs` itself
/// must still be included.
#[test]
fn gate_scans_production_choice_sources() {
    let root = repo_root();
    let files = scanned_files(&root);
    let choice_tests = PathBuf::from("crates/smelt-logical/src/maintenance/choice/tests.rs");
    let write_suppression_tests =
        PathBuf::from("crates/smelt-logical/src/maintenance/choice/write_suppression_tests.rs");
    let choice_mod = PathBuf::from("crates/smelt-logical/src/maintenance/choice/mod.rs");
    assert!(
        !files.contains(&choice_tests),
        "expected {} to be excluded as test-only, got it in the scanned set",
        choice_tests.display()
    );
    assert!(
        !files.contains(&write_suppression_tests),
        "expected {} to be excluded as test-only, got it in the scanned set",
        write_suppression_tests.display()
    );
    assert!(
        files.contains(&choice_mod),
        "expected {} to remain in the scanned set",
        choice_mod.display()
    );
}

/// The committed tree carries no unclassified raw text-scan in the
/// admission/proof surface.
#[test]
fn admission_paths_have_no_raw_text_scans() {
    let root = repo_root();
    let files = scanned_files(&root);
    assert!(
        !files.is_empty(),
        "expected at least one *.rs file under the scanned dirs"
    );
    let mut all_violations = Vec::new();
    for rel in &files {
        let path = root.join(rel);
        assert!(path.exists(), "{} not found under {root:?}", rel.display());
        for (line_no, text) in unclassified_raw_scans(&path) {
            all_violations.push(format!("{}:{line_no}: {text}", rel.display()));
        }
    }
    assert!(
        all_violations.is_empty(),
        "unclassified raw text-scan(s) in the admission/proof surface — each \
         `.contains(\"…\")` on case-folded free text must be tagged `Leaf \
         classifier` or `Advisory heuristic` in the enclosing function's doc \
         comment (or the file's module-level `//!` doc), per \
         `docs/specs/architecture.md` §\"Property composition walk rule\":\n{}",
        all_violations.join("\n")
    );
}

/// Regression guard for the gate itself: an injected raw scan with no
/// classification tag, in a synthetic fixture file mirroring this crate's
/// style, is detected. Deterministic and self-contained — no network, no
/// dependence on the real source tree beyond `SCANNED_FILES` existing.
#[test]
fn detects_an_unclassified_injected_raw_scan() {
    let dir =
        std::env::temp_dir().join(format!("smelt-walk-coverage-probe-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp probe dir");
    let probe = dir.join("probe.rs");
    fs::write(
        &probe,
        "fn ok() -> bool {\n\
         \x20   // Leaf classifier for the test.\n\
         \x20   \"x\".to_uppercase().contains(\"UNBOUNDED\")\n\
         }\n\n\
         fn not_ok() -> bool {\n\
         \x20   let upper = \"x\".to_uppercase();\n\
         \x20   upper.contains(\"UNBOUNDED\")\n\
         }\n",
    )
    .expect("write probe fixture");

    // Note: `ok()`'s inline `//` comment is not a `///` doc comment, so it
    // does not classify `ok()` either — both functions should be flagged.
    // This asserts the *unclassified* half of that: `not_ok()` is caught.
    let violations = unclassified_raw_scans(&probe);
    assert!(
        violations
            .iter()
            .any(|(_, text)| text.contains("not_ok")
                || text.contains("upper.contains(\"UNBOUNDED\")")),
        "expected the injected raw scan in `not_ok` to be flagged, got: {violations:?}"
    );
    assert_eq!(
        violations.len(),
        2,
        "expected both unclassified scans (`//` is not `///`) to be flagged, got: {violations:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Regression lock for the "first-`#[cfg(test)]`-truncates-the-rest" blind
/// spot (review finding on `docs/plans/20260808-substrate-unification.md`
/// Phase 6): `maintenance/propagate.rs`'s real shape is a
/// `#[cfg(test)] mod day_interval_tests { .. }` block, then ~450 lines of
/// production code (`normalize` and friends), then two more test modules.
/// `load_production_source` must scan the production span that sits
/// *between* two test spans, not just "everything before the first
/// `#[cfg(test)]` line".
#[test]
fn load_production_source_scans_production_code_interleaved_between_cfg_test_blocks() {
    let dir = std::env::temp_dir().join(format!(
        "smelt-walk-coverage-probe-interleaved-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp probe dir");
    let probe = dir.join("propagate_shape.rs");
    fs::write(
        &probe,
        "#[cfg(test)]\n\
         mod day_interval_tests {\n\
         \x20   #[test]\n\
         \x20   fn probe() {\n\
         \x20       assert!(true);\n\
         \x20   }\n\
         }\n\
         \n\
         fn normalize(x: &str) -> bool {\n\
         \x20   x.to_lowercase().contains(\"group by\")\n\
         }\n\
         \n\
         #[cfg(test)]\n\
         mod later_tests {\n\
         \x20   #[test]\n\
         \x20   fn probe2() {\n\
         \x20       assert!(true);\n\
         \x20   }\n\
         }\n",
    )
    .expect("write probe fixture");

    let source = load_production_source(&probe);
    let joined = source.lines.join("\n");

    assert!(
        joined.contains("fn normalize"),
        "expected the production fn sandwiched between two cfg(test) blocks to be \
         scanned, got:\n{joined}"
    );
    assert!(
        joined.contains("x.to_lowercase().contains(\"group by\")"),
        "expected the production scan site itself to survive exclusion, got:\n{joined}"
    );
    assert!(
        !joined.contains("day_interval_tests"),
        "expected the first cfg(test) block's own content to be excluded, got:\n{joined}"
    );
    assert!(
        !joined.contains("later_tests"),
        "expected the second cfg(test) block's own content to be excluded, got:\n{joined}"
    );
    assert!(
        !joined.contains("assert!(true)"),
        "expected every test-module body line to be excluded, got:\n{joined}"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// End-to-end twin of the span test above: the full gate
/// (`unclassified_raw_scans`) must actually flag the unclassified scan
/// sitting in the interleaved production span, and must not flag anything
/// inside either surrounding test module.
#[test]
fn gate_detects_a_violation_interleaved_between_two_cfg_test_blocks() {
    let dir = std::env::temp_dir().join(format!(
        "smelt-walk-coverage-probe-interleaved-gate-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp probe dir");
    let probe = dir.join("propagate_shape_gate.rs");
    fs::write(
        &probe,
        "#[cfg(test)]\n\
         mod day_interval_tests {\n\
         \x20   #[test]\n\
         \x20   fn probe() {\n\
         \x20       assert!(\"y\".to_lowercase().contains(\"should not be flagged\"));\n\
         \x20   }\n\
         }\n\
         \n\
         fn normalize(x: &str) -> bool {\n\
         \x20   x.to_lowercase().contains(\"group by\")\n\
         }\n\
         \n\
         #[cfg(test)]\n\
         mod later_tests {\n\
         \x20   #[test]\n\
         \x20   fn probe2() {\n\
         \x20       assert!(\"z\".to_lowercase().contains(\"also not flagged\"));\n\
         \x20   }\n\
         }\n",
    )
    .expect("write probe fixture");

    let violations = unclassified_raw_scans(&probe);
    assert_eq!(
        violations.len(),
        1,
        "expected exactly one violation (the interleaved production scan), got: {violations:?}"
    );
    assert!(
        violations[0]
            .1
            .contains("x.to_lowercase().contains(\"group by\")"),
        "expected the flagged line to be the production scan, got: {violations:?}"
    );
    // The flagged line number must point at the actual production line in
    // the file, not a shifted index from dropping the preceding test span.
    let expected_line_no = std::fs::read_to_string(&probe)
        .expect("reread probe")
        .lines()
        .position(|l| l.contains("x.to_lowercase().contains(\"group by\")"))
        .map(|idx| idx + 1)
        .expect("scan line present in probe");
    assert_eq!(violations[0].0, expected_line_no);

    let _ = fs::remove_dir_all(&dir);
}

/// `rules/cumulative.rs` is no longer parked in `KNOWN_NONCOMPLIANT` — the
/// keyed-admission whole-SQL scans it carried (`docs/outcomes/
/// 20260904-walk-migration-residue/phases/04-plan.md`) migrated onto the
/// walk. Regression lock: a future re-addition of the skip-list entry (e.g.
/// to hide a reintroduced scan) makes this fail.
#[test]
fn cumulative_rs_is_covered_by_the_gate() {
    let root = repo_root();
    let files = scanned_files(&root);
    let rel = PathBuf::from("crates/smelt-logical/src/rules/cumulative.rs");
    assert!(
        files.contains(&rel),
        "expected {} to be scanned by the gate (not skip-listed), got: {files:?}",
        rel.display()
    );
}

/// The widened `is_raw_scan_line` catches the non-literal, case-folded-
/// variable scan form (`let upper = s.to_uppercase(); … upper.contains(&x)`)
/// a bare `.contains("` grep cannot see — and does not flag an ordinary
/// collection-membership `.contains(&x)` on a receiver that was never
/// case-folded.
#[test]
fn detects_an_unclassified_case_folded_variable_scan() {
    let dir = std::env::temp_dir().join(format!(
        "smelt-walk-coverage-probe-case-folded-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp probe dir");
    let probe = dir.join("case_folded_probe.rs");
    fs::write(
        &probe,
        "fn scans_case_folded(s: &str, pattern: &str) -> bool {\n\
         \x20   let upper = s.to_uppercase();\n\
         \x20   upper.contains(pattern)\n\
         }\n\n\
         fn checks_membership(names: &Vec<String>, name: &str) -> bool {\n\
         \x20   names.contains(&name.to_string())\n\
         }\n",
    )
    .expect("write probe fixture");

    let violations = unclassified_raw_scans(&probe);
    assert!(
        violations
            .iter()
            .any(|(_, text)| text.contains("upper.contains(pattern)")),
        "expected the case-folded-variable scan to be flagged, got: {violations:?}"
    );
    assert!(
        !violations
            .iter()
            .any(|(_, text)| text.contains("names.contains(&name.to_string())")),
        "expected the unrelated collection-membership check to survive, got: {violations:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

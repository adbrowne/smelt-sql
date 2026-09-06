use super::*;

// ─── Position ───────────────────────────────────────────────────────────────

/// `Position` has exactly the five documented variants, and `Any` is a
/// lookup wildcard no classifier ever returns — it exists only so a
/// registry entry can state one verdict that applies at every call
/// position. This match is exhaustive: adding or removing a variant is a
/// compile error here, forcing this test (and its doc comment) to be
/// updated alongside the enum.
#[test]
fn position_variants_are_exhaustive() {
    let variants = [
        Position::Any,
        Position::Scalar,
        Position::Aggregate,
        Position::WholePartitionWindow,
        Position::Window,
    ];
    for v in variants {
        match v {
            Position::Any
            | Position::Scalar
            | Position::Aggregate
            | Position::WholePartitionWindow
            | Position::Window => {}
        }
    }
    assert_eq!(
        variants.len(),
        5,
        "Position must have exactly five variants"
    );
}

/// A verdict stated at the call's own position wins over one stated only at
/// the `Any` wildcard for the same dialect — the wildcard is a default, not a
/// veto.
#[test]
fn emission_at_prefers_exact_position_over_any() {
    let sig = test_signature("TEST_PREFERS_EXACT").with_emission(&[
        (
            DialectId::BigQuery,
            Position::Any,
            Emission::Rename("ANY_SPELLING"),
        ),
        (
            DialectId::BigQuery,
            Position::Aggregate,
            Emission::Rename("AGG_SPELLING"),
        ),
    ]);
    assert_eq!(
        sig.emission_at(DialectId::BigQuery, Position::Aggregate),
        Emission::Rename("AGG_SPELLING"),
        "the exact-position entry must win over Any"
    );
    // A position with no entry of its own still falls through to Any.
    assert_eq!(
        sig.emission_at(DialectId::BigQuery, Position::Scalar),
        Emission::Rename("ANY_SPELLING"),
        "a position with no dedicated entry falls back to Any"
    );
}

/// Lookup falls from the exact position to `Any`, and from `Any` to
/// `Native` when the dialect has no entry at all — and stops there.
#[test]
fn emission_at_falls_back_to_any_then_native() {
    let sig = test_signature("TEST_FALLS_BACK").with_emission(&[(
        DialectId::BigQuery,
        Position::Any,
        Emission::Rename("X"),
    )]);
    assert_eq!(
        sig.emission_at(DialectId::BigQuery, Position::Scalar),
        Emission::Rename("X"),
        "no Scalar entry, but an Any entry exists for this dialect"
    );
    assert_eq!(
        sig.emission_at(DialectId::DuckDb, Position::Scalar),
        Emission::Native,
        "no entry at all for this dialect: Native"
    );
}

/// The finding that motivated position-scoped emission: the two window
/// positions must never answer for each other. Falling from
/// `WholePartitionWindow` to `Window` would refuse a whole-partition call
/// the restructure exists to serve; falling from `Window` to `Any` would let
/// a running window reach the backend as `Native` and fail at the warehouse.
#[test]
fn window_positions_never_fall_back_to_each_other() {
    let sig = test_signature("TEST_WINDOW_POSITIONS").with_emission(&[
        (
            DialectId::BigQuery,
            Position::WholePartitionWindow,
            Emission::Native,
        ),
        (
            DialectId::BigQuery,
            Position::Window,
            Emission::Unsupported {
                reason: "no analytic form for a running window",
            },
        ),
    ]);
    assert_eq!(
        sig.emission_at(DialectId::BigQuery, Position::WholePartitionWindow),
        Emission::Native,
        "the whole-partition verdict must not be shadowed by the running-window one"
    );
    assert!(
        matches!(
            sig.emission_at(DialectId::BigQuery, Position::Window),
            Emission::Unsupported { .. }
        ),
        "the running-window verdict must not be shadowed by the whole-partition one"
    );
}

/// Coverage-totality gate: an entry declaring a verdict at one window
/// position must declare one at the other, because there is no fallback
/// between them for the lookup to fall back on. Checked against the real
/// registry data, not a hypothetical signature, so the gate fires the moment
/// a real entry violates it.
#[test]
fn window_verdict_totality() {
    let mut violations: Vec<String> = Vec::new();
    for name in BuiltinRegistry::names() {
        let Some(sig) = BuiltinRegistry::resolve(name) else {
            continue;
        };
        for dialect in DialectId::ALL {
            let has_whole_partition = sig
                .emission
                .iter()
                .any(|(d, p, _)| *d == *dialect && *p == Position::WholePartitionWindow);
            let has_window = sig
                .emission
                .iter()
                .any(|(d, p, _)| *d == *dialect && *p == Position::Window);
            if has_whole_partition != has_window {
                let (present, missing) = if has_whole_partition {
                    ("WholePartitionWindow", "Window")
                } else {
                    ("Window", "WholePartitionWindow")
                };
                violations.push(format!(
                    "{name} on {}: declares {present} but not {missing}",
                    dialect.slug()
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "window-verdict totality violated (an entry stating one window \
         position must state the other, since lookup never falls between \
         them):\n{}",
        violations.join("\n")
    );
}

// ─── Retired dialects

/// The whole `signatures` module source, concatenated — read at test time
/// rather than `include_str!`-ed one file at a time, so a new submodule
/// dropped into `src/signatures/` (or `src/signatures/builtins/`) is scanned
/// the moment it exists, never only once someone remembers to list it here.
fn signatures_src() -> String {
    fn read_dir_recursive(dir: &std::path::Path, out: &mut Vec<(std::path::PathBuf, String)>) {
        let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .expect("readable signatures module directory")
            .map(|e| e.expect("readable dir entry").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                read_dir_recursive(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let src = std::fs::read_to_string(&path).expect("readable source file");
                out.push((path, src));
            }
        }
    }
    let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src/signatures"));
    let mut files = Vec::new();
    read_dir_recursive(dir, &mut files);
    assert!(
        !files.is_empty(),
        "no signatures sources found — the gate would pass vacuously"
    );
    files
        .into_iter()
        .map(|(_, src)| src)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn no_registry_row_names_a_retired_dialect() {
    // The PostgreSQL emission dialect was retired (#181): no backend crate
    // exists to verify its verdicts, so a template or conditional-verdict
    // arm reintroducing it would carry an unverifiable claim. This scans the
    // registry source directly rather than `DialectId::ALL` (which would
    // trivially be silent about a variant that no longer compiles).
    let src = signatures_src();
    for spelling in ["PostgreSql", "PostgreSQL"] {
        assert!(
            !src.contains(spelling),
            "the signatures module names the retired dialect spelling {spelling:?}"
        );
    }
}

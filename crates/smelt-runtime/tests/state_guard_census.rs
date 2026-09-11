//! Structural gate: the run layer never disagrees with the plan layer about
//! what a dialect can build (`docs/specs/state.md` §"The state-structure
//! inventory"; `docs/outcomes/20260906-bigquery-correctness` success
//! criterion 8).
//!
//! Every `SqlDialect::DuckDB` comparison in `src/maintenance_driver/` is a
//! *state guard*: it exists because some state structure has no realisation
//! off DuckDB. Each one must say which — `// STATE-GUARD: <StateStructure>` —
//! and the named structure must be absent from
//! `realisable_state_structures` for every non-DuckDB dialect.
//!
//! That pins both directions of the failure that produced the 2026-09-10 hard
//! stop:
//!
//! - **Claim without a guard.** A dialect's row flips on in
//!   `realisable_state_structures` while a guard still refuses it — the run
//!   dies where the plan expected a graceful degradation. Caught here,
//!   because the annotated structure would now be realisable.
//! - **Guard without a claim.** A new `!= SqlDialect::DuckDB` lands with no
//!   annotation at all, silently reintroducing a hardcoded dialect
//!   assumption. Caught here as an unannotated site.
//!
//! The preferred fix for a *new* guard is not an annotation but a predicate
//! derived from the availability layer — see
//! `maintenance_driver::records_observed_deltas` and
//! `maintenance_driver::realises_merge_ledger`, which is why the three T5
//! write sites and the merge-ledger bookkeeping site no longer appear in this
//! census at all.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smelt_dialect::SqlDialect;
use smelt_logical::maintenance::availability::{realisable_state_structures, StateStructure};

/// How far above a guard the annotation may sit (the guard is often preceded
/// by a short explanatory comment block).
const ANNOTATION_LOOKBACK_LINES: usize = 3;

const MARKER: &str = "STATE-GUARD:";

#[derive(Debug, PartialEq, Eq)]
struct Guard {
    file: String,
    line: usize,
    structure: Option<String>,
}

fn structure_by_name(name: &str) -> Option<StateStructure> {
    match name {
        "MergeLedger" => Some(StateStructure::MergeLedger),
        "ReconciliationLedger" => Some(StateStructure::ReconciliationLedger),
        "ObservedOutputDeltas" => Some(StateStructure::ObservedOutputDeltas),
        "FingerprintSidecar" => Some(StateStructure::FingerprintSidecar),
        "TombstoneLedger" => Some(StateStructure::TombstoneLedger),
        _ => None,
    }
}

/// Pure scanner, so the non-vacuity controls below can drive it on synthetic
/// text rather than trusting the real tree to contain a counterexample.
fn scan(file: &str, text: &str) -> Vec<Guard> {
    let lines: Vec<&str> = text.lines().collect();
    let mut guards = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        // Prose mentioning the dialect is not a guard.
        if trimmed.starts_with("//") {
            continue;
        }
        if !line.contains("SqlDialect::DuckDB") {
            continue;
        }
        // A struct-literal field (`dialect: SqlDialect::DuckDB`) configures a
        // value; it does not branch on one.
        if trimmed.starts_with("dialect:") {
            continue;
        }
        let lo = i.saturating_sub(ANNOTATION_LOOKBACK_LINES);
        let structure = lines[lo..=i].iter().rev().find_map(|l| {
            l.split_once(MARKER)
                .map(|(_, rest)| rest.trim().to_string())
        });
        guards.push(Guard {
            file: file.to_string(),
            line: i + 1,
            structure,
        });
    }
    guards
}

fn maintenance_driver_sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read maintenance_driver dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                // Test fixtures configure a dialect, they do not guard on one.
                if path.file_name().is_some_and(|n| n == "tests.rs") {
                    continue;
                }
                out.push(path);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/maintenance_driver");
    let mut out = Vec::new();
    walk(&root, &mut out);
    out.sort();
    assert!(!out.is_empty(), "found no maintenance_driver sources");
    out
}

fn census() -> Vec<Guard> {
    let mut guards = Vec::new();
    for path in maintenance_driver_sources() {
        let text = std::fs::read_to_string(&path).expect("read source");
        let rel = path
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap_or(&path)
            .display()
            .to_string();
        guards.extend(scan(&rel, &text));
    }
    guards
}

/// Every guard names a structure, and that structure really is unrealisable
/// off DuckDB.
#[test]
fn every_duckdb_guard_names_an_unrealisable_structure() {
    let mut unannotated = Vec::new();
    let mut unknown = Vec::new();
    let mut contradicted = Vec::new();

    for guard in census() {
        let Some(name) = guard.structure.clone() else {
            unannotated.push(format!("{}:{}", guard.file, guard.line));
            continue;
        };
        let Some(structure) = structure_by_name(&name) else {
            unknown.push(format!("{}:{} names `{name}`", guard.file, guard.line));
            continue;
        };
        for dialect in [SqlDialect::SparkSQL, SqlDialect::BigQuery] {
            if realisable_state_structures(dialect).contains(&structure) {
                contradicted.push(format!(
                    "{}:{} guards {structure:?}, but {dialect:?} now claims to realise it",
                    guard.file, guard.line
                ));
            }
        }
    }

    assert!(
        unannotated.is_empty(),
        "these DuckDB guards name no state structure — add `// {MARKER} <StateStructure>`, or \
         better, derive the guard from the availability layer the way \
         `records_observed_deltas` does:\n  {}",
        unannotated.join("\n  "),
    );
    assert!(
        unknown.is_empty(),
        "unrecognised StateStructure in a {MARKER} annotation:\n  {}",
        unknown.join("\n  "),
    );
    assert!(
        contradicted.is_empty(),
        "the plan layer and the run layer disagree — a dialect claims a structure that a \
         driver guard still refuses. Delete the guard in the same commit that lands the \
         emitters:\n  {}",
        contradicted.join("\n  "),
    );
}

/// The census is not empty and covers the guards we know about, so the test
/// above cannot pass by scanning nothing.
#[test]
fn the_census_is_non_empty_and_covers_the_known_guards() {
    let guards = census();
    assert!(
        guards.len() >= 3,
        "expected at least the three known ledger guards, found {}: {guards:#?}",
        guards.len(),
    );
    let structures: BTreeSet<_> = guards.iter().filter_map(|g| g.structure.clone()).collect();
    for expected in ["ReconciliationLedger", "TombstoneLedger"] {
        assert!(
            structures.contains(expected),
            "expected a {expected} guard in the census, got {structures:?}",
        );
    }
    // Two structures must have NO raw guard left, each for the same reason:
    // their write sites ask a predicate derived from the availability layer
    // (`records_observed_deltas`, `realises_merge_ledger`) instead of comparing
    // dialects themselves. A guard reappearing for either means someone
    // reintroduced a hardcoded dialect assumption.
    for retired in ["ObservedOutputDeltas", "MergeLedger"] {
        assert!(
            !structures.contains(retired),
            "{retired} write sites must derive their gate from the availability layer, not \
             compare dialects directly: {structures:?}",
        );
    }
}

/// Non-vacuity: the scanner really does flag an unannotated guard, really
/// does accept an annotated one, and really does ignore prose and fixtures.
#[test]
fn the_scanner_distinguishes_guards_from_prose() {
    let unannotated = scan(
        "f.rs",
        "fn a() {\n    if x != SqlDialect::DuckDB {\n    }\n}\n",
    );
    assert_eq!(unannotated.len(), 1);
    assert_eq!(unannotated[0].structure, None);

    let annotated = scan(
        "f.rs",
        "fn a() {\n    // STATE-GUARD: MergeLedger\n    if x != SqlDialect::DuckDB {\n    }\n}\n",
    );
    assert_eq!(annotated.len(), 1);
    assert_eq!(annotated[0].structure.as_deref(), Some("MergeLedger"));

    let prose = scan("f.rs", "/// Only SqlDialect::DuckDB has a builder today.\n");
    assert!(prose.is_empty(), "doc prose must not count as a guard");

    let fixture = scan(
        "f.rs",
        "    let b = B {\n        dialect: SqlDialect::DuckDB,\n    };\n",
    );
    assert!(fixture.is_empty(), "a struct-literal field is not a guard");
}

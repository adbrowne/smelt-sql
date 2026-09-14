//! Phase 5 (`docs/outcomes/20260913-trino-ledger/outcome.md`) — **claim ⇒
//! builder**, re-derived from the real builders rather than compared against
//! a second hand-maintained table.
//!
//! `crates/smelt-logical/tests/maintenance_availability/realisation.rs`'s
//! `has_emitters` used to be a hand-typed restatement of which dialect has a
//! builder for which [`StateStructure`] — exactly the kind of second source
//! of truth `docs/specs/state.md` §"The state-structure inventory" warns
//! against (the 2026-09-10 BigQuery/Spark defect this outcome's own
//! `realisation.rs` header cites). This module is the census that lets
//! `has_emitters` be *derived* instead: every `pub fn` builder entry point in
//! `smelt-state::{ledger, observed_delta, tombstone}` is named here against
//! the [`StateStructure`] it realises, [`every_state_builder_entry_point_is_classified`]
//! keeps the census in step with the source (a new or renamed builder fails
//! it by name), and [`a_builder_answers_exactly_when_its_structure_is_claimed`]
//! calls each real builder and checks its `is_ok()` against
//! `realisable_state_structures` directly.

use std::path::{Path, PathBuf};

use smelt_dialect::SqlDialect;
use smelt_logical::maintenance::availability::{realisable_state_structures, StateStructure};
use smelt_types::DataType;

/// One builder entry point (a `pub fn` in `smelt-state::{ledger,
/// observed_delta, tombstone}` returning a dialect-refusing `Result`) and the
/// [`StateStructure`] it realises.
struct CensusEntry {
    name: &'static str,
    structure: StateStructure,
}

/// The census. Editing this list without landing (or removing) the matching
/// `smelt-state` builder turns [`every_state_builder_entry_point_is_classified`]
/// red from one side or the other.
///
/// `ledger_table_ddl`/`ledger_insert_sql`/`ledger_fold_record_sql`/
/// `ledger_exists_sql`/`ledger_recompute_reset_sqls` all serve the additive
/// fold's never-fold-twice refusal (`StateStructure::ReconciliationLedger`);
/// `ledger_upsert_sql` alone serves the idempotent bookkeeping record
/// (`StateStructure::MergeLedger`) — the split `maintenance_driver::ledger`'s
/// `realises_merge_ledger`/`realises_reconciliation_ledger` doc comments
/// describe. `StateStructure::MergeLedger` and `StateStructure::
/// ReconciliationLedger` happen to agree on every [`SqlDialect`] today, so
/// this split has no effect on [`a_builder_answers_exactly_when_its_structure_is_claimed`]'s
/// verdicts yet — it is recorded for when they first diverge (a dialect
/// gaining one ledger structure without the other).
const CENSUS: &[CensusEntry] = &[
    CensusEntry {
        name: "ledger_table_ddl",
        structure: StateStructure::ReconciliationLedger,
    },
    CensusEntry {
        name: "ledger_insert_sql",
        structure: StateStructure::ReconciliationLedger,
    },
    CensusEntry {
        name: "ledger_fold_record_sql",
        structure: StateStructure::ReconciliationLedger,
    },
    CensusEntry {
        name: "ledger_upsert_sql",
        structure: StateStructure::MergeLedger,
    },
    CensusEntry {
        name: "ledger_exists_sql",
        structure: StateStructure::ReconciliationLedger,
    },
    CensusEntry {
        name: "ledger_recompute_reset_sqls",
        structure: StateStructure::ReconciliationLedger,
    },
    CensusEntry {
        name: "observed_delta_table_ddl",
        structure: StateStructure::ObservedOutputDeltas,
    },
    CensusEntry {
        name: "observed_delta_upsert_sql",
        structure: StateStructure::ObservedOutputDeltas,
    },
    CensusEntry {
        name: "observed_delta_select_sql",
        structure: StateStructure::ObservedOutputDeltas,
    },
    CensusEntry {
        name: "tombstone_table_ddl",
        structure: StateStructure::TombstoneLedger,
    },
    CensusEntry {
        name: "tombstone_table_drop_ddl",
        structure: StateStructure::TombstoneLedger,
    },
];

/// The three `smelt-state` files the census must exactly cover.
fn state_files() -> Vec<PathBuf> {
    let state_src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../smelt-state/src");
    vec![
        state_src.join("ledger.rs"),
        state_src.join("observed_delta.rs"),
        state_src.join("tombstone.rs"),
    ]
}

/// Scan `path` for every `pub fn NAME(...) -> ReturnType {` whose return
/// type names one of the three dialect-refusing `Result` aliases
/// (`LedgerResult`, `ObservedDeltaResult`, or `TombstoneDdlError`, the last
/// spelled directly since it is `pub enum`, not a private `type` alias).
fn builder_entry_points(path: &Path) -> Vec<String> {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while let Some(idx) = rest.find("pub fn ") {
        let after = &rest[idx + "pub fn ".len()..];
        let Some(paren) = after.find('(') else { break };
        let name = after[..paren].trim().to_string();
        // The signature's own return arrow lives before the next `pub fn`
        // (or, for the file's last function, before the file ends) — every
        // signature in these three files closes `) -> ReturnType {` on one
        // line, so a bounded forward window is enough.
        let window_end = after[paren..]
            .find("\npub fn")
            .map(|i| paren + i)
            .unwrap_or(after.len());
        let window = &after[paren..window_end];
        let is_builder = if let Some(arrow) = window.find("->") {
            let after_arrow = &window[arrow + 2..];
            let brace = after_arrow.find('{').unwrap_or(after_arrow.len());
            let return_type = &after_arrow[..brace];
            return_type.contains("LedgerResult")
                || return_type.contains("ObservedDeltaResult")
                || return_type.contains("TombstoneDdlError")
        } else {
            false
        };
        if is_builder {
            out.push(name);
        }
        rest = &rest[idx + "pub fn ".len()..];
    }
    out
}

/// Two-sided: every `pub fn` builder entry point in the three files is named
/// in [`CENSUS`], and every [`CENSUS`] entry names a `pub fn` that still
/// exists. A new or renamed builder that nothing classifies fails from the
/// first side; a stale census row naming a removed function fails from the
/// second.
#[test]
fn every_state_builder_entry_point_is_classified() {
    let mut found: Vec<String> = Vec::new();
    for file in state_files() {
        found.extend(builder_entry_points(&file));
    }
    found.sort();
    let mut censused: Vec<&str> = CENSUS.iter().map(|e| e.name).collect();
    censused.sort();

    let missing_from_census: Vec<&String> = found
        .iter()
        .filter(|f| !censused.contains(&f.as_str()))
        .collect();
    let stale_in_census: Vec<&&str> = censused
        .iter()
        .filter(|c| !found.iter().any(|f| f == *c))
        .collect();

    assert!(
        missing_from_census.is_empty() && stale_in_census.is_empty(),
        "census drift: builder(s) with no census row = {missing_from_census:?}; census row(s) \
         naming no builder = {stale_in_census:?}"
    );
}

/// Call the real builder named by `entry` with representative sample
/// arguments, returning `is_ok()`.
fn call_is_ok(entry: &str, dialect: SqlDialect) -> bool {
    use smelt_state::{ledger, observed_delta, tombstone};
    match entry {
        "ledger_table_ddl" => ledger::ledger_table_ddl(dialect, "s").is_ok(),
        "ledger_insert_sql" => {
            ledger::ledger_insert_sql(dialect, "s", "m", "g", "i", "d", "rs", "re").is_ok()
        }
        "ledger_fold_record_sql" => {
            ledger::ledger_fold_record_sql(dialect, "s", "m", "g", "i", "d", "rs", "re").is_ok()
        }
        "ledger_upsert_sql" => {
            ledger::ledger_upsert_sql(dialect, "s", "m", "g", "i", "d", "rs", "re").is_ok()
        }
        "ledger_exists_sql" => ledger::ledger_exists_sql(dialect, "s", "m", "g", "i", "d").is_ok(),
        "ledger_recompute_reset_sqls" => {
            ledger::ledger_recompute_reset_sqls(dialect, "s", "m", "g", "rs", "re", "i", "d")
                .is_ok()
        }
        "observed_delta_table_ddl" => {
            observed_delta::observed_delta_table_ddl(dialect, "s").is_ok()
        }
        "observed_delta_upsert_sql" => {
            observed_delta::observed_delta_upsert_sql(dialect, "s", "m", "s0", "e0", "q").is_ok()
        }
        "observed_delta_select_sql" => {
            observed_delta::observed_delta_select_sql(dialect, "s", "m", "s0", "e0").is_ok()
        }
        "tombstone_table_ddl" => tombstone::tombstone_table_ddl(
            dialect,
            "q",
            &[("k".to_string(), DataType::Integer)],
            "clk",
            &DataType::Timestamp {
                with_timezone: false,
            },
        )
        .is_ok(),
        "tombstone_table_drop_ddl" => tombstone::tombstone_table_drop_ddl(dialect, "q").is_ok(),
        other => panic!("call_is_ok: unclassified census entry {other}"),
    }
}

const ALL_DIALECTS: [SqlDialect; 4] = [
    SqlDialect::DuckDB,
    SqlDialect::SparkSQL,
    SqlDialect::BigQuery,
    SqlDialect::Trino,
];

/// The right-hand side of "claim ⇒ builder", from the real builders: for
/// every dialect and every census entry, the builder answers `Ok` exactly
/// when `realisable_state_structures` claims the entry's structure.
///
/// Replaces `realisation.rs::has_emitters` as the source of truth for the
/// four builder-backed structures — `FingerprintSidecar` has no
/// `smelt-state` builder (its realisation gate is the capability flag
/// covered by [`crate::realisation`]'s widened capability loop, not a
/// builder call).
#[test]
fn a_builder_answers_exactly_when_its_structure_is_claimed() {
    for dialect in ALL_DIALECTS {
        let claimed = realisable_state_structures(dialect);
        for entry in CENSUS {
            let is_ok = call_is_ok(entry.name, dialect);
            let should_be_ok = claimed.contains(&entry.structure);
            assert_eq!(
                is_ok, should_be_ok,
                "{dialect:?} / {}: builder answered ok={is_ok} but \
                 realisable_state_structures says claimed={should_be_ok} for {:?}",
                entry.name, entry.structure,
            );
        }
    }
}

/// **State-builder call-site allowlist** — every production call site of a
/// census entry point (called as `<path>::NAME(`, any module alias) must
/// live inside one of the gated modules: `smelt-runtime/src/
/// maintenance_driver/`, `smelt-runtime/src/execute/project/
/// ledger_reset.rs`, `smelt-runtime/src/execute/key_addressed.rs`,
/// `smelt-backend-bigquery/src/`, or `smelt-state/src/` itself (the
/// builders calling each other, and doc comments naming one). A call from
/// anywhere else is a path the `required_state_structure` gating argument
/// never covered.
#[test]
fn state_builder_call_sites_stay_inside_the_gated_modules() {
    let crates_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("crates/ dir exists");
    let mut offenders = Vec::new();
    for crate_entry in std::fs::read_dir(&crates_dir).expect("read crates/") {
        let crate_entry = crate_entry.expect("dir entry");
        let src_dir = crate_entry.path().join("src");
        if !src_dir.is_dir() {
            continue;
        }
        for file in walk_rs_files(&src_dir) {
            let rel = file
                .strip_prefix(&crates_dir)
                .expect("file under crates/")
                .to_string_lossy()
                .replace('\\', "/");
            let allowed = rel.starts_with("smelt-runtime/src/maintenance_driver/")
                || rel == "smelt-runtime/src/execute/project/ledger_reset.rs"
                || rel == "smelt-runtime/src/execute/key_addressed.rs"
                || rel.starts_with("smelt-backend-bigquery/src/")
                || rel.starts_with("smelt-state/src/");
            if allowed {
                continue;
            }
            let text = std::fs::read_to_string(&file).expect("read source file");
            for (line_no, line) in text.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                for entry in CENSUS {
                    let needle = format!("::{}(", entry.name);
                    if line.contains(&needle) {
                        offenders.push(format!("{rel}:{} ({})", line_no + 1, entry.name));
                    }
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "found a state-builder call site outside the gated modules: {offenders:?}"
    );
}

fn walk_rs_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_rs_files(&path));
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    out
}

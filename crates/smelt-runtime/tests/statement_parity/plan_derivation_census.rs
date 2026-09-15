//! Criterion 6 (`docs/outcomes/20260913-trino-incremental/phases/
//! 07-plan.md`): the maintenance plan is derived exactly once, in
//! `smelt-logical`'s single owner, and no plan-consuming tree branches on a
//! backend's resolved dialect. Two independent source-scan gates, in the
//! `hardening_budget.rs`/`structural_and_ledger.rs` style — offline, no live
//! tier needed.

use std::path::{Path, PathBuf};

use super::structural_and_ledger::repo_root;

// =============================================================================
// Gate A: the maintenance plan has exactly one production derivation site.
// =============================================================================

/// One `derive_maintenance_plan`-family call/definition-site hit.
#[derive(Debug)]
struct DerivationHit {
    file: PathBuf,
    line_no: usize,
    text: String,
}

/// Substrings identifying a call into the plan-derivation family
/// (`derive_maintenance_plan`/`derive_maintenance_plan_with_referential_
/// integrity`/`derive_maintenance_plan_with_referential_integrity_and_
/// retentions`/`append_model_edge_cells`/`derive_triggers`). A single
/// `"derive_maintenance_plan"` substring catches all three plan-family
/// names since they share that prefix.
const DERIVATION_MARKERS: &[&str] = &[
    "derive_maintenance_plan",
    "append_model_edge_cells",
    "derive_triggers",
];

/// The allowlist is two entries, both required to have at least one hit
/// (asserted in [`maintenance_plan_is_derived_in_exactly_one_production_site`]):
/// `smelt-logical/src/maintenance/derive/`, the owner module directory
/// (definitions plus its own unit tests), and `smelt-db/src/queries/
/// maintenance/plan.rs`, the one Salsa-cached consumer that calls into it
/// (`docs/specs/architecture.md` §"Salsa purity rule (analysis)" — the
/// Salsa query is a thin wrapper that calls the pure derivation).
fn is_allowed_derivation_site(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized.contains("smelt-logical/src/maintenance/derive/")
        || normalized.ends_with("smelt-db/src/queries/maintenance/plan.rs")
}

/// Same truncation/skip rules as `structural_and_ledger::scan_statement_authoring_file`:
/// stop at the first `#[cfg(test)]` line, skip comment lines (the family's
/// name appears constantly in doc comments cross-referencing the derivation,
/// which is not a call site).
fn scan_derivation_file(path: &Path, hits: &mut Vec<DerivationHit>) {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#[cfg(test)]") {
            break;
        }
        if trimmed.starts_with("//") {
            continue;
        }
        if DERIVATION_MARKERS.iter().any(|m| line.contains(m)) {
            hits.push(DerivationHit {
                file: path.to_path_buf(),
                line_no: idx + 1,
                text: line.trim().to_string(),
            });
        }
    }
}

fn scan_derivation_dir(dir: &Path, hits: &mut Vec<DerivationHit>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().map(|n| n == "tests").unwrap_or(false) {
                continue;
            }
            scan_derivation_dir(&path, hits);
        } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
            if path.file_name().map(|n| n == "tests.rs").unwrap_or(false) {
                continue;
            }
            scan_derivation_file(&path, hits);
        }
    }
}

/// Every `crates/*/src` directory — the production scan root for both
/// gates in this file.
fn all_crate_src_dirs() -> Vec<PathBuf> {
    let crates_dir = repo_root().join("crates");
    let mut dirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&crates_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let src = path.join("src");
                if src.is_dir() {
                    dirs.push(src);
                }
            }
        }
    }
    dirs
}

/// Test 4: the hit file set equals the allowlist — every hit lives inside
/// `smelt-logical/src/maintenance/derive/` or is exactly `smelt-db/src/
/// queries/maintenance/plan.rs`, and both categories actually have at least
/// one hit (so this isn't vacuously true if the scan itself broke).
#[test]
fn maintenance_plan_is_derived_in_exactly_one_production_site() {
    let mut hits = Vec::new();
    for src in all_crate_src_dirs() {
        scan_derivation_dir(&src, &mut hits);
    }

    let disallowed: Vec<_> = hits
        .iter()
        .filter(|h| !is_allowed_derivation_site(&h.file))
        .collect();
    assert!(
        disallowed.is_empty(),
        "a maintenance-plan derivation call site was found outside the single owner \
         (smelt-logical/src/maintenance/derive/) and its one Salsa-cached consumer \
         (smelt-db/src/queries/maintenance/plan.rs) — a second derivation site has appeared:\n{}",
        disallowed
            .iter()
            .map(|h| format!("  {}:{}: {}", h.file.display(), h.line_no, h.text))
            .collect::<Vec<_>>()
            .join("\n")
    );

    let normalized_files: Vec<String> = hits
        .iter()
        .map(|h| h.file.to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(
        normalized_files
            .iter()
            .any(|f| f.contains("smelt-logical/src/maintenance/derive/")),
        "expected at least one derivation hit inside smelt-logical/src/maintenance/derive/ \
         (the owner) — got none; the scan may be broken"
    );
    assert!(
        normalized_files
            .iter()
            .any(|f| f.ends_with("smelt-db/src/queries/maintenance/plan.rs")),
        "expected at least one derivation hit in smelt-db/src/queries/maintenance/plan.rs \
         (the one Salsa-cached consumer) — got none; the scan may be broken"
    );
}

/// Test 5: the scoping proof for test 4 — a synthetic file calling
/// `derive_maintenance_plan(` outside the allowlist is flagged.
#[test]
fn census_flags_a_second_derivation_site() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let file = tmp.path().join("rogue.rs");
    std::fs::write(
        &file,
        "fn f(inputs: &ModelInputs) -> MaintenancePlan {\n    \
         derive_maintenance_plan(inputs, &[])\n}\n",
    )
    .unwrap();

    let mut hits = Vec::new();
    scan_derivation_file(&file, &mut hits);

    assert_eq!(
        hits.len(),
        1,
        "a call to derive_maintenance_plan( outside the allowlist must be flagged: {:#?}",
        hits
    );
    assert!(
        !is_allowed_derivation_site(&file),
        "the synthetic temp file must not be on the allowlist"
    );
}

// =============================================================================
// Gate B: no plan consumer branches on a backend's resolved dialect.
// =============================================================================

/// One `SqlDialect::`/`MaintenanceDialect::`/`BackendType::` occurrence hit
/// in a plan-consuming tree.
#[derive(Debug)]
struct DialectBranchHit {
    file: PathBuf,
    line_no: usize,
    text: String,
}

const DIALECT_BRANCH_MARKERS: &[&str] = &["SqlDialect::", "MaintenanceDialect::", "BackendType::"];

/// The plan-consuming trees this gate scans (`docs/outcomes/
/// 20260913-trino-incremental/phases/07-plan.md`): the maintenance-plan
/// Salsa queries, the maintenance-plan-derived clamp/edge refs, the
/// planner's rule-application layer, and the runtime's compile+execute
/// driver.
const PLAN_CONSUMER_TREES: &[&str] = &[
    "smelt-db/src/queries/maintenance",
    "smelt-db/src/maintenance_refs",
    "smelt-planner/src",
    "smelt-runtime/src/execute",
];

/// `(file suffix, substring, reason)` — every entry here is a dialect
/// *selection* site (turning a declared backend name/type into the
/// `SqlDialect`/`MaintenanceDialect` value everything downstream carries as
/// data) or a narrowing that predates this outcome, never a branch added to
/// special-case a particular backend for this outcome's sake. Removing an
/// entry without fixing (or re-justifying) the underlying branch is itself
/// the review signal this gate exists to raise.
const DIALECT_BRANCH_ALLOWLIST: &[(&str, &str, &str)] = &[
    (
        "smelt-runtime/src/execute/targets.rs",
        "SqlDialect::",
        "the two total BackendType -> SqlDialect selection maps in this file \
         (maintenance_dialect_for_target, sql_dialect_for_target) plus their DuckDB \
         fallback arms — dialect selection, the input every downstream consumer reads as \
         data, not a consumer branching on an already-resolved dialect",
    ),
    (
        "smelt-runtime/src/execute/targets.rs",
        "MaintenanceDialect::",
        "the same selection map's no-backend MaintenanceDialect fallback arm",
    ),
    (
        "smelt-db/src/queries/maintenance/write_pin.rs",
        "SqlDialect::",
        "the `write:` pin's backend-name-string -> SqlDialect parse (backend_dialect_for) — \
         pre-existing dialect selection from a declared name, not a branch on a resolved plan",
    ),
    (
        "smelt-runtime/src/execute/project/mod.rs",
        "== smelt_backend::SqlDialect::DuckDB",
        "DuckDB-only capability narrowing pre-dating this outcome (delta-restricted dispatch \
         x2, the DuckDB-qualified diagnostic report near line 4245)",
    ),
    (
        "smelt-runtime/src/execute/project/mod.rs",
        "BackendType::Databricks",
        "refuse_databricks_cross_edges' databricks-target check (no host-visible warehouse \
         path for read_parquet(), docs/specs/multi_backend.md \"Cross-engine data exchange\") \
         — pre-existing, measured by this phase's census, not enumerated in the original plan \
         listing",
    ),
];

fn dialect_branch_is_allowlisted(path: &Path, line: &str) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    DIALECT_BRANCH_ALLOWLIST
        .iter()
        .any(|(suffix, substr, _)| normalized.ends_with(suffix) && line.contains(substr))
}

fn scan_dialect_branch_file(path: &Path, hits: &mut Vec<DialectBranchHit>) {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#[cfg(test)]") {
            break;
        }
        if trimmed.starts_with("//") {
            continue;
        }
        if DIALECT_BRANCH_MARKERS.iter().any(|m| line.contains(m)) {
            hits.push(DialectBranchHit {
                file: path.to_path_buf(),
                line_no: idx + 1,
                text: line.trim().to_string(),
            });
        }
    }
}

fn scan_dialect_branch_dir(dir: &Path, hits: &mut Vec<DialectBranchHit>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().map(|n| n == "tests").unwrap_or(false) {
                continue;
            }
            scan_dialect_branch_dir(&path, hits);
        } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
            if path.file_name().map(|n| n == "tests.rs").unwrap_or(false) {
                continue;
            }
            scan_dialect_branch_file(&path, hits);
        }
    }
}

/// Test 6: every `SqlDialect::`/`MaintenanceDialect::`/`BackendType::` hit
/// in the plan-consuming trees matches an allowlist entry.
#[test]
fn plan_consumers_hold_no_per_dialect_branch() {
    let crates_dir = repo_root().join("crates");
    let mut hits = Vec::new();
    for tree in PLAN_CONSUMER_TREES {
        scan_dialect_branch_dir(&crates_dir.join(tree), &mut hits);
    }

    let disallowed: Vec<_> = hits
        .iter()
        .filter(|h| !dialect_branch_is_allowlisted(&h.file, &h.text))
        .collect();
    assert!(
        disallowed.is_empty(),
        "a plan-consuming file branches on a backend's resolved dialect outside the allowlisted \
         selection sites — either widen the allowlist with a measured reason or remove the \
         branch (the maintenance plan must be dialect-independent data, docs/specs/\
         architecture.md §\"Constraints & Invariants\" item 12):\n{}",
        disallowed
            .iter()
            .map(|h| format!("  {}:{}: {}", h.file.display(), h.line_no, h.text))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Test 7: criterion 6 in assertion form — no allowlist entry's substring
/// or reason names Trino, so adding Trino provably introduced no
/// consumer-side dialect branch. Every entry above is either a pre-existing
/// selection site or a pre-existing narrowing unrelated to this outcome.
#[test]
fn no_consumer_dialect_allowlist_entry_names_trino() {
    for (file, substr, reason) in DIALECT_BRANCH_ALLOWLIST {
        assert!(
            !substr.contains("Trino"),
            "allowlist substring for {file} names Trino: {substr}"
        );
        assert!(
            !reason.contains("Trino"),
            "allowlist reason for {file} names Trino: {reason}"
        );
    }
}

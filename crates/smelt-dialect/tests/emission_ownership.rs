//! The printer holds no name-matched dialect arm.
//!
//! `CLAUDE.md` §"Function-registry single ownership" extends to a built-in's
//! emission: a function's per-dialect spelling derives from `BuiltinRegistry`,
//! never from a `match dialect` / `eq_ignore_ascii_case` chain in the printer.
//! This is the sibling gate to `registry_consistency`.

const PRINTER_SRC: &str = include_str!("../src/printer.rs");

/// Case-folded comparisons that are **not** per-dialect emission facts, and so
/// cannot move into `Signature::emission`.
///
/// One entry only: `DATE 'lit'` → `DATE('lit')` matches the *literal keyword*
/// `DATE`, not a function name, and it is already gated on
/// `BackendCapabilities::supports_date_literal` rather than on a dialect. There
/// is no registry entry for a literal form to carry the verdict.
///
/// Adding an entry here is a deliberate, reviewable widening of the gate — it is
/// not the escape hatch for a function-name match. Spelling the comparison a
/// different way (`to_ascii_uppercase()`, `to_lowercase()`, a `matches!` on
/// both cases) to slip past the substring check is the same violation, written
/// less honestly.
const NON_EMISSION_CASE_FOLDS: &[&str] = &[r#"token.text().eq_ignore_ascii_case("DATE")"#];

#[test]
fn the_printer_matches_no_function_name() {
    let hits: Vec<(usize, &str)> = PRINTER_SRC
        .lines()
        .enumerate()
        .filter(|(_, l)| l.contains("eq_ignore_ascii_case"))
        .filter(|(_, l)| !NON_EMISSION_CASE_FOLDS.iter().any(|a| l.contains(a)))
        .map(|(i, l)| (i + 1, l.trim()))
        .collect();
    assert!(
        hits.is_empty(),
        "printer.rs matches function names by string. Per-dialect spelling is \
         registry data (`Signature::emission`); move the fact into \
         `crates/smelt-types/src/signatures.rs` rather than re-adding an arm here.\n{hits:#?}"
    );
}

/// The allowlist above must stay live: an entry matching nothing is a stale
/// exemption that would quietly re-open the gate for a future line.
#[test]
fn every_allowlisted_case_fold_is_still_present() {
    for allowed in NON_EMISSION_CASE_FOLDS {
        assert!(
            PRINTER_SRC.contains(allowed),
            "allowlisted non-emission case-fold `{allowed}` no longer appears in printer.rs — \
             delete the entry from NON_EMISSION_CASE_FOLDS rather than leaving a dead exemption"
        );
    }
}

#[test]
fn the_printer_branches_on_no_dialect_variant() {
    let hits: Vec<(usize, &str)> = PRINTER_SRC
        .lines()
        .enumerate()
        .filter(|(_, l)| {
            [
                "SqlDialect::DuckDB",
                "SqlDialect::SparkSQL",
                "SqlDialect::PostgreSQL",
                "SqlDialect::BigQuery",
            ]
            .iter()
            .any(|v| l.contains(v))
        })
        .map(|(i, l)| (i + 1, l.trim()))
        .collect();
    assert!(
        hits.is_empty(),
        "printer.rs branches on a concrete dialect. Emission facts belong in \
         `Signature::emission`; capability-shaped differences belong in \
         `BackendCapabilities`.\n{hits:#?}"
    );
}

const SIGNATURES_SRC: &str = include_str!("../../smelt-types/src/signatures.rs");

/// The `RewriteId` variants declared in `smelt-types`, read from the source
/// rather than restated here — a hand-copied list would go stale exactly when a
/// new rewrite is added, which is the moment this gate has to fire.
fn declared_rewrite_ids() -> Vec<String> {
    let body = SIGNATURES_SRC
        .split_once("pub enum RewriteId {")
        .expect("signatures.rs must declare `pub enum RewriteId`")
        .1
        .split_once("\n}")
        .expect("`enum RewriteId` must be brace-terminated")
        .0;
    let ids: Vec<String> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("//") && !l.starts_with("#["))
        .filter_map(|l| l.strip_suffix(','))
        .map(str::to_string)
        .collect();
    assert!(
        !ids.is_empty(),
        "parsed no variants out of `enum RewriteId` — the parser above has gone stale"
    );
    ids
}

#[test]
fn every_rewrite_id_is_dispatched() {
    // A RewriteId the printer never mentions is a registry claim with no
    // implementation — the failure mode a name-matched `if` chain hid.
    for id in declared_rewrite_ids() {
        assert!(
            PRINTER_SRC.contains(&format!("RewriteId::{id}")),
            "RewriteId::{id} is declared in the registry but never dispatched in printer.rs"
        );
    }
}

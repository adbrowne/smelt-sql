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
const NON_EMISSION_CASE_FOLDS: &[&str] = &[
    // `DATE 'lit'` -> `DATE('lit')` matches the literal *keyword* `DATE`, not a
    // function name, and is gated on `supports_date_literal`.
    r#"token.text().eq_ignore_ascii_case("DATE")"#,
    // "did the author already write the target spelling?" — compares the source
    // text against the *registry's own* `Rename` target, so it names no dialect
    // and hardcodes no spelling. Suppressing a no-op rewrite is what keeps
    // DuckDB byte-identity when a user writes `json_extract_string` themselves.
    r#"if name.eq_ignore_ascii_case(new_name) {"#,
];

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

/// The `RestructureId` variants declared in `smelt-types`, read from the
/// source rather than restated here, mirroring `declared_rewrite_ids` — a
/// hand-copied list would go stale exactly when a new restructure shape is
/// added, which is the moment this gate has to fire.
fn declared_restructure_ids() -> Vec<String> {
    let body = SIGNATURES_SRC
        .split_once("pub enum RestructureId {")
        .expect("signatures.rs must declare `pub enum RestructureId`")
        .1
        .split_once("\n}")
        .expect("`enum RestructureId` must be brace-terminated")
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
        "parsed no variants out of `enum RestructureId` — the parser above has gone stale"
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

/// A `RestructureId` the printer never mentions is a registry claim with no
/// implementation, exactly like an undispatched `RewriteId`. The printer
/// never matches on `RestructureId` directly — the plan a call's
/// `Emission::Restructure(RestructureId)` verdict points at is already turned
/// into data by `restructure::plan` before printing starts, and
/// `RestructurePlan`'s variants (`WindowToCte`, `AnalyticToCte`) are named to
/// match `RestructureId`'s one-for-one — so this checks that each id's name
/// appears dispatched as a `RestructurePlan` arm instead.
#[test]
fn every_restructure_id_is_dispatched() {
    for id in declared_restructure_ids() {
        assert!(
            PRINTER_SRC.contains(&format!("RestructurePlan::{id}")),
            "RestructureId::{id} is declared in the registry but its RestructurePlan \
             counterpart is never dispatched in printer.rs"
        );
    }
}

/// The printer holds no name-matched, sibling-peeking, or otherwise ad hoc
/// derivation of a call's SQL position.
///
/// `docs/specs/multi_backend.md` §"Emission is scoped to call position":
/// "Position is decided once, by the compile path, from the source CST, and
/// handed to the registry. The printer never re-derives it: a printer that
/// inspected sibling nodes to tell aggregate position from window position
/// would hold emission knowledge the registry owns." This is what
/// `print_bigquery_median`'s old `node.next_sibling()` / `WINDOW_SPEC` peek
/// did before it was replaced by a `Position` argument computed once, via
/// `position::classify`, in `emit_registered_function`.
#[test]
fn the_printer_derives_no_position_itself() {
    let forbidden = ["next_sibling", "SyntaxKind::WINDOW_SPEC", "WINDOW_SPEC"];
    let hits: Vec<(usize, &str)> = PRINTER_SRC
        .lines()
        .enumerate()
        .filter(|(_, l)| forbidden.iter().any(|f| l.contains(f)))
        .map(|(i, l)| (i + 1, l.trim()))
        .collect();
    assert!(
        hits.is_empty(),
        "printer.rs inspects sibling nodes or a WINDOW_SPEC directly to tell \
         a call's position — that is `position::classify`'s question to \
         answer, once, before the printer ever sees the call. Pass the \
         classified `Position` in instead of re-deriving it here.\n{hits:#?}"
    );
}

/// `emit_registered_function` — the one production call site that resolves a
/// *call's* position (as opposed to `emit_registered_operator`, which always
/// states `Position::Any` because an operator is never itself a window or
/// aggregate call) — must obtain it from `position::classify`, never invent
/// one locally.
#[test]
fn the_printer_classifies_position_through_one_function() {
    assert!(
        PRINTER_SRC.contains("classify_position(node, &root)")
            || PRINTER_SRC.contains("classify_position(node, root)"),
        "emit_registered_function must resolve a call's position via \
         `position::classify` (re-exported as `classify_position`), not by \
         deriving it locally"
    );
}

/// Every `RewriteId` variant's doc comment must state which call structure a
/// placeholder could not name — the reason it is not a `Template` row instead
/// (`docs/specs/multi_backend.md` §"Template interpretation is generic").
#[test]
fn every_rewrite_id_states_why_it_is_not_a_template() {
    let body = SIGNATURES_SRC
        .split_once("pub enum RewriteId {")
        .expect("signatures.rs must declare `pub enum RewriteId`")
        .1
        .split_once("\n}")
        .expect("`enum RewriteId` must be brace-terminated")
        .0;

    // Walk the enum body, resetting the doc-comment buffer at each variant so
    // the check is per-variant, not "somewhere in the enum".
    let mut missing = Vec::new();
    let mut doc_has_justification = false;
    for line in body.lines() {
        let line = line.trim();
        if line.starts_with("///") {
            if line.contains("Not a template:") {
                doc_has_justification = true;
            }
            continue;
        }
        if line.starts_with("#[") || line.is_empty() {
            continue;
        }
        // A variant line: `Name,` or `Name { .. },`.
        if let Some(name) = line.split([',', ' ', '{']).next().filter(|s| !s.is_empty()) {
            if !doc_has_justification {
                missing.push(name.to_string());
            }
            doc_has_justification = false;
        }
    }
    assert!(
        missing.is_empty(),
        "RewriteId variant(s) {missing:?} carry no `Not a template: …` doc line stating which \
         call structure a placeholder could not name"
    );
}

/// The printer must never need a type to print — arm resolution for an
/// `Emission::Conditional` entry happens on the compile path, before the
/// printer ever runs (`docs/specs/multi_backend.md`
/// §"Operand-conditional verdicts": "The printer holds no type context and
/// cannot ask for one").
#[test]
fn printer_holds_no_type_context() {
    let hits: Vec<(usize, &str)> = PRINTER_SRC
        .lines()
        .enumerate()
        .filter(|(_, l)| {
            ["DataType", "TypeContext", "OperandClass"]
                .iter()
                .any(|needle| l.contains(needle))
        })
        .map(|(i, l)| (i + 1, l.trim()))
        .collect();
    assert!(
        hits.is_empty(),
        "printer.rs references a type or type-context symbol — the printer must consume only \
         pre-settled `SettledEmission` verdicts, never resolve one itself.\n{hits:#?}"
    );
}

/// The printer consumes settled verdicts (`ctx.settled_emissions`, via
/// `crate::emission_settle::settled_verdict_for`) — it never resolves a
/// `Conditional` arm or calls the settlement functions itself.
#[test]
fn printer_never_resolves_an_arm() {
    let hits: Vec<(usize, &str)> = PRINTER_SRC
        .lines()
        .enumerate()
        .filter(|(_, l)| {
            ["Emission::Conditional", "settle_at", "settle_emissions"]
                .iter()
                .any(|needle| l.contains(needle))
        })
        .map(|(i, l)| (i + 1, l.trim()))
        .collect();
    assert!(
        hits.is_empty(),
        "printer.rs resolves a `Conditional` arm itself — that is \
         `Signature::settle_at`'s job, run once on the compile path before printing.\n{hits:#?}"
    );
}

/// The template interpreter holds no target-dialect text of its own — every
/// character it emits comes from the registry's template string or from
/// re-printing the call's own arguments. A double-quoted string literal in
/// either function's body would be target text the interpreter authored
/// itself, which is exactly the per-function knowledge templates exist to
/// remove from the printer.
#[test]
fn the_template_interpreter_holds_no_target_text() {
    for (fn_name, needle) in [
        ("print_template", "pub fn print_template("),
        ("is_compound_argument", "fn is_compound_argument("),
    ] {
        let start = PRINTER_SRC
            .find(needle)
            .unwrap_or_else(|| panic!("printer.rs must declare `{fn_name}`"));
        let body = &PRINTER_SRC[start..];
        let end = body
            .find("\n}\n")
            .unwrap_or_else(|| panic!("`{fn_name}` must be brace-terminated"));
        let body = &body[..end];
        assert!(
            !body.contains('"'),
            "{fn_name} contains a double-quoted string literal — the template interpreter \
             must hold no target-dialect text of its own; every character it emits must \
             come from the registry's template string or a re-printed argument"
        );
    }
}

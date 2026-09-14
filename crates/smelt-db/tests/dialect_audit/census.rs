//! The Trino emission coverage census — `docs/outcomes/20260913-trino-emission`
//! phase 2.
//!
//! Refines the three-way `passing`/`gap`/`unverified` vocabulary
//! `docs/specs/multi_backend.md` §"Cross-engine emission audit" states:
//! `passing` splits into [`Coverage::Stated`] (an explicit verdict exists) and
//! [`Coverage::Verified`] (the implicit `Native` default is backed by both a
//! live schema leg and a live value leg — see `BOTH_LEGS_LIVE` below).
//! [`Coverage::Unverified`] is the hole the implicit-`Native` default opens:
//! no explicit verdict, and no both-legs-live audit to have observed the
//! default correct. Every `Unverified` pair for [`DialectId::Trino`] must be named,
//! line for line, in the shrink-only census at
//! `.claude/trino-emission-census.txt` — a pair absent from that file fails
//! immediately, and a recorded pair that has since gained a verdict or an
//! audit leg is a stale-census failure.

use std::collections::HashSet;
use std::path::PathBuf;

use smelt_types::signatures::{Position, Signature};
use smelt_types::DialectId;

use crate::ledger::{LedgerRow, Verdict};
use crate::report::{applicable_positions, position_label};
use crate::AUDITED_DIALECTS;

/// Dialects with **both** the schema and value legs live — the narrower set
/// `classify` consults for `Coverage::Verified`, distinct from
/// [`crate::AUDITED_DIALECTS`] (which drives the offline totality gates: the
/// fixture gate and the print-for-every-dialect gate). A dialect can join
/// `AUDITED_DIALECTS` with only a schema leg — as Trino did in
/// `20260913-trino-emission` phase 5 — without every one of its 232 census
/// rows silently flipping to `Verified` on the strength of that leg alone.
/// Trino joins this set once phase 6 lands its value leg.
const BOTH_LEGS_LIVE: &[DialectId] = &[DialectId::DuckDb, DialectId::SparkSql, DialectId::BigQuery];

/// The four-way refinement of `passing`/`gap`/`unverified`
/// (`docs/specs/multi_backend.md` §"Cross-engine emission audit") the census
/// classifies each `(dialect, entry, position)` pair into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    /// An explicit `(dialect, position)` verdict exists in the registry.
    Stated,
    /// No explicit verdict, but the dialect has both legs live — the implicit
    /// `Native` default has been observed correct by a live schema leg AND a
    /// live value leg.
    Verified,
    /// A ledger row (`Gap` or `Divergent`) covers this pair — a live finding,
    /// checked ahead of the registry's own verdict because a pair the
    /// registry states `Native` for, that a sweep found does not actually
    /// work, is a finding, not a pass.
    Gap,
    /// No explicit verdict, no ledger row, and the dialect carries no audit
    /// leg to back the implicit `Native` default. The hole this census
    /// exists to enumerate.
    Unverified,
}

/// Whether `ledger` carries a `Gap` or `Divergent` row for `(name, dialect,
/// position)`. A row with no `position` of its own (`None`) exempts every
/// position — the same convention every other ledger consumer in this crate
/// follows.
fn has_ledger_row(
    ledger: &[LedgerRow],
    name: &str,
    dialect: DialectId,
    position: Position,
) -> bool {
    ledger.iter().any(|row| {
        row.name == name
            && row.dialect == dialect
            && (row.position.is_none() || row.position == Some(position))
            && matches!(row.verdict, Verdict::Gap { .. } | Verdict::Divergent { .. })
    })
}

/// Classify one `(dialect, entry, position)` pair.
pub fn classify(
    dialect: DialectId,
    sig: &Signature,
    position: Position,
    ledger: &[LedgerRow],
) -> Coverage {
    if has_ledger_row(ledger, &sig.name, dialect, position) {
        return Coverage::Gap;
    }
    if sig.stated_emission_at(dialect, position).is_some() {
        return Coverage::Stated;
    }
    if BOTH_LEGS_LIVE.contains(&dialect) {
        Coverage::Verified
    } else {
        Coverage::Unverified
    }
}

/// One `Unverified` pair, named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CensusRow {
    pub name: String,
    pub position: Position,
}

impl CensusRow {
    /// The census file's line format: `<NAME> <position>`.
    pub fn key(&self) -> String {
        format!("{} {}", self.name, position_label(self.position))
    }
}

/// Every `Unverified` pair for `dialect`, over `entries` — positions come
/// from `report::applicable_positions`, the same axis the coverage table
/// renders, so this maintains no position axis of its own.
pub fn census_for<'a>(
    dialect: DialectId,
    entries: impl Iterator<Item = &'a Signature>,
    ledger: &[LedgerRow],
) -> Vec<CensusRow> {
    let mut rows = Vec::new();
    for sig in entries {
        for &position in applicable_positions(sig.kind) {
            if classify(dialect, sig, position, ledger) == Coverage::Unverified {
                rows.push(CensusRow {
                    name: sig.name.clone(),
                    position,
                });
            }
        }
    }
    rows
}

const CENSUS_HEADER: &str = "\
# Trino emission census — shrink-only, docs/outcomes/20260913-trino-emission
# phase 2 (crates/smelt-db/tests/dialect_audit/census.rs).
#
# Every (entry, position) pair below carries no explicit DialectId::Trino
# emission verdict and no Trino audit-leg observation — Coverage::Unverified,
# docs/specs/multi_backend.md \u{00a7}\"Cross-engine emission audit\". This file
# is two-sided, exactly like .claude/dialect-gaps-baseline.txt: a pair absent
# from this file fails immediately (a newly-added built-in can never
# silently acquire a Trino claim), and a row that has since gained a verdict
# or an audit leg is a STALE CENSUS failure telling you to tighten it. Owned
# by phases 3-6 of the outcome above, which drive the count to zero; the
# file is DELETED, not grandfathered, once it reaches zero, and Unverified
# reverts to a plain failure for Trino.
#
# Format: <NAME> <position>
";

fn census_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.claude/trino-emission-census.txt")
}

fn read_census() -> Vec<String> {
    let content = std::fs::read_to_string(census_path()).unwrap_or_default();
    content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

fn write_census(keys: &[String]) {
    let mut out = CENSUS_HEADER.to_string();
    for k in keys {
        out.push_str(k);
        out.push('\n');
    }
    std::fs::write(census_path(), out).expect("write census file");
}

/// The two-sided diff between what the registry says is `Unverified` today
/// (`actual`) and what the census file records (`recorded`): pairs in
/// `actual` but not `recorded` are missing (a newly-unstated pair with no
/// census row yet), pairs in `recorded` but not `actual` are stale (recorded
/// as unverified but no longer is).
fn diff(actual: &[String], recorded: &[String]) -> (Vec<String>, Vec<String>) {
    let actual_set: HashSet<&str> = actual.iter().map(String::as_str).collect();
    let recorded_set: HashSet<&str> = recorded.iter().map(String::as_str).collect();
    let mut missing: Vec<String> = actual_set
        .difference(&recorded_set)
        .map(|s| s.to_string())
        .collect();
    let mut stale: Vec<String> = recorded_set
        .difference(&actual_set)
        .map(|s| s.to_string())
        .collect();
    missing.sort();
    stale.sort();
    (missing, stale)
}

fn sorted_keys(rows: &[CensusRow]) -> Vec<String> {
    let mut keys: Vec<String> = rows.iter().map(CensusRow::key).collect();
    keys.sort();
    keys.dedup();
    keys
}

// ─────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_types::signatures::SigParam;
    use smelt_types::{BuiltinRegistry, DataType, Emission, TypeConstraint, TypeExpr};

    fn test_signature(name: &str) -> Signature {
        Signature::new(
            name,
            vec![],
            vec![SigParam::Concrete(TypeConstraint::Concrete(
                DataType::Integer,
            ))],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Integer)),
        )
    }

    #[test]
    fn census_classifies_a_stated_verdict_as_stated() {
        let sig = test_signature("TEST_STATED").with_emission(&[(
            DialectId::Trino,
            Position::Scalar,
            Emission::Rename("SOME_TRINO_NAME"),
        )]);
        assert_eq!(
            classify(DialectId::Trino, &sig, Position::Scalar, &[]),
            Coverage::Stated
        );
    }

    #[test]
    fn census_classifies_an_audited_dialect_as_verified() {
        let sig = test_signature("TEST_UNSTATED");
        assert_eq!(
            classify(DialectId::DuckDb, &sig, Position::Scalar, &[]),
            Coverage::Verified,
            "DuckDb is in AUDITED_DIALECTS"
        );
        assert_eq!(
            classify(DialectId::Trino, &sig, Position::Scalar, &[]),
            Coverage::Unverified,
            "Trino has no audit leg yet"
        );
    }

    #[test]
    fn census_classifies_a_ledger_row_as_gap() {
        let sig = test_signature("TEST_GAPPED");
        let ledger = [LedgerRow {
            name: "TEST_GAPPED",
            dialect: DialectId::Trino,
            position: None,
            arm: None,
            leg: crate::ledger::Leg::Schema,
            verdict: Verdict::Gap {
                issue: "#0",
                detail: "test row",
            },
        }];
        assert_eq!(
            classify(DialectId::Trino, &sig, Position::Scalar, &ledger),
            Coverage::Gap
        );
        let divergent = [LedgerRow {
            name: "TEST_GAPPED",
            dialect: DialectId::Trino,
            position: None,
            arm: None,
            leg: crate::ledger::Leg::Value,
            verdict: Verdict::Divergent { reason: "test" },
        }];
        assert_eq!(
            classify(DialectId::Trino, &sig, Position::Scalar, &divergent),
            Coverage::Gap
        );
    }

    #[test]
    fn unverified_pairs_exist_only_for_dialects_without_both_legs() {
        let entries: Vec<&Signature> = BuiltinRegistry::names()
            .filter_map(BuiltinRegistry::resolve)
            .collect();
        for dialect in BOTH_LEGS_LIVE {
            let rows = census_for(
                *dialect,
                entries.iter().copied(),
                crate::ledger::dialect_divergences(),
            );
            assert!(
                rows.is_empty(),
                "{} has both legs live but has Unverified pairs: {:?}",
                dialect.slug(),
                sorted_keys(&rows)
            );
        }
    }

    /// The regression guard for the `Verified`-becomes-leg-aware flip: joining
    /// `AUDITED_DIALECTS` with only a schema leg must not make a dialect's
    /// unstated pairs read as `Verified`.
    #[test]
    fn a_schema_only_dialect_is_not_yet_verified() {
        assert!(
            AUDITED_DIALECTS.contains(&DialectId::Trino),
            "Trino must be in AUDITED_DIALECTS (schema leg is live)"
        );
        assert!(
            !BOTH_LEGS_LIVE.contains(&DialectId::Trino),
            "Trino must not be in BOTH_LEGS_LIVE yet (no value leg until phase 6)"
        );
        let sig = test_signature("TEST_SCHEMA_ONLY_UNSTATED");
        assert_eq!(
            classify(DialectId::Trino, &sig, Position::Scalar, &[]),
            Coverage::Unverified,
            "a schema-only audit leg must not flip an unstated pair to Verified"
        );
    }

    #[test]
    fn the_trino_census_matches_the_registry_exactly() {
        let entries: Vec<&Signature> = BuiltinRegistry::names()
            .filter_map(BuiltinRegistry::resolve)
            .collect();
        let actual = census_for(
            DialectId::Trino,
            entries.iter().copied(),
            crate::ledger::dialect_divergences(),
        );
        let actual_keys = sorted_keys(&actual);

        if std::env::var("SMELT_REGEN_TRINO_CENSUS").as_deref() == Ok("1") {
            write_census(&actual_keys);
            return;
        }

        let recorded = read_census();
        let (missing, stale) = diff(&actual_keys, &recorded);
        assert!(
            missing.is_empty(),
            "{} Unverified Trino pair(s) are not named in \
             .claude/trino-emission-census.txt. Regenerate with:\n  \
             SMELT_REGEN_TRINO_CENSUS=1 cargo test -p smelt-db --test dialect_audit \
             the_trino_census_matches_the_registry_exactly\n{}",
            missing.len(),
            missing.join("\n")
        );
        assert!(
            stale.is_empty(),
            "{} row(s) in .claude/trino-emission-census.txt no longer classify as \
             Unverified — tighten the census. Regenerate with:\n  \
             SMELT_REGEN_TRINO_CENSUS=1 cargo test -p smelt-db --test dialect_audit \
             the_trino_census_matches_the_registry_exactly\n{}",
            stale.len(),
            stale.join("\n")
        );
    }

    /// The red-proof self-test: a synthetic entry absent from the census must
    /// be reported by name, mirroring
    /// `hardening_budget::gate_detects_regression`.
    #[test]
    fn census_gate_detects_a_new_unstated_entry() {
        let sig = test_signature("TEST_NEW_UNSTATED_ENTRY");
        let rows = census_for(DialectId::Trino, std::iter::once(&sig), &[]);
        let actual_keys = sorted_keys(&rows);
        let recorded: Vec<String> = Vec::new();
        let (missing, stale) = diff(&actual_keys, &recorded);
        assert_eq!(missing, vec!["TEST_NEW_UNSTATED_ENTRY scalar".to_string()]);
        assert!(stale.is_empty());
    }

    #[test]
    fn every_census_row_names_a_real_registry_entry_and_position() {
        let recorded = read_census();
        for line in &recorded {
            let (name, position) = line
                .rsplit_once(' ')
                .unwrap_or_else(|| panic!("malformed census line: {line}"));
            let sig = BuiltinRegistry::resolve(name)
                .unwrap_or_else(|| panic!("census names {name}, which the registry does not have"));
            let valid_positions: Vec<&str> = applicable_positions(sig.kind)
                .iter()
                .map(|p| position_label(*p))
                .collect();
            assert!(
                valid_positions.contains(&position),
                "census names {name} at position {position}, which its ExprKind {:?} cannot \
                 occupy (valid: {valid_positions:?})",
                sig.kind
            );
        }
    }

    #[test]
    fn the_census_header_states_the_shrink_only_rule() {
        assert!(
            CENSUS_HEADER.contains("shrink-only")
                || CENSUS_HEADER.to_lowercase().contains("shrink only")
        );
        assert!(CENSUS_HEADER.contains("20260913-trino-emission"));
        assert!(
            CENSUS_HEADER.contains("STALE CENSUS")
                || CENSUS_HEADER.to_lowercase().contains("stale")
        );
    }
}

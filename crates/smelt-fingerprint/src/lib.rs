//! Semantic output-fingerprint for smelt models.
//!
//! Stage 0 of the virtual-environments work (see
//! `docs/research/20260601-virtual-environments.md` §8). Given a model's
//! expanded, typed CST, [`output_fingerprint`] computes a hash over a
//! **canonical normal form** of the query. Two model versions with the same
//! fingerprint are proven to compute the same relation (same multiset of rows,
//! columns matched by name) for the same inputs — so a downstream environment
//! could point at the existing physical table instead of rebuilding.
//!
//! The single load-bearing invariant is **soundness**: a fingerprint match must
//! never be a false positive. Every canonicalisation rule is output-preserving,
//! and anything the canonicaliser cannot prove safe falls back to a verbatim
//! hash (so any change re-fingerprints). Completeness — recognising *more*
//! refactors as equivalent — grows rule by rule, each gated by the DuckDB
//! oracle property test in `tests/`.
//!
//! No state store, no environments, no cross-model lineage live here: this is a
//! single-model judgement.

mod canonical;
mod hash;

use smelt_parser::ast::SelectStmt;

/// A model's semantic output fingerprint — a SHA-256 digest over the canonical
/// normal form of its query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint(pub [u8; 32]);

impl Fingerprint {
    /// Lowercase hex rendering, useful in diagnostics and golden tests.
    pub fn to_hex(self) -> String {
        let mut s = String::with_capacity(64);
        for byte in self.0 {
            s.push_str(&format!("{byte:02x}"));
        }
        s
    }
}

/// A place where the canonicaliser conservatively declined to recognise an
/// equivalence (e.g. a non-inlinable CTE, observable projection order, inline
/// non-determinism). Recorded so a later stage can quantify the completeness
/// gap; never affects soundness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissedReuse {
    pub reason: String,
}

/// Result of fingerprinting a model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerprintResult {
    pub fingerprint: Fingerprint,
    /// `false` when the canonicaliser fell back to a verbatim hash because it
    /// could not prove the query safe to canonicalise.
    pub canonicalisable: bool,
    pub missed_reuse: Vec<MissedReuse>,
}

/// Compute the output fingerprint of an (already function-expanded) model
/// `SELECT`.
///
/// `output_schema` is an optional list of `(column_name, type_rendering)` pairs;
/// when supplied it is folded into the fingerprint so a type-only change is
/// detected. Pass `&[]` to fingerprint structure alone — which is already sound,
/// because identical canonical structure denotes an identical query over
/// identical inputs.
pub fn output_fingerprint(
    expanded_select: &SelectStmt,
    output_schema: &[(String, String)],
) -> FingerprintResult {
    let built = canonical::build(expanded_select, output_schema);
    let canonicalisable = matches!(built.canon, canonical::Canon::Structured(_));
    FingerprintResult {
        fingerprint: Fingerprint(built.canon.fingerprint()),
        canonicalisable,
        missed_reuse: built.missed,
    }
}

/// Parse `sql` as a single SELECT model body and fingerprint it. Convenience for
/// callers/tests that start from raw (expanded) SQL text. Returns `None` if the
/// text does not parse to a `SELECT`.
pub fn output_fingerprint_from_sql(
    sql: &str,
    output_schema: &[(String, String)],
) -> Option<FingerprintResult> {
    let parse = smelt_parser::parse(sql);
    let select = parse.syntax().descendants().find_map(SelectStmt::cast)?;
    Some(output_fingerprint(&select, output_schema))
}

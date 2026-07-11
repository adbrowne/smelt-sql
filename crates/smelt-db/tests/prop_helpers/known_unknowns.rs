//! Registry of expression shapes for which smelt is *expected* to infer
//! `Unknown(Dynamic)` — the type-side analogue of `divergences.rs`.
//!
//! The strict property oracle treats an inferred `Unknown(Dynamic)` as a failure
//! by default: a column smelt cannot type is a coverage hole, not a free pass.
//! An `Unknown` is only tolerated when the *generating expression* matches a
//! registered entry here, so every hole is named, greppable, and carries a
//! reason and a tracking note.
//!
//! Entries are keyed by expression shape via `markers`: a set of substrings that
//! must **all** appear in the generating expression's SQL. A single marker keys
//! on one construct (`["->"]`); multiple markers pin a combination
//! (`["dec_col", "/"]` = a division with a decimal operand) without a full
//! parser. Entries that never fire over a run are reported at warn level
//! (staleness pressure): a hole that has since been closed should have its entry
//! deleted.

/// Why smelt infers `Unknown` for this expression shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownStatus {
    /// A gap in smelt's inference we plan to close (the registry keeps the
    /// property test green until then).
    #[allow(dead_code)]
    KnownGap,
    /// Intentional: the result type is genuinely dynamic / not portably known.
    ByDesign,
}

/// A registered expectation that smelt infers `Unknown` for a shape of
/// expression.
#[derive(Debug)]
pub struct KnownUnknown {
    pub id: &'static str,
    pub description: &'static str,
    /// Substrings that must **all** appear in the generating expression's SQL
    /// (function name, operator token, column-family prefix, …).
    pub markers: &'static [&'static str],
    pub status: UnknownStatus,
}

/// All registered known-unknown expression shapes.
pub fn known_unknowns() -> Vec<KnownUnknown> {
    vec![
        // The JSON arrow operators (`->`, `->>`) return an untyped JSON value in
        // DuckDB; smelt does not statically resolve the extracted type.
        KnownUnknown {
            id: "json_arrow_operator",
            description: "col -> 'k' / col ->> 'k' — JSON extraction has no statically \
                known result type; DuckDB returns JSON/VARCHAR at runtime.",
            markers: &["->"],
            status: UnknownStatus::ByDesign,
        },
        // Decimal division is intentionally rejected (spec §15): engines disagree
        // on the result family/precision, so smelt returns Unknown and emits a
        // TypeMismatch diagnostic rather than committing to a non-portable type.
        KnownUnknown {
            id: "decimal_division",
            description: "decimal / x or x / decimal — division with a Decimal operand is \
                not in the portable surface (spec §15); smelt returns Unknown and \
                diagnoses it rather than guess a non-portable Decimal result. Keyed on the \
                spaced division token: the entry is only consulted when smelt already inferred \
                Unknown, and non-decimal division is typed (Double), so a ` / `-bearing \
                Unknown is necessarily a decimal division. Column names differ across \
                single-model (`dec_col`) and cross-model (`up_0`, `b_N`) scenarios, so a \
                name-based marker cannot cover both — hence the operator token. The spacing \
                avoids matching any non-operator `/`.",
            markers: &[" / "],
            status: UnknownStatus::ByDesign,
        },
        KnownUnknown {
            id: "decimal_multiply_overflow",
            description: "decimal * decimal whose Spark-style growth precision exceeds 38 \
                (spec §15) — smelt returns Unknown and emits a Decimal-precision-overflow \
                diagnostic rather than silently truncating, while DuckDB clamps to a native \
                DECIMAL. Reached in cross-model chains where an already-widened decimal (e.g. \
                DECIMAL(21,4) from a prior `*`) is multiplied again (p'=43>38). Keyed on the \
                spaced multiply token — distinct from `COUNT(*)`, which has no surrounding \
                spaces — under the is_unknown() guard (non-overflowing multiply is a typed \
                Decimal, integer multiply is a typed Integer).",
            markers: &[" * "],
            status: UnknownStatus::ByDesign,
        },
    ]
}

/// Find the registered known-unknown entry, if any, all of whose `markers`
/// appear in the generating expression's SQL.
pub fn find_known_unknown<'a>(
    expr_sql: &str,
    entries: &'a [KnownUnknown],
) -> Option<&'a KnownUnknown> {
    entries
        .iter()
        .find(|e| e.markers.iter().all(|m| expr_sql.contains(m)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_arrow_is_registered() {
        let entries = known_unknowns();
        let found = find_known_unknown("col -> 'a'", &entries);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "json_arrow_operator");
    }

    #[test]
    fn division_token_matches_decimal_division() {
        let entries = known_unknowns();
        // The ` / ` token keys the entry (precision comes from the is_unknown()
        // guard at the call site: only decimal division yields Unknown).
        assert_eq!(
            find_known_unknown("dec_col_0 / dec_col_1", &entries)
                .unwrap()
                .id,
            "decimal_division"
        );
        assert_eq!(
            find_known_unknown("up_0 / up_0", &entries).unwrap().id,
            "decimal_division"
        );
    }

    #[test]
    fn multiply_token_matches_overflow_but_not_count_star() {
        let entries = known_unknowns();
        assert_eq!(
            find_known_unknown("up_0 * up_0", &entries).unwrap().id,
            "decimal_multiply_overflow"
        );
        // COUNT(*) must NOT match the multiply-overflow entry.
        assert!(find_known_unknown("COUNT(*)", &entries).is_none());
    }

    #[test]
    fn unregistered_shape_is_none() {
        let entries = known_unknowns();
        assert!(find_known_unknown("UPPER(str_col)", &entries).is_none());
    }

    #[test]
    fn entry_ids_are_unique() {
        let entries = known_unknowns();
        let mut ids: Vec<&str> = entries.iter().map(|e| e.id).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate known-unknown id");
    }
}

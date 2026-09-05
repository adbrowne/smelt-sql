//! Conservative "provably NOT NULL from a key's first stored row" derivation
//! (`docs/specs/incremental_shapes.md` §"Key temporal locality", structural
//! precondition 2) for a **non-key** `timeseries.partition_column`.
//!
//! Lives under `analysis/` (rather than `maintenance/locality.rs`, which
//! consumes it) per `docs/specs/architecture.md` §"Property composition walk
//! rule": this is a **leaf classifier** — it operates over one already-
//! bounded node's own text (the model's own `SELECT` list), never composes
//! across nodes, and never re-derives a property the shared walk
//! (`analysis::walk`) already owns. It is invoked by both of
//! [`crate::maintenance::locality::establish_locality`]'s callers
//! (`smelt-db`'s static plan-derivation query and `smelt-runtime`'s keyed
//! execution loop — `crate::maintenance::locality::resolve_driving_source`'s
//! doc comment gives the same reasoning for factoring resolution once rather
//! than duplicating it per caller): a model `smelt-db` admits through the
//! locality gate must also be admitted by `smelt-runtime` when it actually
//! executes, or the run would fail on a model `smelt build --dry-run`/`smelt
//! explain` reported as valid.
//!
//! A wrong or missing derivation here only ever narrows to "not proven",
//! never fabricates a proof `establish_locality` doesn't independently gate
//! — this is input assembly, not admission.

use crate::analysis::{select_stmt_items, SelectItemKind};

/// A `unique_key` column is non-null within its own group by construction —
/// the key never changes across merges (the same fact `classify_once_write`
/// route 1 already leans on for the bare key-derived spelling). Leaf
/// classifier: a case-insensitive membership check over the model's own
/// declared `unique_key`, nothing more.
pub fn column_provably_not_null(unique_key: &[String], column: &str) -> bool {
    unique_key.iter().any(|k| k.eq_ignore_ascii_case(column))
}

/// Two cases are recognised:
/// - `partition_column` is itself a `unique_key` column (route 1's own
///   fact — a key component is never meaningfully NULL).
/// - `partition_column` is a direct `MIN`/`MAX` aggregate over the driving
///   source's own partition column — that column is axiomatically NOT NULL
///   (every clocked source's partition column is assumed populated;
///   `analysis::source_bounds` and the classifier both already lean on
///   this), so the aggregate over it inherits non-nullness for any key that
///   has at least one stored row.
/// - `partition_column` is a direct, non-null-preserving scalar wrapper
///   (`DATE_TRUNC`, a non-`TRY_CAST` `CAST`) around the driving source's own
///   clock column, checked structurally against the parsed expression tree
///   ([`expr_is_driving_clock_or_wrapper`]) rather than by scanning the
///   projection's rendered text — a text scan cannot distinguish a bare
///   reference to the clock column from a NULL-producing conditional
///   branch that merely *mentions* it (e.g. `CASE WHEN flag THEN NULL ELSE
///   event_ts END`), which would unsoundly prove NOT NULL for a column that
///   is not.
///
/// Note: this only proves *non-nullness*; it says nothing about whether the
/// value is a per-key constant across merges over time (once-write
/// provenance is a distinct proof — `docs/specs/incremental_models.md`
/// §"Key temporal locality", route 2 — consumed independently by
/// `maintenance::locality::establish_locality`).
///
/// A real, general non-key nullability prover remains future work — this is
/// deliberately the narrow slice route 2's real fixture needs today.
pub fn partition_column_provably_not_null(
    sql: &str,
    unique_key: &[String],
    partition_column: &str,
    driving_source_partition_column: Option<&str>,
) -> bool {
    if column_provably_not_null(unique_key, partition_column) {
        return true;
    }
    let Some(driving_col) = driving_source_partition_column else {
        return false;
    };
    let parse = smelt_parser::parse(sql);
    let Some(file) = smelt_parser::File::cast(parse.syntax()) else {
        return false;
    };
    let Some(select) = file.select_stmt() else {
        return false;
    };
    let Some(items) = select_stmt_items(&select) else {
        return false;
    };
    for item in &items {
        match item {
            SelectItemKind::OtherAggregate { alias, expr, .. } => {
                if !alias.eq_ignore_ascii_case(partition_column) {
                    continue;
                }
                let Some(func) = expr.as_function_call() else {
                    continue;
                };
                let Some(name) = func.name() else { continue };
                if !name.eq_ignore_ascii_case("MIN") && !name.eq_ignore_ascii_case("MAX") {
                    continue;
                }
                let args = func.arguments();
                if args.len() != 1 {
                    continue;
                }
                if expr_is_driving_clock_or_wrapper(&args[0], driving_col) {
                    return true;
                }
            }
            SelectItemKind::GroupByKey { alias, expr, .. }
                if alias.eq_ignore_ascii_case(partition_column)
                    && expr_is_driving_clock_or_wrapper(expr, driving_col) =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

/// Whether `expr` is exactly the driving source's own clock column, or a
/// direct non-null-preserving scalar wrapper around it (`DATE_TRUNC(unit,
/// col)`, a plain `CAST(col AS type)`/`col::type` — but not `TRY_CAST`,
/// which returns NULL on a failed conversion). A structural check over the
/// parsed expression tree, not a text scan: it recurses only through
/// wrapper shapes it explicitly recognises, so a conditional expression
/// that merely mentions the column's name in a branch that does not
/// evaluate to it (e.g. a `CASE`/`COALESCE` with a `NULL` branch) never
/// matches.
fn expr_is_driving_clock_or_wrapper(expr: &smelt_parser::Expr, driving_col: &str) -> bool {
    if let Some(col_ref) = expr.as_column_ref() {
        return col_ref.name().eq_ignore_ascii_case(driving_col);
    }
    if let Some(cast) = expr.as_cast() {
        if cast.is_try_cast() {
            return false;
        }
        return match cast.expression() {
            Some(inner) => expr_is_driving_clock_or_wrapper(&inner, driving_col),
            None => false,
        };
    }
    if let Some(func) = expr.as_function_call() {
        if let Some(name) = func.name() {
            if name.eq_ignore_ascii_case("DATE_TRUNC") {
                let args = func.arguments();
                if let Some(last) = args.last() {
                    return expr_is_driving_clock_or_wrapper(last, driving_col);
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_provably_not_null_true_for_key_member() {
        assert!(column_provably_not_null(&["id".to_string()], "ID"));
    }

    #[test]
    fn column_provably_not_null_false_for_non_key() {
        assert!(!column_provably_not_null(&["id".to_string()], "val"));
    }

    #[test]
    fn key_column_is_provably_not_null() {
        assert!(partition_column_provably_not_null(
            "SELECT event_id, event_date FROM smelt.sources.raw.events",
            &["event_date".to_string()],
            "event_date",
            Some("event_date"),
        ));
    }

    #[test]
    fn min_over_driving_clock_is_provably_not_null() {
        let sql = "SELECT event_id, MIN(event_date) AS first_seen_date FROM \
                   smelt.sources.raw.events GROUP BY event_id";
        assert!(partition_column_provably_not_null(
            sql,
            &["event_id".to_string()],
            "first_seen_date",
            Some("event_date"),
        ));
    }

    #[test]
    fn date_trunc_wrapper_over_driving_clock_is_provably_not_null() {
        let sql = "SELECT event_id, DATE_TRUNC('day', event_date) AS day FROM \
                   smelt.sources.raw.events";
        assert!(partition_column_provably_not_null(
            sql,
            &["event_id".to_string()],
            "day",
            Some("event_date"),
        ));
    }

    /// Regression test (reviewer-found soundness gap): a `CASE` expression
    /// that merely *mentions* the driving clock column in one branch, but
    /// produces `NULL` on another, must not be proven NOT NULL — the old
    /// substring-containment check was fooled by this shape because it only
    /// checked whether the identifier appeared anywhere in the rendered
    /// text, not whether the expression structurally reduces to a bare
    /// reference (or a recognised non-null-preserving wrapper).
    #[test]
    fn case_with_null_branch_mentioning_driving_clock_is_not_proven_not_null() {
        let sql = "SELECT event_id, CASE WHEN flag THEN NULL ELSE event_date END AS \
                   first_seen_date FROM smelt.sources.raw.events GROUP BY event_id, flag";
        assert!(!partition_column_provably_not_null(
            sql,
            &["event_id".to_string()],
            "first_seen_date",
            Some("event_date"),
        ));
    }

    #[test]
    fn try_cast_over_driving_clock_is_not_proven_not_null() {
        let sql = "SELECT event_id, TRY_CAST(event_date AS DATE) AS day FROM \
                   smelt.sources.raw.events";
        assert!(!partition_column_provably_not_null(
            sql,
            &["event_id".to_string()],
            "day",
            Some("event_date"),
        ));
    }

    #[test]
    fn unrelated_column_is_not_proven_not_null() {
        let sql = "SELECT event_id, MAX(other_column) AS thing FROM \
                   smelt.sources.raw.events GROUP BY event_id";
        assert!(!partition_column_provably_not_null(
            sql,
            &["event_id".to_string()],
            "thing",
            Some("event_date"),
        ));
    }

    #[test]
    fn no_driving_source_partition_column_is_not_proven_not_null() {
        let sql = "SELECT event_id, MIN(event_date) AS first_seen_date FROM \
                   smelt.sources.raw.events GROUP BY event_id";
        assert!(!partition_column_provably_not_null(
            sql,
            &["event_id".to_string()],
            "first_seen_date",
            None,
        ));
    }
}

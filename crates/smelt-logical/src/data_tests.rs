//! Declarative column-test proof — the resolution order's "consult derived
//! properties first" step (`docs/specs/data_tests.md` §Semantics
//! "Resolution order").
//!
//! Pure data + pure functions; no Salsa dependency (Salsa purity rule,
//! `docs/specs/architecture.md` §"Salsa purity rule (analysis)"). Given
//! already-derived facts about a model (a column's inferred nullability, the
//! model's known key column sets), decides whether a `not_null`/`unique`
//! declarative column test is **proven** (no scan needed) or must fall
//! through to a scan. `accepted_values`/`relationships` have no proof path
//! today — see `docs/specs/data_tests.md` §"Known Divergences".
//!
//! A proof may only remove a scan, never suppress a failure
//! (`docs/specs/data_tests.md` §"Proof is a scan-elimination, never a
//! failure-suppression"): every function here is fail-safe by construction —
//! an undecidable or absent input resolves to [`TestVerdict::NeedsScan`],
//! never to a claimed proof.

use smelt_core::metadata::ColumnTest;
use std::collections::BTreeSet;

/// Verdict for a single declarative column test's proof step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestVerdict {
    /// The model's derived properties prove the test true at compile time;
    /// no scan is emitted.
    Proven,
    /// The proof step could not decide the test from derived properties; it
    /// must lower to a failing-rows scan.
    NeedsScan,
}

/// Resolve a `not_null` test's verdict from the tested column's inferred
/// nullability.
///
/// `is_non_nullable` should come from the model's inferred output schema
/// (`docs/specs/model_properties.md`'s nullability analysis) for the tested
/// column — `Some(true)` when the column is proven non-nullable, `Some(false)`
/// when it is known nullable, and `None` when the column's nullability is
/// undecidable (e.g. absent from the schema, or the schema doesn't track a
/// reliable source for it). Only a positive `Some(true)` proves the test;
/// every other input falls through to a scan.
pub fn resolve_not_null_verdict(is_non_nullable: Option<bool>) -> TestVerdict {
    match is_non_nullable {
        Some(true) => TestVerdict::Proven,
        _ => TestVerdict::NeedsScan,
    }
}

/// Resolve a `unique` test's verdict for a (possibly composite) column set.
///
/// Proven when `test_columns` is exactly one of the model's known key sets —
/// order-insensitive, set-equal comparison. `known_key_sets` is the model's
/// declared/proven grain key column sets (today: the declared `unique_key:`
/// fact; a future walk-proven grain/functional-dependency key set may extend
/// this list — `docs/specs/data_tests.md` §Semantics). An empty
/// `test_columns` or no matching key set falls through to a scan.
pub fn resolve_unique_verdict(
    test_columns: &[String],
    known_key_sets: &[Vec<String>],
) -> TestVerdict {
    if test_columns.is_empty() {
        return TestVerdict::NeedsScan;
    }
    let test_set: BTreeSet<&str> = test_columns.iter().map(String::as_str).collect();
    let proven = known_key_sets.iter().any(|key_set| {
        let key_set: BTreeSet<&str> = key_set.iter().map(String::as_str).collect();
        key_set == test_set
    });
    if proven {
        TestVerdict::Proven
    } else {
        TestVerdict::NeedsScan
    }
}

// ── Scan lowering (step 2: "Lower to a scan when unproven") ────────────────

/// One unproven declarative column test, lowered to a failing-rows SELECT
/// (`docs/specs/data_tests.md` §Semantics step 2). A pure text emitter — no
/// backend or Salsa dependency. The caller wraps `failing_rows_sql` in a
/// `smelt.check <name> AS (...)` declaration and drives it through the
/// existing `smelt_runtime::run_single_check` machinery exactly like a
/// hand-authored `smelt.check` (run-pipeline parity rule,
/// `docs/specs/architecture.md` §"Run pipeline parity rule (CLI ↔ UI)") —
/// this module never executes SQL itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanLowering {
    /// `"not_null"` | `"unique"` | `"accepted_values"` | `"relationships"`.
    pub kind: &'static str,
    /// The failing-rows SELECT body (no `smelt.check ... AS (...)` wrapper).
    /// Zero rows = the test passes. References upstream models with the same
    /// `smelt.<model>` addressing a hand-authored check uses, so ref
    /// extraction, `CheckTargetNotBuilt` detection, and compilation all go
    /// through the same path a real check does.
    pub failing_rows_sql: String,
}

/// Lower one already-validated `ColumnTest` to its failing-rows scan.
///
/// Every test that reaches this function is expected to already have passed
/// `smelt_core::metadata::validate_column_tests` (unknown kinds and malformed
/// parameterized shapes are hard diagnostics raised earlier,
/// `docs/specs/data_tests.md` §"Fail-loud validation"). This function stays
/// fail-safe regardless: a shape it cannot lower returns `Err` naming the
/// column and the problem, rather than emitting SQL that silently tests
/// nothing or panicking.
pub fn lower_column_test(
    model_name: &str,
    column: &str,
    test: &ColumnTest,
) -> Result<ScanLowering, String> {
    match test {
        ColumnTest::Simple(kind) if kind == "not_null" => Ok(ScanLowering {
            kind: "not_null",
            failing_rows_sql: format!("SELECT * FROM smelt.{model_name} WHERE {column} IS NULL"),
        }),
        ColumnTest::Simple(kind) if kind == "unique" => Ok(ScanLowering {
            kind: "unique",
            failing_rows_sql: format!(
                "SELECT {column} FROM smelt.{model_name} GROUP BY {column} HAVING COUNT(*) > 1"
            ),
        }),
        ColumnTest::Simple(other) => Err(format!(
            "unrecognized column test kind '{other}' on column '{column}'"
        )),
        ColumnTest::Parameterized(params) => {
            if params.len() != 1 {
                return Err(format!(
                    "unsupported parameterized test shape on column '{column}'"
                ));
            }
            let Some((param_kind, value)) = params.iter().next() else {
                return Err(format!(
                    "empty parameterized test entry on column '{column}'"
                ));
            };
            match param_kind.as_str() {
                "accepted_values" => lower_accepted_values(model_name, column, value),
                "relationships" => lower_relationships(model_name, column, value),
                other => Err(format!(
                    "unrecognized column test kind '{other}' on column '{column}'"
                )),
            }
        }
    }
}

fn lower_accepted_values(
    model_name: &str,
    column: &str,
    value: &serde_yaml::Value,
) -> Result<ScanLowering, String> {
    let Some(seq) = value.as_sequence() else {
        return Err(format!(
            "accepted_values on column '{column}' must be a list"
        ));
    };
    if seq.is_empty() {
        return Err(format!(
            "accepted_values on column '{column}' must be non-empty"
        ));
    }
    let mut literals = Vec::with_capacity(seq.len());
    for item in seq {
        match render_scalar_literal(item) {
            Some(lit) => literals.push(lit),
            None => {
                return Err(format!(
                    "accepted_values on column '{column}' contains an unsupported literal: {item:?}"
                ))
            }
        }
    }
    Ok(ScanLowering {
        kind: "accepted_values",
        failing_rows_sql: format!(
            "SELECT * FROM smelt.{model_name} WHERE {column} IS NOT NULL AND {column} NOT IN ({})",
            literals.join(", ")
        ),
    })
}

fn lower_relationships(
    model_name: &str,
    column: &str,
    value: &serde_yaml::Value,
) -> Result<ScanLowering, String> {
    let mapping = value.as_mapping();
    let to = mapping
        .and_then(|m| m.get(serde_yaml::Value::String("to".to_string())))
        .and_then(|v| v.as_str());
    let field = mapping
        .and_then(|m| m.get(serde_yaml::Value::String("field".to_string())))
        .and_then(|v| v.as_str());
    match (to, field) {
        (Some(to), Some(field)) if !to.is_empty() && !field.is_empty() => Ok(ScanLowering {
            kind: "relationships",
            failing_rows_sql: format!(
                "SELECT c.* FROM smelt.{model_name} c WHERE c.{column} IS NOT NULL AND NOT EXISTS \
                 (SELECT 1 FROM smelt.{to} p WHERE p.{field} = c.{column})"
            ),
        }),
        _ => Err(format!(
            "relationships test on column '{column}' must declare `to` and `field`"
        )),
    }
}

/// Render a YAML scalar as a SQL literal. `None` for non-scalar values
/// (sequences/mappings/null), which `lower_accepted_values` treats as an
/// unsupported literal rather than silently coercing.
fn render_scalar_literal(value: &serde_yaml::Value) -> Option<String> {
    match value {
        serde_yaml::Value::String(s) => Some(format!("'{}'", s.replace('\'', "''"))),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(if *b { "TRUE" } else { "FALSE" }.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn not_null_proven_when_schema_says_non_nullable() {
        assert_eq!(resolve_not_null_verdict(Some(true)), TestVerdict::Proven);
    }

    #[test]
    fn not_null_needs_scan_when_nullable() {
        assert_eq!(
            resolve_not_null_verdict(Some(false)),
            TestVerdict::NeedsScan
        );
    }

    #[test]
    fn not_null_needs_scan_when_undecidable() {
        assert_eq!(resolve_not_null_verdict(None), TestVerdict::NeedsScan);
    }

    #[test]
    fn unique_proven_when_matches_declared_key() {
        let verdict = resolve_unique_verdict(&["id".to_string()], &[vec!["id".to_string()]]);
        assert_eq!(verdict, TestVerdict::Proven);
    }

    #[test]
    fn unique_proven_for_composite_key_regardless_of_order() {
        let verdict = resolve_unique_verdict(
            &["b".to_string(), "a".to_string()],
            &[vec!["a".to_string(), "b".to_string()]],
        );
        assert_eq!(verdict, TestVerdict::Proven);
    }

    #[test]
    fn unique_needs_scan_when_no_key_set_matches() {
        let verdict = resolve_unique_verdict(&["email".to_string()], &[vec!["id".to_string()]]);
        assert_eq!(verdict, TestVerdict::NeedsScan);
    }

    #[test]
    fn unique_needs_scan_when_no_known_key_sets() {
        let verdict = resolve_unique_verdict(&["id".to_string()], &[]);
        assert_eq!(verdict, TestVerdict::NeedsScan);
    }

    // ── lower_column_test — generated_sql_is_emitter_authored ──────────────

    #[test]
    fn lower_not_null_emits_is_null_predicate() {
        let test = ColumnTest::Simple("not_null".to_string());
        let lowering = lower_column_test("revenue", "amount", &test).unwrap();
        assert_eq!(lowering.kind, "not_null");
        assert_eq!(
            lowering.failing_rows_sql,
            "SELECT * FROM smelt.revenue WHERE amount IS NULL"
        );
    }

    #[test]
    fn lower_unique_emits_group_by_having_predicate() {
        let test = ColumnTest::Simple("unique".to_string());
        let lowering = lower_column_test("revenue", "order_id", &test).unwrap();
        assert_eq!(lowering.kind, "unique");
        assert_eq!(
            lowering.failing_rows_sql,
            "SELECT order_id FROM smelt.revenue GROUP BY order_id HAVING COUNT(*) > 1"
        );
    }

    #[test]
    fn lower_accepted_values_emits_not_in_predicate() {
        let mut params = BTreeMap::new();
        params.insert(
            "accepted_values".to_string(),
            serde_yaml::Value::Sequence(vec![
                serde_yaml::Value::String("pending".to_string()),
                serde_yaml::Value::String("shipped".to_string()),
            ]),
        );
        let test = ColumnTest::Parameterized(params);
        let lowering = lower_column_test("revenue", "status", &test).unwrap();
        assert_eq!(lowering.kind, "accepted_values");
        assert_eq!(
            lowering.failing_rows_sql,
            "SELECT * FROM smelt.revenue WHERE status IS NOT NULL AND status NOT IN ('pending', 'shipped')"
        );
    }

    #[test]
    fn lower_accepted_values_escapes_quotes_in_string_literals() {
        let mut params = BTreeMap::new();
        params.insert(
            "accepted_values".to_string(),
            serde_yaml::Value::Sequence(vec![serde_yaml::Value::String("O'Brien".to_string())]),
        );
        let test = ColumnTest::Parameterized(params);
        let lowering = lower_column_test("revenue", "status", &test).unwrap();
        assert!(
            lowering.failing_rows_sql.contains("'O''Brien'"),
            "expected escaped literal, got: {}",
            lowering.failing_rows_sql
        );
    }

    #[test]
    fn lower_relationships_emits_not_exists_anti_join() {
        let mut inner = serde_yaml::Mapping::new();
        inner.insert(
            serde_yaml::Value::String("to".to_string()),
            serde_yaml::Value::String("customers".to_string()),
        );
        inner.insert(
            serde_yaml::Value::String("field".to_string()),
            serde_yaml::Value::String("id".to_string()),
        );
        let mut params = BTreeMap::new();
        params.insert(
            "relationships".to_string(),
            serde_yaml::Value::Mapping(inner),
        );
        let test = ColumnTest::Parameterized(params);
        let lowering = lower_column_test("revenue", "customer_id", &test).unwrap();
        assert_eq!(lowering.kind, "relationships");
        assert_eq!(
            lowering.failing_rows_sql,
            "SELECT c.* FROM smelt.revenue c WHERE c.customer_id IS NOT NULL AND NOT EXISTS \
             (SELECT 1 FROM smelt.customers p WHERE p.id = c.customer_id)"
        );
    }

    #[test]
    fn lower_accepted_values_rejects_empty_list() {
        let mut params = BTreeMap::new();
        params.insert(
            "accepted_values".to_string(),
            serde_yaml::Value::Sequence(vec![]),
        );
        let test = ColumnTest::Parameterized(params);
        assert!(lower_column_test("revenue", "status", &test).is_err());
    }

    #[test]
    fn lower_relationships_rejects_missing_field() {
        let mut inner = serde_yaml::Mapping::new();
        inner.insert(
            serde_yaml::Value::String("to".to_string()),
            serde_yaml::Value::String("customers".to_string()),
        );
        let mut params = BTreeMap::new();
        params.insert(
            "relationships".to_string(),
            serde_yaml::Value::Mapping(inner),
        );
        let test = ColumnTest::Parameterized(params);
        assert!(lower_column_test("revenue", "customer_id", &test).is_err());
    }
}

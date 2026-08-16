//! Integration test for `smelt_logical::analysis::affected_keys` — the P7
//! affected-key discovery proof (`docs/specs/model_properties.md`
//! §"Affected-key discovery"). Lives outside the unit-test module because it
//! exercises composition through a CTE rename chain rather than a single
//! top-level scope.

use std::collections::BTreeSet;

use smelt_logical::analysis::affected_keys::{
    derive_affected_keys, AffectedKeyContext, AffectedKeys, DeltaShape,
};

#[test]
fn cte_composed_grain_column_resolves_through_rename_chain() {
    let sql = "WITH base AS (SELECT customer_id, amount FROM smelt.sources.orders) \
               SELECT customer_id, SUM(amount) AS total FROM base GROUP BY customer_id";
    let delta = DeltaShape {
        source: "orders".to_string(),
        columns: ["customer_id".to_string(), "amount".to_string()]
            .into_iter()
            .collect::<BTreeSet<_>>(),
        keyed: true,
    };
    let verdict = derive_affected_keys(&delta, sql, &AffectedKeyContext::default());
    assert_eq!(
        verdict,
        AffectedKeys::Keys {
            cols: vec!["customer_id".to_string()]
        },
        "{verdict:?}"
    );
}

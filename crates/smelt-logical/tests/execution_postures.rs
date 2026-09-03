//! Phase 4 (`docs/outcomes/20260815-keyed-grain-residue`) — the three
//! derived execution postures (`execution_postures`).
//!
//! See `docs/specs/incremental_shapes.md` §"Derived execution postures" for
//! the normative spec these tests pin.

use std::collections::HashMap;

use smelt_core::config::{Granularity, TimeseriesConfig};
use smelt_logical::{classify_cumulative, SourceTimeseriesMap};

fn ts(partition_col: &str) -> TimeseriesConfig {
    TimeseriesConfig {
        event_time_column: partition_col.to_string(),
        partition_column: partition_col.to_string(),
        granularity: Granularity::Day,
        week_start: None,
        assert_monotonic: false,
    }
}

fn events_source_map() -> SourceTimeseriesMap {
    let mut m = HashMap::new();
    m.insert("smelt.silver.events_parsed".to_string(), ts("event_date"));
    m
}

/// An additive-fold (`SUM`) column: re-run tolerance fails, naming the
/// column; order-independence holds — `+` is commutative and associative.
#[test]
fn additive_sum_model_is_not_rerun_tolerant_but_is_order_independent() {
    let sql = r#"SELECT
    device_id,
    SUM(amount) AS total_amount
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let classification =
        classify_cumulative(sql, &refs, &events_source_map(), false, &[]).expect("must classify");

    let postures = classification.execution_postures();
    assert!(!postures.rerun_tolerant.holds);
    assert!(
        postures.rerun_tolerant.reason.contains("total_amount"),
        "reason should name the offending column: {}",
        postures.rerun_tolerant.reason
    );
    assert!(postures.order_independent.holds);
    assert!(postures.reprocessing_refused.holds);
}

/// An order-monotone overwrite (`MAX_BY`) column: re-run tolerant (its
/// state folds via `Min`/`Max`/`OrderMonotone`, never additive), but
/// order-independence fails — the incumbent-wins comparison depends on
/// window arrival order — and the reason names the offending column.
#[test]
fn order_monotone_column_forces_sequential_application() {
    let sql = r#"SELECT
    device_id,
    MAX_BY(status, updated_at) AS status
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let classification =
        classify_cumulative(sql, &refs, &events_source_map(), false, &[]).expect("must classify");

    let postures = classification.execution_postures();
    assert!(postures.rerun_tolerant.holds);
    assert!(!postures.order_independent.holds);
    assert!(
        postures.order_independent.reason.contains("status"),
        "reason should name the offending column: {}",
        postures.order_independent.reason
    );
    assert!(postures.reprocessing_refused.holds);
}

/// A mix of extremal/lattice (`MAX`) and once-write (`COALESCE`,
/// key-derived) columns — neither has an additive combiner or state — is
/// both re-run tolerant and order-independent. Adding a decomposed-fold
/// (`AVG`) column, whose hidden state folds via `Sum`, keeps
/// order-independence (`+` is order-independent) but loses re-run
/// tolerance — the admission matrix grades decomposed fold "ledger-enforced,
/// graded additive", matching `WindowedKeyedRule::ledger_grade`.
#[test]
fn lattice_and_once_write_are_rerun_tolerant_and_order_independent() {
    let sql = r#"SELECT
    device_id,
    MAX(amount) AS max_amount,
    COALESCE(device_id, 'n/a') AS first_seen_device
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let classification =
        classify_cumulative(sql, &refs, &events_source_map(), false, &[]).expect("must classify");

    let postures = classification.execution_postures();
    assert!(postures.rerun_tolerant.holds);
    assert!(postures.order_independent.holds);
    assert!(postures.reprocessing_refused.holds);
}

/// Decomposed fold (`AVG`)'s hidden `Sum` state loses re-run tolerance
/// (matching the admission matrix's "ledger-enforced, graded additive") but
/// keeps order-independence — `+` is commutative and associative regardless
/// of which state column it acts on.
#[test]
fn decomposed_fold_is_order_independent_but_not_rerun_tolerant() {
    let sql = r#"SELECT
    device_id,
    AVG(amount) AS avg_amount
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let classification =
        classify_cumulative(sql, &refs, &events_source_map(), false, &[]).expect("must classify");

    let postures = classification.execution_postures();
    assert!(!postures.rerun_tolerant.holds);
    assert!(
        postures.rerun_tolerant.reason.contains("avg_amount"),
        "reason should name the offending column: {}",
        postures.rerun_tolerant.reason
    );
    assert!(postures.order_independent.holds);
    assert!(postures.reprocessing_refused.holds);
}

/// A plain-overwrite (`ANY_VALUE`) column under snapshot-reconcile: the
/// delta always wins, so order-independence fails.
#[test]
fn plain_overwrite_is_order_dependent() {
    let sql = r#"SELECT
    device_id,
    ANY_VALUE(status) AS status
FROM smelt.silver.lookup_table
GROUP BY device_id"#;
    let refs = vec!["smelt.silver.lookup_table".to_string()];
    let classification = classify_cumulative(sql, &refs, &HashMap::new(), false, &[])
        .expect("plain-overwrite must classify under snapshot-reconcile");

    let postures = classification.execution_postures();
    assert!(!postures.order_independent.holds);
    assert!(
        postures.order_independent.reason.contains("status"),
        "reason should name the offending column: {}",
        postures.order_independent.reason
    );
}

/// Reprocessing refusal holds unconditionally, across every classification
/// above — additive, order-monotone, lattice/once-write/decomposed, and
/// plain-overwrite alike.
#[test]
fn reprocessing_refusal_holds_for_every_family() {
    let additive_sql = r#"SELECT
    device_id,
    SUM(amount) AS total_amount
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let order_monotone_sql = r#"SELECT
    device_id,
    MAX_BY(status, updated_at) AS status
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let lattice_sql = r#"SELECT
    device_id,
    MAX(amount) AS max_amount
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
    let plain_overwrite_sql = r#"SELECT
    device_id,
    ANY_VALUE(status) AS status
FROM smelt.silver.lookup_table
GROUP BY device_id"#;

    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let lookup_refs = vec!["smelt.silver.lookup_table".to_string()];

    for (sql, refs, sources) in [
        (additive_sql, &refs, events_source_map()),
        (order_monotone_sql, &refs, events_source_map()),
        (lattice_sql, &refs, events_source_map()),
        (plain_overwrite_sql, &lookup_refs, HashMap::new()),
    ] {
        let classification =
            classify_cumulative(sql, refs, &sources, false, &[]).expect("must classify");
        assert!(
            classification
                .execution_postures()
                .reprocessing_refused
                .holds
        );
    }
}

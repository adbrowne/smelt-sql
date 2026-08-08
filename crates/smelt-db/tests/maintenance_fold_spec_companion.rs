//! `derive_fold_spec` (`crates/smelt-db/src/queries/maintenance.rs`) must
//! stay in lockstep with the runtime classifier
//! (`smelt_logical::rules::cumulative::classify_order_monotone_column`) on
//! whether an order-monotone (`MAX_BY`/`MIN_BY`, `ArgMax`/`ArgMin`) column
//! is admitted into a fold — otherwise `smelt explain`/LSP diagnostics
//! report a `KeyedFold` cell the runtime then refuses with
//! `KeyedUnknownCombiner` (a fail-loud discipline violation: a compile-time
//! false positive).
//!
//! Spec: `docs/specs/incremental_models.md` §"The column-family catalogue",
//! §"Statement emission (single owner)".

use smelt_db::queries::maintenance::derive_fold_spec;
use smelt_types::SqlFunction;

/// A `MAX_BY(value, ordering)` projection with NO companion `MAX(ordering)`
/// projection in the same SELECT list must not be admitted into the derived
/// `FoldSpec` — the plan layer must refuse exactly where the runtime
/// classifier refuses (`KeyedUnknownCombiner`).
#[test]
fn max_by_without_companion_is_not_admitted() {
    let sql = "SELECT user_id, MAX_BY(status, updated_at) AS status \
               FROM smelt.sources.events GROUP BY user_id";
    assert!(
        derive_fold_spec(sql).is_none(),
        "a companion-less MAX_BY must not be admitted into a FoldSpec — the runtime \
         classifier refuses this exact SQL with KeyedUnknownCombiner"
    );
}

/// The mirror-image positive case: a companion `MAX(updated_at)` projection
/// is present, so the `MAX_BY` column is admitted.
#[test]
fn max_by_with_companion_is_admitted() {
    let sql = "SELECT user_id, MAX_BY(status, updated_at) AS status, \
               MAX(updated_at) AS updated_at \
               FROM smelt.sources.events GROUP BY user_id";
    let spec = derive_fold_spec(sql).expect("companioned MAX_BY must be admitted");
    assert!(spec
        .add_columns
        .iter()
        .any(|(alias, f)| alias == "status" && *f == SqlFunction::ArgMax));
}

/// `MIN_BY` without a companion `MIN(ordering)` is refused the same way.
#[test]
fn min_by_without_companion_is_not_admitted() {
    let sql = "SELECT user_id, MIN_BY(status, updated_at) AS status \
               FROM smelt.sources.events GROUP BY user_id";
    assert!(derive_fold_spec(sql).is_none());
}

/// Degenerate self-companion: `MAX_BY(x, x)` — the value expression IS the
/// ordering expression, so the projected value is trivially the running
/// max of the ordering value already. No separate companion column is
/// required, and this must be admitted identically to the runtime
/// classifier.
#[test]
fn max_by_self_companion_is_admitted() {
    let sql = "SELECT user_id, MAX_BY(event_ts, event_ts) AS first_seen \
               FROM smelt.sources.events GROUP BY user_id";
    let spec = derive_fold_spec(sql).expect("self-companion MAX_BY must be admitted");
    assert!(spec
        .add_columns
        .iter()
        .any(|(alias, f)| alias == "first_seen" && *f == SqlFunction::ArgMax));
}

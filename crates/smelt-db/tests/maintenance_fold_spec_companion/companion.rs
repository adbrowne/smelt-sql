use smelt_db::queries::maintenance::derive_fold_spec;
use smelt_types::SqlFunction;

/// A `MAX_BY(value, ordering)` projection with NO companion `MAX(ordering)`
/// projection in the same SELECT list is admitted into the derived
/// `FoldSpec` — it decomposes to hidden state, the same shape the runtime
/// classifier now admits.
#[test]
fn max_by_without_companion_is_admitted() {
    let sql = "SELECT user_id, MAX_BY(status, updated_at) AS status \
               FROM smelt.sources.events GROUP BY user_id";
    let spec = derive_fold_spec(sql, &[]).expect("companion-less MAX_BY must be admitted");
    assert!(spec
        .add_columns
        .iter()
        .any(|(alias, f)| alias == "status" && *f == SqlFunction::ArgMax));
}

/// `MIN_BY` without a companion `MIN(ordering)` is admitted the same way.
#[test]
fn min_by_without_companion_is_admitted() {
    let sql = "SELECT user_id, MIN_BY(status, updated_at) AS status \
               FROM smelt.sources.events GROUP BY user_id";
    let spec = derive_fold_spec(sql, &[]).expect("companion-less MIN_BY must be admitted");
    assert!(spec
        .add_columns
        .iter()
        .any(|(alias, f)| alias == "status" && *f == SqlFunction::ArgMin));
}

/// A `MAX_BY`/`MIN_BY` call of the wrong arity still refuses (returns
/// `None`) — fail-closed survives the admission widen.
#[test]
fn max_by_wrong_arity_is_not_admitted() {
    let sql = "SELECT user_id, MAX_BY(status) AS status \
               FROM smelt.sources.events GROUP BY user_id";
    assert!(derive_fold_spec(sql, &[]).is_none());
}

/// Degenerate self-companion: `MAX_BY(x, x)` — the value expression IS the
/// ordering expression. Admitted identically to the runtime classifier.
#[test]
fn max_by_self_companion_is_admitted() {
    let sql = "SELECT user_id, MAX_BY(event_ts, event_ts) AS first_seen \
               FROM smelt.sources.events GROUP BY user_id";
    let spec = derive_fold_spec(sql, &[]).expect("self-companion MAX_BY must be admitted");
    assert!(spec
        .add_columns
        .iter()
        .any(|(alias, f)| alias == "first_seen" && *f == SqlFunction::ArgMax));
}

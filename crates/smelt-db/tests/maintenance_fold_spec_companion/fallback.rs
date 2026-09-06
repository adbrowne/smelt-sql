use smelt_core::config::FunctionalDependency;
use smelt_db::queries::maintenance::derive_fold_spec;
use smelt_types::SqlFunction;

/// The plan layer admits the once-write not-null fallback spelling
/// (`COALESCE(MAX(id), 0)` over the model's own `unique_key`) exactly as
/// `rules::cumulative::classify_once_write` does — plan-layer/runtime
/// admission parity for `docs/outcomes/20260904-decided-gap-residue`
/// phase 3's new route.
#[test]
fn fold_spec_admits_the_not_null_fallback_spelling() {
    let sql = "SELECT id, COALESCE(MAX(id), 0) AS first_id \
               FROM smelt.sources.events GROUP BY id";
    let fds = vec![FunctionalDependency {
        key: vec!["id".to_string()],
        determines: "id".to_string(),
    }];
    let spec =
        derive_fold_spec(sql, &fds).expect("the not-null fallback spelling must be admitted");
    assert!(spec
        .add_columns
        .iter()
        .any(|(alias, f)| alias == "first_id" && *f == SqlFunction::Coalesce));
}

/// Human decision (c) (`docs/outcomes/20260904-decided-gap-residue`
/// outcome.md Decision log): the plan layer admits the same once-write
/// fallback spelling with **no** declared functional dependency at all,
/// exactly as `rules::cumulative::classify_once_write`'s route-2 skip
/// does, since `id` is itself the model's `unique_key` column.
#[test]
fn fold_spec_admits_the_key_member_candidate_without_a_declared_fd() {
    let sql = "SELECT id, COALESCE(MAX(id), 0) AS first_id \
               FROM smelt.sources.events GROUP BY id";
    let spec = derive_fold_spec(sql, &[])
        .expect("the key-member candidate must be admitted without a declared FD");
    assert!(spec
        .add_columns
        .iter()
        .any(|(alias, f)| alias == "first_id" && *f == SqlFunction::Coalesce));
}

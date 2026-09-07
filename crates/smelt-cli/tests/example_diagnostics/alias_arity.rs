use crate::support::*;
use crate::support_ext::*;

/// Phase 3 TDD: `examples/values_broken_alias_arity/` emits exactly one
/// `AliasColumnArityMismatch` at `models/broken_arity.sql`.
#[test]
fn values_broken_alias_arity_emits_arity_mismatch() {
    check_workspace_emits_exactly_one_alias_arity_mismatch(
        "examples/values_broken_alias_arity",
        "models/broken_arity.sql",
    );
}

/// Phase 3 TDD: `examples/cte_broken_alias_arity/` emits exactly one
/// `AliasColumnArityMismatch` at `models/broken_arity.sql`.
#[test]
fn cte_broken_alias_arity_emits_arity_mismatch() {
    check_workspace_emits_exactly_one_alias_arity_mismatch(
        "examples/cte_broken_alias_arity",
        "models/broken_arity.sql",
    );
}

/// Phase 2 closure assertion: `examples/per_cohort_union/models/all_cohorts_unioned.sql`
/// is a UNION ALL of three generator-emitted cohort models. Each emitted model's typed
/// schema is propagated to the consumer through the `smelt.<path>` resolver. All five
/// output columns must resolve to a concrete, non-Unknown type.
///
/// Prior to Phase 2 the FROM-clause typing path did not consult the emission registry,
/// so every projected column typed as Unknown.
#[test]
fn per_cohort_union_all_cohorts_unioned_has_concrete_typed_schema() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::Workspace;
    use std::path::Path;

    let example_dir = "examples/per_cohort_union";
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    // Find all_cohorts_unioned.sql
    let consumer_path = path.join("models").join("all_cohorts_unioned.sql");
    let consumer_file = db
        .source_file(&consumer_path)
        .expect("all_cohorts_unioned.sql not found in workspace");

    let schema = smelt_db::typed_model_schema(&db, ws, consumer_file);

    assert_eq!(
        schema.columns.len(),
        5,
        "all_cohorts_unioned.sql should export 5 columns, got {}:\n  {:?}",
        schema.columns.len(),
        schema.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    );

    for col in &schema.columns {
        let dt = col
            .data_type
            .as_ref()
            .map(|tc| tc.data_type.clone())
            .unwrap_or(smelt_db::DataType::unknown_dynamic());
        assert_ne!(
            dt,
            smelt_db::DataType::unknown_dynamic(),
            "all_cohorts_unioned.sql column '{}' should be concrete (from emitted cohort), \
             got Unknown",
            col.name
        );
    }
}

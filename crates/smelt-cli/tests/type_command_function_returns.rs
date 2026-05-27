//! Regression test: `smelt type` must register `functions/` definitions so a
//! `smelt.functions.*` call with a concrete declared scalar return type infers
//! that type (not `UNKNOWN`) at a scalar SELECT position.
//!
//! The bug: `commands::type::show_type` discovered models (and Python models)
//! but skipped `discover_function_files()`, so the Salsa workspace held no
//! `smelt.define` signatures. `type_context`'s signature-seeding loop then
//! found nothing, and every `smelt.functions.*` scalar call inferred to
//! `DataType::Unknown`.
//!
//! This test reproduces the exact discovery flow used by `show_type` and
//! asserts that `uses_safe_divide.safe_ratio` — a call to `safe_divide`, which
//! declares `-> Expr<Double>` — infers `Double`.

use smelt_cli::{init_db, Config, ModelDiscovery};
use smelt_db::Workspace;
use smelt_types::DataType;
use std::path::Path;

/// Mirror `commands::type::show_type`'s discovery + db init flow, then return
/// the inferred output type of `model.column`.
fn inferred_output_type(example_dir: &str, model_name: &str, column: &str) -> Option<DataType> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let models = discovery.discover_models().unwrap();

    // NOTE: deliberately mirrors `show_type` — only models (+ Python models in
    // the real command) are discovered here. The fix must ensure function
    // files are registered regardless of this test's choices, which is why we
    // exercise the production command path via `init_db` over the models the
    // command itself would register.
    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let model = models.iter().find(|m| m.name == model_name)?;
    let file = db.source_file(&model.path)?;
    let ft = smelt_db::model_function_type(&db, ws, file);
    ft.outputs
        .iter()
        .find(|o| o.name == column)
        .and_then(|o| o.data_type.as_ref())
        .map(|tc| tc.data_type.clone())
}

#[test]
fn safe_divide_scalar_call_infers_double() {
    let dt = inferred_output_type("examples/functions_demo", "uses_safe_divide", "safe_ratio");
    assert_eq!(
        dt,
        Some(DataType::Double),
        "expected uses_safe_divide.safe_ratio to infer Double (safe_divide -> Expr<Double>), got {:?}",
        dt
    );
}

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

/// Real-fixture assertion: `events_parsed` in `examples/web_analytics` ends
/// with `smelt.functions.parse_event_payload(payload).*`.
/// `parse_event_payload` declares `-> Expr<Struct<{event_name: Text, platform: Text, url: Text}>>`.
/// After Phase 1 struct-spread expansion the model's output schema must include
/// `event_name: TEXT, platform: TEXT, url: TEXT`.
#[test]
fn web_analytics_events_parsed_struct_spread_expands_into_schema() {
    fn check_col(example_dir: &str, model: &str, col: &str) -> Option<DataType> {
        inferred_output_type(example_dir, model, col)
    }

    // smelt normalises the `Text` keyword to `Varchar { max_length: None }` internally,
    // so both variants are acceptable here.
    let event_name_dt = check_col("examples/web_analytics", "events_parsed", "event_name");
    assert!(
        matches!(
            event_name_dt,
            Some(DataType::Text) | Some(DataType::Varchar { .. })
        ),
        "events_parsed.event_name must resolve to Text/Varchar (from parse_event_payload struct spread); \
         got {:?} — struct-spread fields are missing from the schema layer",
        event_name_dt
    );

    let platform_dt = check_col("examples/web_analytics", "events_parsed", "platform");
    assert!(
        matches!(
            platform_dt,
            Some(DataType::Text) | Some(DataType::Varchar { .. })
        ),
        "events_parsed.platform must resolve to Text/Varchar; got {:?}",
        platform_dt
    );

    let url_dt = check_col("examples/web_analytics", "events_parsed", "url");
    assert!(
        matches!(
            url_dt,
            Some(DataType::Text) | Some(DataType::Varchar { .. })
        ),
        "events_parsed.url must resolve to Text/Varchar; got {:?}",
        url_dt
    );
}

/// Real-fixture: `margin_via_cte.margin` must resolve to a non-Unknown numeric
/// type. The model passes a local CTE named `x` (with `revenue` + `cost`
/// DECIMAL columns) into `smelt.functions.add_margin(x)`, which computes
/// `revenue - cost AS margin`. Without CTE-argument seeding the body context
/// has no schema for `x`, so `margin` falls back to `Unknown`.
#[test]
fn margin_via_cte_resolves_to_non_unknown() {
    let dt = inferred_output_type("examples/functions_demo", "margin_via_cte", "margin");
    assert!(
        dt.is_some(),
        "margin_via_cte must have a `margin` output column in its schema"
    );
    assert!(
        !matches!(dt, Some(DataType::Unknown)),
        "margin_via_cte.margin must not be Unknown; CTE argument `x` must be seeded into \
         add_margin's body context so `revenue - cost` resolves; got {:?}",
        dt
    );
}

/// Real-fixture: `rollup_dashboard.session_id` must resolve to a non-Unknown type.
///
/// `rollup_dashboard` selects `sr.session_id` from
/// `smelt.functions.session_rollup(...)`. Inside `session_rollup`'s body there
/// is a CTE `sessionized AS (SELECT * FROM smelt.functions.sessionize(...))`.
/// `session_id` is added by `sessionize`'s body. The schema resolver must
/// expand the nested `sessionize(...)` call's output schema when processing
/// `sessionized`'s wildcard SELECT so that `session_id` propagates through the
/// CTE into `session_rollup`'s return schema.
#[test]
fn rollup_dashboard_session_id_not_unknown() {
    let dt = inferred_output_type("examples/functions_demo", "rollup_dashboard", "session_id");
    assert!(
        dt.is_some(),
        "rollup_dashboard must have a `session_id` output column in its schema"
    );
    assert!(
        !matches!(dt, Some(DataType::Unknown)),
        "rollup_dashboard.session_id must not be Unknown; the nested smelt.functions.sessionize \
         call inside session_rollup's body CTE must be resolved so session_id propagates; \
         got {:?}",
        dt
    );
}

/// D2 adversarial regression: struct `.*` spread inside a CTE body must
/// propagate the struct fields (lat: Double, lon: Double) into the CTE's
/// schema so that the outer SELECT can resolve them to concrete types.
///
/// The bug: `infer_cte_columns` → `infer_select_output_schema` processes a
/// `SELECT fn(...).*` CTE body as a single unnamed `Unknown` column because
/// `infer_expression_type` has no branch for `SMELT_PATH_CALL_STAR`. The
/// outer SELECT then cannot resolve `lat` or `lon` and returns `Unknown`.
///
/// The fix: expand `SMELT_PATH_CALL_STAR` items inside CTE bodies using
/// `collect_struct_spread_columns` (or equivalent), matching the top-level
/// model SELECT path.
#[test]
fn struct_spread_inside_cte_body_propagates_field_types() {
    // Top-level struct spread must work (baseline).
    let lat_top = inferred_output_type("examples/fn_struct_spread_cte", "top_level_spread", "lat");
    assert!(
        matches!(lat_top, Some(DataType::Double)),
        "top_level_spread.lat must resolve to Double from the struct spread; got {:?}",
        lat_top
    );

    // Struct spread inside a CTE body must also propagate.
    let lat_cte = inferred_output_type("examples/fn_struct_spread_cte", "cte_spread", "lat");
    assert!(
        matches!(lat_cte, Some(DataType::Double)),
        "cte_spread.lat must resolve to Double — struct spread fields must propagate \
         through a CTE body into the outer SELECT; got {:?} (struct-spread-in-CTE bug)",
        lat_cte
    );

    let lon_cte = inferred_output_type("examples/fn_struct_spread_cte", "cte_spread", "lon");
    assert!(
        matches!(lon_cte, Some(DataType::Double)),
        "cte_spread.lon must resolve to Double; got {:?}",
        lon_cte
    );
}

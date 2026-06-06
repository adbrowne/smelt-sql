//! P3 regression tests: `collect_loader_values` dispatches to
//! `loader_resolved_value_with_overlay` when `active_target` is set and a
//! matching `<basename>.<target>.<ext>` overlay file is registered.
//!
//! Written red (failing) before the implementation in `project.rs`.
//!
//! Cases:
//!   1. `overlay_dispatch_uses_base_when_no_target` — no active target → emitted
//!      body uses base `min_revenue: 100`.
//!   2. `overlay_dispatch_uses_overlay_when_target_prod` — `active_target = "prod"`
//!      + overlay `cohorts.prod.yaml` (min_revenue: 999) → emitted body uses 999.
//!   3. `overlay_dispatch_falls_back_to_base_when_overlay_absent` — active_target
//!      = "prod" but no overlay file registered → emitted body still uses base 100.
//!   4. `overlay_dispatch_invalidates_on_target_change` — change active_target from
//!      None → "prod" → None; each step yields the expected value (Salsa reactive).

use std::sync::Arc;
use tempfile::TempDir;

use smelt_db::{emitted_models, Database, Workspace};

const SMELT_YML: &str = "name: overlay_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: dev.duckdb\n    schema: main\n  prod:\n    type: duckdb\n    database: prod.duckdb\n    schema: main\ndefault_materialization: table\n";

/// Generator file that loads a `List<{name, min_revenue}>` and emits one model
/// per row whose body is `SELECT c.min_revenue AS threshold`.
const GEN_SRC: &str = r#"---
generates: models
---
smelt.config.load_yaml('cohorts.yaml', List<{ name: Text, min_revenue: Integer }>)
  |> map(fn c => ModelDef {
       name: c.name,
       body: SELECT c.min_revenue AS threshold
     })
"#;

const BASE_YAML: &str = "- name: west\n  min_revenue: 100\n";
const OVERLAY_YAML: &str = "- name: west\n  min_revenue: 999\n";

fn build_db_with_overlays(
    tmp: &TempDir,
    base_yaml: Option<&str>,
    overlay_yaml: Option<&str>,
) -> (Database, Workspace) {
    use std::fs;
    let root = tmp.path().to_path_buf();

    fs::write(root.join("smelt.yml"), SMELT_YML).expect("write smelt.yml");
    fs::create_dir_all(root.join("models")).expect("create models/");

    let gen_path = root.join("models").join("cohorts.gen.sql");
    fs::write(&gen_path, GEN_SRC).expect("write gen sql");

    let mut db = Database::default();
    let project = db.set_project_input(root.clone(), SMELT_YML.to_string());
    let gen_sf = db.set_source_file(gen_path, GEN_SRC.to_string(), root.clone());
    db.set_workspace(vec![gen_sf], vec![project]);

    if let Some(text) = base_yaml {
        db.set_loader_file(Arc::from("cohorts.yaml"), Arc::from(text), true);
    }
    if let Some(text) = overlay_yaml {
        db.set_loader_file(Arc::from("cohorts.prod.yaml"), Arc::from(text), true);
    }

    let ws = db.workspace();
    (db, ws)
}

/// No active target → emitted body uses the base value (100).
#[test]
fn overlay_dispatch_uses_base_when_no_target() {
    let tmp = TempDir::new().expect("tempdir");
    let (db, ws) = build_db_with_overlays(&tmp, Some(BASE_YAML), Some(OVERLAY_YAML));
    // active_target defaults to None — no override.

    let result = emitted_models(&db, ws);
    assert_eq!(result.survivors.len(), 1, "expected 1 emission");
    let body = &result.survivors[0].body_text;
    assert!(
        body.contains("100"),
        "base-only: body must contain '100' (base min_revenue), got: {body:?}"
    );
    assert!(
        !body.contains("999"),
        "base-only: body must NOT contain '999' (overlay value), got: {body:?}"
    );
}

/// active_target = "prod" + overlay `cohorts.prod.yaml` (min_revenue: 999)
/// → emitted body must use the overlay value (999), not the base (100).
#[test]
fn overlay_dispatch_uses_overlay_when_target_prod() {
    let tmp = TempDir::new().expect("tempdir");
    let (mut db, ws) = build_db_with_overlays(&tmp, Some(BASE_YAML), Some(OVERLAY_YAML));
    db.set_active_target(Some(Arc::from("prod")));

    let result = emitted_models(&db, ws);
    assert_eq!(result.survivors.len(), 1, "expected 1 emission");
    let body = &result.survivors[0].body_text;
    assert!(
        body.contains("999"),
        "prod target: body must contain '999' (overlay min_revenue), got: {body:?}"
    );
    assert!(
        !body.contains("100"),
        "prod target: body must NOT contain '100' (base value), got: {body:?}"
    );
}

/// active_target = "prod" but no overlay file registered → falls back to base (100).
#[test]
fn overlay_dispatch_falls_back_to_base_when_overlay_absent() {
    let tmp = TempDir::new().expect("tempdir");
    let (mut db, ws) = build_db_with_overlays(&tmp, Some(BASE_YAML), None);
    db.set_active_target(Some(Arc::from("prod")));

    let result = emitted_models(&db, ws);
    assert_eq!(result.survivors.len(), 1, "expected 1 emission");
    let body = &result.survivors[0].body_text;
    assert!(
        body.contains("100"),
        "absent overlay: body must contain '100' (base fallback), got: {body:?}"
    );
}

/// Changing active_target triggers Salsa re-evaluation: the body tracks the
/// target change reactively (None → "prod" → None cycles the value).
#[test]
fn overlay_dispatch_invalidates_on_target_change() {
    let tmp = TempDir::new().expect("tempdir");
    let (mut db, ws) = build_db_with_overlays(&tmp, Some(BASE_YAML), Some(OVERLAY_YAML));

    // Step 1: no target → base value 100.
    let r1 = emitted_models(&db, ws);
    let body1 = r1
        .survivors
        .first()
        .map(|e| e.body_text.clone())
        .unwrap_or_default();
    assert!(
        body1.contains("100"),
        "step 1 (no target): expected base value 100 in body, got: {body1:?}"
    );

    // Step 2: set target → overlay value 999.
    db.set_active_target(Some(Arc::from("prod")));
    let r2 = emitted_models(&db, ws);
    let body2 = r2
        .survivors
        .first()
        .map(|e| e.body_text.clone())
        .unwrap_or_default();
    assert!(
        body2.contains("999"),
        "step 2 (target=prod): expected overlay value 999 in body, got: {body2:?}"
    );

    // Step 3: clear target → base value 100 again.
    db.set_active_target(None);
    let r3 = emitted_models(&db, ws);
    let body3 = r3
        .survivors
        .first()
        .map(|e| e.body_text.clone())
        .unwrap_or_default();
    assert!(
        body3.contains("100"),
        "step 3 (no target): expected base value 100 in body after clearing target, got: {body3:?}"
    );
}

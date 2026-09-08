//! TDD tests for externally-produced sources (black-box steps).
//!
//! Spec: `docs/specs/sources.md` §"Externally-produced sources (black-box
//! steps)". Plan: `docs/outcomes/20260906-external-dag-steps/phases/02-plan.md`.

use smelt_core::external_step::{
    discover_external_step_errors, discover_external_steps, parse_external_step_yaml,
    validate_external_steps, ExternalStepError,
};
use smelt_core::resolver::{classify, EntityKind};
use smelt_core::sources::discover_source_infos;
use std::fs;
use tempfile::TempDir;

fn make_project(tmp: &TempDir, paths: &[&str]) -> std::path::PathBuf {
    let root = tmp.path().to_path_buf();
    let paths_yaml: String = paths
        .iter()
        .map(|p| format!("  - {}\n", p))
        .collect::<String>();
    let smelt_yml = format!("name: test\npaths:\n{}", paths_yaml);
    fs::write(root.join("smelt.yml"), smelt_yml).unwrap();
    root
}

// ---------------------------------------------------------------------------
// 1. yml_with_external_step_classifies_as_step
// ---------------------------------------------------------------------------

#[test]
fn yml_with_external_step_classifies_as_step() {
    let content = r#"
external_step:
  produces:
    - smelt.sources.raw.github_events
  command: ["bash", "scripts/loader.sh"]
"#;
    let kind = classify(std::path::Path::new("step.yml"), Some(content), &[]).unwrap();
    assert_eq!(kind, EntityKind::ExternalStep);
}

// ---------------------------------------------------------------------------
// 2. external_step_beats_csv_sibling
// ---------------------------------------------------------------------------

#[test]
fn external_step_beats_csv_sibling() {
    let content = r#"
external_step:
  produces:
    - smelt.sources.raw.github_events
  command: ["bash", "scripts/loader.sh"]
"#;
    let yml_path = std::path::PathBuf::from("step.yml");
    let csv_sibling = std::path::PathBuf::from("step.csv");
    let kind = classify(&yml_path, Some(content), &[csv_sibling]).unwrap();
    assert_eq!(
        kind,
        EntityKind::ExternalStep,
        "external_step: discriminator must be checked before the seed-sidecar tiebreaker"
    );
}

// ---------------------------------------------------------------------------
// 3. source_discovery_skips_step_files
// ---------------------------------------------------------------------------

#[test]
fn source_discovery_skips_step_files() {
    let tmp = TempDir::new().unwrap();
    let root = make_project(&tmp, &["models"]);
    let dir = root.join("models/sources");
    fs::create_dir_all(&dir).unwrap();

    fs::write(
        dir.join("orders.yml"),
        r#"
columns:
  - { name: order_id, type: INTEGER, nullable: false }
"#,
    )
    .unwrap();
    fs::write(
        dir.join("github_loader.yml"),
        r#"
external_step:
  produces:
    - smelt.sources.orders
  command: ["bash", "scripts/loader.sh"]
"#,
    )
    .unwrap();

    let sources = discover_source_infos(&root, &["models".to_string()]);
    assert_eq!(
        sources.len(),
        1,
        "step file must not be discovered as a source: {sources:?}"
    );
    assert_eq!(sources[0].address_segments, vec!["sources", "orders"]);

    let source_errors = smelt_core::sources::discover_source_errors(&root, &["models".to_string()]);
    assert!(
        source_errors.is_empty(),
        "step file must not surface a MalformedSource error: {source_errors:?}"
    );
}

// ---------------------------------------------------------------------------
// 4. parses_description_produces_command_cadence
// ---------------------------------------------------------------------------

#[test]
fn parses_description_produces_command_cadence() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("models");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("github_loader.yml");
    fs::write(
        &path,
        r#"
external_step:
  description: Loads github events.
  produces:
    - smelt.sources.raw.github_events
    - smelt.sources.raw.github_events_arrival
  command: ["bash", "scripts/bq-dogfood-loader.sh", "--date", "{run_date}"]
  cadence: '1 day'
"#,
    )
    .unwrap();

    let info = parse_external_step_yaml(&path).unwrap();
    assert_eq!(info.description.as_deref(), Some("Loads github events."));
    assert_eq!(
        info.produces,
        vec![
            "smelt.sources.raw.github_events".to_string(),
            "smelt.sources.raw.github_events_arrival".to_string(),
        ]
    );
    assert_eq!(
        info.command,
        vec![
            "bash".to_string(),
            "scripts/bq-dogfood-loader.sh".to_string(),
            "--date".to_string(),
            "{run_date}".to_string(),
        ]
    );
    let cadence = info.cadence.expect("cadence should parse");
    assert_eq!(cadence.seconds, 86400);
    assert_eq!(cadence.display, "1 day");
}

// ---------------------------------------------------------------------------
// 5. absent_produces_is_malformed / empty_produces_is_malformed
// ---------------------------------------------------------------------------

#[test]
fn absent_produces_is_malformed() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("step.yml");
    fs::write(
        &path,
        r#"
external_step:
  command: ["bash", "scripts/loader.sh"]
"#,
    )
    .unwrap();

    let err = parse_external_step_yaml(&path).unwrap_err();
    assert!(matches!(err, ExternalStepError::EmptyProduces));
}

#[test]
fn empty_produces_is_malformed() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("step.yml");
    fs::write(
        &path,
        r#"
external_step:
  produces: []
  command: ["bash", "scripts/loader.sh"]
"#,
    )
    .unwrap();

    let err = parse_external_step_yaml(&path).unwrap_err();
    assert!(matches!(err, ExternalStepError::EmptyProduces));
}

// ---------------------------------------------------------------------------
// 6. absent_command_is_malformed / empty_command_is_malformed / non_list_command_is_malformed
// ---------------------------------------------------------------------------

#[test]
fn absent_command_is_malformed() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("step.yml");
    fs::write(
        &path,
        r#"
external_step:
  produces:
    - smelt.sources.raw.orders
"#,
    )
    .unwrap();

    let err = parse_external_step_yaml(&path).unwrap_err();
    assert!(matches!(err, ExternalStepError::EmptyCommand));
}

#[test]
fn empty_command_is_malformed() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("step.yml");
    fs::write(
        &path,
        r#"
external_step:
  produces:
    - smelt.sources.raw.orders
  command: []
"#,
    )
    .unwrap();

    let err = parse_external_step_yaml(&path).unwrap_err();
    assert!(matches!(err, ExternalStepError::EmptyCommand));
}

#[test]
fn non_list_command_is_malformed() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("step.yml");
    fs::write(
        &path,
        r#"
external_step:
  produces:
    - smelt.sources.raw.orders
  command: "bash scripts/loader.sh"
"#,
    )
    .unwrap();

    let err = parse_external_step_yaml(&path).unwrap_err();
    assert!(matches!(err, ExternalStepError::CommandNotList));
}

// ---------------------------------------------------------------------------
// 7. columns_alongside_external_step_is_malformed
// ---------------------------------------------------------------------------

#[test]
fn columns_alongside_external_step_is_malformed() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("step.yml");
    fs::write(
        &path,
        r#"
external_step:
  produces:
    - smelt.sources.raw.orders
  command: ["bash", "scripts/loader.sh"]
columns:
  - { name: order_id, type: INTEGER }
"#,
    )
    .unwrap();

    let err = parse_external_step_yaml(&path).unwrap_err();
    assert!(matches!(
        err,
        ExternalStepError::ColumnsAlongsideExternalStep
    ));
}

// ---------------------------------------------------------------------------
// 8. unparseable_cadence_is_malformed
// ---------------------------------------------------------------------------

#[test]
fn unparseable_cadence_is_malformed() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("step.yml");
    fs::write(
        &path,
        r#"
external_step:
  produces:
    - smelt.sources.raw.orders
  command: ["bash", "scripts/loader.sh"]
  cadence: 'not an interval'
"#,
    )
    .unwrap();

    let err = parse_external_step_yaml(&path).unwrap_err();
    assert!(matches!(err, ExternalStepError::UnparseableCadence(_)));
}

// ---------------------------------------------------------------------------
// 9. unknown_key_is_malformed
// ---------------------------------------------------------------------------

#[test]
fn unknown_key_is_malformed() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("step.yml");
    fs::write(
        &path,
        r#"
external_step:
  produces:
    - smelt.sources.raw.orders
  command: ["bash", "scripts/loader.sh"]
  bogus_key: true
"#,
    )
    .unwrap();

    let err = parse_external_step_yaml(&path).unwrap_err();
    assert!(matches!(err, ExternalStepError::YamlParse { .. }));
}

// ---------------------------------------------------------------------------
// 10. produces_address_naming_no_declared_source_is_malformed
// ---------------------------------------------------------------------------

#[test]
fn produces_address_naming_no_declared_source_is_malformed() {
    let tmp = TempDir::new().unwrap();
    let root = make_project(&tmp, &["models"]);
    let dir = root.join("models/sources");
    fs::create_dir_all(&dir).unwrap();

    fs::write(
        dir.join("step.yml"),
        r#"
external_step:
  produces:
    - smelt.sources.no_such_source
  command: ["bash", "scripts/loader.sh"]
"#,
    )
    .unwrap();

    let steps = discover_external_steps(&root, &["models".to_string()]);
    let sources = discover_source_infos(&root, &["models".to_string()]);
    let errors = validate_external_steps(&steps, &sources);

    assert_eq!(errors.len(), 1, "expected exactly one error: {errors:?}");
    assert!(matches!(
        errors[0].1,
        ExternalStepError::ProducesUnknownSource(_)
    ));
}

// ---------------------------------------------------------------------------
// 11. two_steps_producing_one_source_conflict
// ---------------------------------------------------------------------------

#[test]
fn two_steps_producing_one_source_conflict() {
    let tmp = TempDir::new().unwrap();
    let root = make_project(&tmp, &["models"]);
    let dir = root.join("models/sources");
    fs::create_dir_all(&dir).unwrap();

    fs::write(
        dir.join("orders.yml"),
        r#"
columns:
  - { name: order_id, type: INTEGER, nullable: false }
"#,
    )
    .unwrap();
    fs::write(
        dir.join("step_a.yml"),
        r#"
external_step:
  produces:
    - smelt.sources.orders
  command: ["bash", "scripts/loader_a.sh"]
"#,
    )
    .unwrap();
    fs::write(
        dir.join("step_b.yml"),
        r#"
external_step:
  produces:
    - smelt.sources.orders
  command: ["bash", "scripts/loader_b.sh"]
"#,
    )
    .unwrap();

    let steps = discover_external_steps(&root, &["models".to_string()]);
    let sources = discover_source_infos(&root, &["models".to_string()]);
    let errors = validate_external_steps(&steps, &sources);

    assert_eq!(errors.len(), 1, "expected exactly one conflict: {errors:?}");
    assert!(matches!(
        errors[0].1,
        ExternalStepError::ProducerConflict { .. }
    ));
    // Anchored at the later-sorted (second) step's path — "step_b.yml" sorts
    // after "step_a.yml".
    assert_eq!(errors[0].0, dir.join("step_b.yml"));

    // No parse errors: both step files are individually well-formed.
    let parse_errors = discover_external_step_errors(&root, &["models".to_string()]);
    assert!(
        parse_errors.is_empty(),
        "expected zero parse errors: {parse_errors:?}"
    );
}

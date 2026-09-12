//! TDD tests for the `command:` placeholder grammar
//! (`docs/specs/sources.md` §"Externally-produced sources (black-box
//! steps)"). Plan: `docs/outcomes/20260906-external-dag-steps/phases/04-plan.md`.

use smelt_core::external_step::{
    parse_external_step_yaml, resolve_command, CommandResolveError, ExternalStepError,
    StepRunContext,
};
use std::fs;
use tempfile::TempDir;

fn write_step(tmp: &TempDir, command: &str) -> std::path::PathBuf {
    let path = tmp.path().join("step.yml");
    fs::write(
        &path,
        format!(
            "external_step:\n  produces:\n    - smelt.sources.raw.events\n  command: {command}\n"
        ),
    )
    .unwrap();
    path
}

#[test]
fn run_date_placeholder_substituted() {
    let tmp = TempDir::new().unwrap();
    let path = write_step(
        &tmp,
        r#"["bash", "loader.sh", "--date", "{run_date}", "--tag", "static"]"#,
    );
    let step = parse_external_step_yaml(&path).expect("valid step");
    let ctx = StepRunContext {
        run_date: Some("2026-09-08".to_string()),
        run_end: None,
    };
    let argv = resolve_command(&step, &ctx).expect("resolves");
    assert_eq!(
        argv,
        vec![
            "bash",
            "loader.sh",
            "--date",
            "2026-09-08",
            "--tag",
            "static"
        ]
    );
}

#[test]
fn escaped_braces_are_literal() {
    let tmp = TempDir::new().unwrap();
    let path = write_step(&tmp, r#"["bash", "loader.sh", "{{run_date}}"]"#);
    let step = parse_external_step_yaml(&path).expect("valid step");
    let ctx = StepRunContext::default();
    let argv = resolve_command(&step, &ctx).expect("resolves");
    assert_eq!(argv, vec!["bash", "loader.sh", "{run_date}"]);
}

#[test]
fn unknown_placeholder_is_malformed() {
    let tmp = TempDir::new().unwrap();
    let path = write_step(&tmp, r#"["bash", "loader.sh", "{nope}"]"#);
    let err = parse_external_step_yaml(&path).expect_err("unknown placeholder must be refused");
    assert!(
        matches!(err, ExternalStepError::UnknownPlaceholder(_)),
        "expected UnknownPlaceholder, got: {err:?}"
    );
}

#[test]
fn placeholder_without_window_is_not_invocable() {
    let tmp = TempDir::new().unwrap();
    let path = write_step(&tmp, r#"["bash", "loader.sh", "--date", "{run_date}"]"#);
    let step = parse_external_step_yaml(&path).expect("valid step");
    let ctx = StepRunContext::default();
    let err = resolve_command(&step, &ctx).expect_err("no window means not invocable");
    assert_eq!(
        err,
        CommandResolveError::NoValueForPlaceholder("run_date".to_string())
    );
}

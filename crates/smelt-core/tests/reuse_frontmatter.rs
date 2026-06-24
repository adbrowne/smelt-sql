//! Tests for P2: `reuse` frontmatter hatches and `forward_only`/`state`
//! fields on `ModelMetadata` (D-46, D-47).

use smelt_core::metadata::{extract_file_metadata, FileMetadata, ModelMetadata};

fn single_metadata(sql: &str) -> ModelMetadata {
    match extract_file_metadata(sql) {
        Ok(FileMetadata::Single { metadata, .. }) => *metadata,
        other => panic!("expected Single metadata, got {:?}", other),
    }
}

// ── reuse: block ─────────────────────────────────────────────────────────────

#[test]
fn reuse_accept_current_parses() {
    let sql = "---\nreuse:\n  accept_current: true\n---\nSELECT 1";
    let meta = single_metadata(sql);
    let reuse = meta.reuse.expect("reuse must be Some");
    assert!(reuse.accept_current);
    assert!(!reuse.assert_deterministic);
}

#[test]
fn reuse_assert_deterministic_parses() {
    let sql = "---\nreuse:\n  assert_deterministic: true\n---\nSELECT 1";
    let meta = single_metadata(sql);
    let reuse = meta.reuse.expect("reuse must be Some");
    assert!(reuse.assert_deterministic);
    assert!(!reuse.accept_current);
}

#[test]
fn reuse_defaults_when_absent() {
    let sql = "---\nname: my_model\n---\nSELECT 1";
    let meta = single_metadata(sql);
    assert!(meta.reuse.is_none(), "reuse must be None when absent");
}

#[test]
fn reuse_unknown_sub_key_is_rejected() {
    // deny_unknown_fields on ReuseConfig must reject unknown sub-keys.
    let sql = "---\nreuse:\n  bogus_key: true\n---\nSELECT 1";
    let result = extract_file_metadata(sql);
    assert!(
        result.is_err(),
        "unknown sub-key under reuse: must be rejected; got: {:?}",
        result
    );
}

// ── forward_only: ─────────────────────────────────────────────────────────────

#[test]
fn forward_only_parses() {
    let sql = "---\nforward_only: true\n---\nSELECT 1";
    let meta = single_metadata(sql);
    assert!(meta.forward_only, "forward_only must be true");
}

#[test]
fn forward_only_defaults_false() {
    let sql = "---\nname: my_model\n---\nSELECT 1";
    let meta = single_metadata(sql);
    assert!(!meta.forward_only, "forward_only must default to false");
}

// ── state: block in model frontmatter ─────────────────────────────────────────

#[test]
fn model_state_mode_stateless_parses() {
    let sql = "---\nstate:\n  mode: stateless\n---\nSELECT 1";
    let meta = single_metadata(sql);
    use smelt_core::config::StateMode;
    let state = meta.state.expect("state must be Some");
    assert_eq!(state.mode, StateMode::Stateless);
}

#[test]
fn model_state_mode_environments_parses() {
    let sql = "---\nstate:\n  mode: environments\n---\nSELECT 1";
    let meta = single_metadata(sql);
    use smelt_core::config::StateMode;
    let state = meta.state.expect("state must be Some");
    assert_eq!(state.mode, StateMode::Environments);
}

#[test]
fn model_state_defaults_none_when_absent() {
    let sql = "---\nname: my_model\n---\nSELECT 1";
    let meta = single_metadata(sql);
    assert!(meta.state.is_none(), "state must be None when absent");
}

#[test]
fn model_state_unknown_mode_is_rejected() {
    let sql = "---\nstate:\n  mode: quantum\n---\nSELECT 1";
    let result = extract_file_metadata(sql);
    assert!(
        result.is_err(),
        "unknown state.mode value must be rejected; got: {:?}",
        result
    );
}

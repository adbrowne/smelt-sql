//! `examples/trino_broken_foreign_keys` fixture test
//! (`docs/outcomes/20260913-trino-target-spine/phases/03-plan.md`): a `trino`
//! target carrying keys that belong to another backend's shape must fail to
//! load, naming every offending key.

use std::path::PathBuf;

fn workspace_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
        .join("trino_broken_foreign_keys")
}

#[test]
fn trino_broken_foreign_keys_workspace_fails_to_load_naming_every_key() {
    let dir = workspace_dir();
    let err = smelt_core::Config::load(&dir)
        .expect_err("a trino target carrying foreign keys must fail to load");
    let message = err.to_string();
    for key in ["warehouse", "project", "dataset"] {
        assert!(message.contains(key), "error must name `{key}`: {message}");
    }
    assert!(
        message.contains("trino"),
        "error must name the backend: {message}"
    );
}

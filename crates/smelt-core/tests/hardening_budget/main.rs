use std::path::PathBuf;

mod batched_vocab;
mod cfg_test_declared;
mod dev_only_crates;
mod gate_regression;
mod println_gate;

pub(crate) fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

pub(crate) fn script_path() -> PathBuf {
    repo_root().join(".claude/scripts/hardening-budget.sh")
}

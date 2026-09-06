//! Structural: the run pipeline's `FileStore` must be posture-gated
//! (`docs/specs/state.md` §"`state.mode` and what each posture provides").
//! The `execute/` pipeline is the one production call site that owns a run's
//! `.smelt/` writes; it must construct its store via
//! `FileStore::with_state_mode`, never the permissive `FileStore::new`, so a
//! future edit can't silently reintroduce writes under `state.mode:
//! stateless`.
//!
//! Spec: `docs/specs/state.md` §"The optionality rule".

#[test]
fn execute_rs_constructs_the_run_store_via_with_state_mode() {
    const EXECUTE_SRC: &str = concat!(
        include_str!("../src/execute/backend.rs"),
        include_str!("../src/execute/bootstrap.rs"),
        include_str!("../src/execute/key_addressed.rs"),
        include_str!("../src/execute/mod.rs"),
        include_str!("../src/execute/outcome.rs"),
        include_str!("../src/execute/plan.rs"),
        include_str!("../src/execute/project.rs"),
        include_str!("../src/execute/retry.rs"),
        include_str!("../src/execute/sink.rs"),
        include_str!("../src/execute/sources.rs"),
        include_str!("../src/execute/targets.rs"),
        include_str!("../src/execute/window.rs"),
    );
    assert!(
        EXECUTE_SRC.contains("FileStore::with_state_mode("),
        "the execute pipeline must build the run pipeline's FileStore via \
         with_state_mode(..., config.state.mode) so state.mode gates every \
         .smelt/ write; found no call to FileStore::with_state_mode"
    );
    assert!(
        !EXECUTE_SRC.contains("FileStore::new("),
        "the execute pipeline must not construct a permissive FileStore::new — that \
         bypasses state.mode gating and would write .smelt/ even under \
         `stateless`. Use FileStore::with_state_mode instead."
    );
}

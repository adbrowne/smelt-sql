//! Structural assertion over `src/execute/project/mod.rs`'s own source: the
//! `.smelt/` state lock (`docs/specs/run_state.md` §"Locking") is acquired
//! from exactly one call site, and that call site is not guarded by any
//! condition on the run's target, dialect, or backend. The lock and the
//! layout-version check it performs are backend-independent — realised
//! identically on every target, including one (Trino) that claims no
//! engine-resident correctness structure (`docs/specs/state.md` §"The
//! degradation contract") — so a target-conditional introduced around this
//! call site would silently make the lock stop locking on whichever target
//! the condition excludes, exactly the failure criterion 9 rules out.

const EXECUTE_PROJECT_SRC: &str = include_str!("../src/execute/project/mod.rs");

/// The exact call site, anchored on its known `.context(...)` message so an
/// unrelated `.lock()` elsewhere in the file (there are many — `graph.lock()`,
/// `db.lock()`, tokio mutexes) can't be miscounted as the state lock.
const LOCK_CALL_ANCHOR: &str =
    "let _state_lock = file_store\n        .lock()\n        .context(\"failed to acquire the .smelt/ state lock\")?;";

#[test]
fn execute_project_acquires_the_state_lock_unconditionally() {
    let call_count = EXECUTE_PROJECT_SRC.matches(LOCK_CALL_ANCHOR).count();
    assert_eq!(
        call_count, 1,
        "execute/project/mod.rs contains the `.smelt/` state-lock acquisition anchor \
         {call_count} times (expected exactly 1). If the call site's wording changed \
         intentionally, update `LOCK_CALL_ANCHOR`; if a second call site was added, the run \
         pipeline no longer acquires the lock from a single, easily-audited place."
    );
}

/// The 5 lines immediately preceding the lock call must contain no
/// `if`/`match` that could branch the acquisition on the run's target,
/// dialect, or backend — only the doc comment explaining why the lock is
/// unconditional is permitted there. A conditional introduced right above
/// the call site is exactly the failure criterion 9 ("never a lock that
/// never locks") rules out.
#[test]
fn no_conditional_guards_the_lock_call() {
    let idx = EXECUTE_PROJECT_SRC
        .find(LOCK_CALL_ANCHOR)
        .unwrap_or_else(|| {
            panic!("LOCK_CALL_ANCHOR not found — run the other test in this file for the message")
        });
    let preceding_lines: Vec<&str> = EXECUTE_PROJECT_SRC[..idx].lines().collect();
    let window: String = preceding_lines
        .iter()
        .rev()
        .take(5)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    for banned in ["if ", "match "] {
        assert!(
            !window.contains(banned),
            "found `{banned}` in the 5 lines immediately preceding the state-lock \
             acquisition — the lock must be reached unconditionally, never behind a \
             target/dialect/backend branch:\n{window}"
        );
    }
}

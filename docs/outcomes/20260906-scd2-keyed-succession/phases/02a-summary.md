# Phase 2a summary — De-flake the `smelt-core` baseline scratch-hygiene test

**Shipped:**
- `materialize_in(resolved: &ResolvedBaseline, scratch_parent: &Path) -> Result<BaselineCheckout, BaselineError>`
  (`crates/smelt-core/src/baseline/git.rs`) — the extracted seam, re-exported from
  `crates/smelt-core/src/baseline/mod.rs` alongside `materialize`.
- `materialize` is now a one-line delegation to `materialize_in(resolved, &std::env::temp_dir())`.
- `checkout_scratch_is_deleted_when_materialization_fails` rewritten to assert against a private
  `TempDir` scratch parent instead of snapshotting `std::env::temp_dir()`.
- Two new tests: `checkout_scratch_is_deleted_on_drop_uses_the_given_parent` (private-parent
  placement + cleanup) and `materialize_defaults_its_scratch_parent_to_the_system_temp_dir`
  (plain `materialize` still lands under the system temp dir).

**Decisions:**
- Followed the plan exactly — no reshape needed for phase 2a's own scope.
- `rg -n 'temp_dir\(\)' crates/` found no other test snapshotting all entries under
  `std::env::temp_dir()`; the other hits are process-id-scoped subdirs (`smelt-logical`'s
  `walk_coverage.rs`) or non-snapshotting references — none needed the same fix.

**For the next planner:**
- **New finding, not fixed here:** `bash .claude/scripts/verify-phase.sh` is still red. The one
  remaining failure is `smelt-core`'s `hardening_budget::gate_detects_regression`: `smelt-logical`
  production `unwrap` is 2 (baseline 1) and `expect("` is 4 (baseline 1). Isolated the four new
  sites to `crates/smelt-logical/src/analysis/succession.rs` (1 unwrap + 3 expect), which phase 2
  landed (`84ca6c86`) — confirmed by stashing this phase's own diff and re-running the gate on the
  unchanged tree, still red. Not caused by the `analysis/mod.rs`/`walk.rs` split refactors
  (`a411f3f6`, `5107c66b`) despite those looking like the obvious suspect. Out of this phase's
  scope (different gate, different file). Inserted phase **2b** to resolve it: classify each site
  as infallible or convert to `Result`, per the fail-loud gate's "never lower without a reviewer
  sign-off note" rule — no silent `--update` of the baseline.
- Confirmed `cargo test -p smelt-runtime --lib` (compiles cleanly, `property_diff.rs`'s
  `materialize` call site unaffected) and `cargo test -p smelt-cli --test transformer_metamorphic`
  (2 passed) — the other `materialize` callers are unaffected by the signature change.

**Gates:**
- `for i in $(seq 1 20); do cargo test -p smelt-core --test baseline --quiet ...; done` — 20/20 green.
- `cargo test -p smelt-core --quiet` — green (23 baseline tests + all other smelt-core tests).
- `cargo test -p smelt-runtime --lib property_diff --quiet` — compiles, 0 tests filtered (lib module).
- `cargo test -p smelt-cli --test transformer_metamorphic --quiet` — 2 passed.
- `bash .claude/scripts/verify-phase.sh` — **FAILED** on `hardening_budget::gate_detects_regression`
  only (fmt-check, clippy, example_diagnostics all green); this phase's own target
  (`checkout_scratch_is_deleted_when_materialization_fails`) is fixed and green. See "For the next
  planner" — phase 2b addresses the remaining failure.

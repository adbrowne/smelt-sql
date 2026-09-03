# Phase 27c summary — keyless (whole-row `EXCEPT ALL`) staged-candidate realisation

**Shipped:**
- `smelt_logical::maintenance::emit::emit_staged_candidate_conditional_keyless` (`crates/smelt-logical/src/maintenance/emit.rs`): the 7-statement transactional keyless realisation — stage the candidate, materialise a two-way `EXCEPT ALL` sentinel (stored side optionally region-restricted) before either write, then a region-grained guarded `DELETE`+`INSERT`, then two `DROP`s. No key parameter — panics only on an empty `candidate_select`.
- `smelt_logical::maintenance::choice::resolve_keyless_staged_suppression` (`crates/smelt-logical/src/maintenance/choice.rs`): admits `Suppressed` only for `RowIdentity::WholeRow` with every column in the model's full payload set proven `Comparable`; fail-closed on a `Key` identity, an empty column set, or any unproven/incomparable column.
- `MembershipRecomputeWrite::StagedKeyless` + `execute_staged_keyless_recompute` (`crates/smelt-runtime/src/maintenance_driver.rs`): the runtime executor, mirroring `execute_staged_membership_recompute` minus the observed-delta leg (a keyless write has no key to record against).
- `resolve_live_membership_recompute_cell` now handles a `RowIdentity::WholeRow` `Technique::DeleteInsert` cell (previously skipped outright): it runs the keyless proof directly (never the keyed `resolve_write_suppression`, which refuses solely on `WholeRow`) over the union of the model's own derived column groups.
- **New production dispatch** in `crates/smelt-runtime/src/execute.rs`'s non-keyed batch loop: a live keyless membership cell (mutation-gated, never on the creation run) now executes once per run via `execute_staged_keyless_recompute` and reports `strategy: "delete_insert_suppressed"`, replacing the batch loop entirely for that run. This closes the actual reachability gap — the resolver alone is dead code from the keyed-only call site, since a `plan_is_keyed` model always has `RowIdentity::Key`.
- Spec deltas: `docs/specs/model_transforms.md` (whole-row realisation shape/contract, table coverage cell, narrowed Known Divergences bullet) and `docs/specs/incremental_models.md` (removed the "whole-row (keyless) staged-candidate realisation does not exist" clause).

**Decisions:**
- Suppression is **region-grained**, not row-grained, for the keyless case — no portable SQL can delete a multiset difference with per-row multiplicity without an address. Matches the plan's design-call.
- The keyless proof compares the model's **full payload column set** (union of all derived column groups), not just the triggering cell's own mutation-sensitive group — a whole-row `EXCEPT ALL` diff is sensitive to every selected column, unlike the keyed diff join which only needs its own compared group.
- Wired the new dispatch as a **once-per-run, full-model-recompute** action (mirrors the keyed `StagedRecompute`'s own shape) rather than a per-batch region-scoped write, keeping it structurally parallel to the existing keyed-loop caller and avoiding a second, materially different dispatch shape in the same phase.

**For the next planner:**
- Three existing tests (`bakeoff_seam.rs`, `maintenance_pins.rs`, `technique_lowering.rs`) hard-coded the pre-27c "no live dispatch for `grain: partition` `WholeRow` membership cells" assumption in both doc comments and `strategy == "deleteinsert"` assertions. Updated in place to `"delete_insert_suppressed"` — this is the intended, correct behavior change this phase produces, not a regression to guard against.
- 27d (write-pin selecting keyed MERGE vs. staged-candidate) and 27g (runtime dispatch for that selection) are unaffected — this phase's mechanism is a distinct, keyless-only fallback that never competes with the keyed selection.
- Not done: extending `statement_parity`'s byte-identical structural leg and the `architecture.md` Known Divergences narrowing for the backbuild emitter family — that's phase 30's explicit scope.
- The new dispatch executes once per run using the model's full unwindowed recompile as `candidate_select`, with no `region_predicate` — i.e. it diffs the WHOLE stored table against the whole candidate, not per-batch regions, even though the emitter itself supports a `region_predicate`. A future phase could region-scope this per batch for lower per-run cost on wide-window runs; left unbuilt here since the plan's own tests validate the full-model shape and scoping it per-batch is a genuinely separate design decision (interacts with the batch loop's own chunking).

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings both feature sets, full `cargo test` workspace, `example_diagnostics`)
- `cargo test -p smelt-logical --lib maintenance` — 185 passed
- `cargo test -p smelt-runtime --test statement_parity --test repair_lowering` — 19 + 27 passed
- `cargo test -p smelt-cli --test maintenance_conformance` — 74 passed
- `cargo test --workspace` (full sweep, per phase 25's note) — all 342 test binaries green, 0 failures

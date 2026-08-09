# Phase 4 summary — `diff_patch` write pattern

**Shipped**
- `crates/smelt-logical/src/maintenance/diff_patch.rs` (new): `DeleteLeg`, `DiffPatchRefusal`,
  `AdmittedDiffPatch`, `admit_diff_patch` — identity via `RowIdentity::Key`, comparability reused
  verbatim from `choice::resolve_write_suppression`, slice completeness as a caller-supplied
  `Result<(), String>`.
- `crates/smelt-logical/src/maintenance/emit.rs:947` `emit_diff_patch` — stage → update-leg
  delete → optional delete-leg delete → insert → drop, one transactional `StatementGroup`, both
  deletes carry the slice `Region` predicate.
- `crates/smelt-logical/src/maintenance/mod.rs` — `diff_patch` registry entry (`Identity`,
  `Always`), `WriteSelection::DiffPatch`, `selects()` arm, registry test list.
- `crates/smelt-logical/src/maintenance/choice.rs` — `ChosenTechnique::DiffPatch { recompute,
  delete_leg }`, `admits_write_selection` admits `None | DeleteInsert | PerGroupRecompute`,
  `resolve_cell_choice`'s `write_pin` arm, unit test `pin_diff_patch_resolves_to_a_diff_write`
  (line ~1500).
- `crates/smelt-logical/tests/diff_patch.rs` (new, 10 tests) — registry, admission (identity,
  comparability, both `DeleteLeg` shapes), emitter (statement order/count, `IS DISTINCT FROM`,
  delete-leg omission, empty-key panic).
- `crates/smelt-runtime/tests/technique_lowering.rs:2820` — third exhaustive-match arm.
- `crates/smelt-runtime/src/maintenance_driver.rs:717` — `WriteSelection::DiffPatch` arm in the
  (no-production-call-site) dimension-merge resolver, refusing the same way the `Technique(other)`
  arm does.
- `crates/smelt-cli/tests/fixtures/explain_show_sql_daily_events_golden.txt` — `, diff_patch`
  appended to both admissible-pattern lines.
- `docs-site/docs/examples/web-analytics/deduplication.md` — regenerated via
  `python3 examples/web_analytics/generate_tutorial.py` (same admissible-list fact).

**Decisions** (made ahead of implementation by this phase's own prompt, not re-derived here;
mirrored into `outcome.md`'s Decision log):
- `diff_patch` enters the closed enum namespace via `WriteSelection::DiffPatch` +
  `ChosenTechnique::DiffPatch { recompute: Technique, delete_leg: DeleteLeg }`, not a new
  `Technique` variant — a pin can never silently degrade to a blanket delete+insert.
- An incomparable/unproven compared column refuses the whole pattern
  (`ComparabilityRequired`), never falls back to an unconditional update leg — that would just be
  delete+insert with extra steps.
- `resolve_cell_choice`'s `DiffPatch` arm always produces `DeleteLeg::Omitted` today (a stated,
  documented placeholder) — no slice-completeness proof is threaded through this admission layer
  yet.
- `emit_diff_patch` is one function with a conditional statement (not two sibling emitters like
  `emit_staged_candidate_conditional[_recompute]`) because the delete-leg degradation is a
  per-call runtime fact, not a distinct fixed caller population.

**For the next planner**
- **Primary flag:** `resolve_cell_choice`'s `WriteSelection::DiffPatch` arm hard-codes
  `DeleteLeg::Omitted` with a placeholder `why`. Phase 5 must either thread a real completeness
  proof through (repair family's key-temporal-locality premise for a `PerGroupRecompute` source,
  or the region write-window clamp for a `DeleteInsert` source) or explicitly accept that
  `diff_patch` never produces a delete leg until it does.
- Nothing routes to `diff_patch` yet (no runtime lowering, no executed-vs-emitted parity leg) —
  by design, deferred to phase 5 per the plan.
- No other follow-ups noticed beyond the plan's own scope boundary.

**Gates**
- `cargo test -p smelt-logical --test diff_patch` — PASS (10/10)
- `cargo test -p smelt-logical --lib maintenance::choice` / `diff_patch` — PASS
- `cargo test -p smelt-runtime --test statement_parity` — PASS (19/19)
- `cargo test -p smelt-runtime --test technique_lowering` — PASS (27/27)
- `cargo test -p smelt-logical --test walk_coverage` — PASS (4/4)
- `cargo test -p smelt-cli --test explain` — PASS (4/4)
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy, full workspace test,
  example_diagnostics all green)

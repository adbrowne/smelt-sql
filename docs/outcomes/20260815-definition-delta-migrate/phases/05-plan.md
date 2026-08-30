# Phase 5 plan — definition-edit step kind in the generative conformance suite

## Objective

Give the generative maintenance-conformance suite a step kind that stages a definition change
mid-history and routes it through the *shipped* `smelt migrate` derive→apply path (phases 2–3),
then asserts the new-definition oracle **immediately after the migration** rather than after a
later full-refresh catch-up. Advances success criteria 4 (the harness gains the step kind), 8/9
(the `definition_deltas.md` divergence bullet is removed; standing gates green).

Today `ConformanceStep::RewriteModel` exists but (a) is never generated — `arb_schedule_for`
emits only window steps — and (b) deliberately asserts the *pre-migrate* contract ("the next run
compiles whatever SQL is on disk"), with a doc comment claiming the spec's classification is
unbuilt. That claim is now false.

## Spec delta

`docs/specs/definition_deltas.md` §"Known Divergences / Open Questions" — delete the bullet
**"The conformance harness has no definition-edit step kind yet"**. §"The oracle"'s closing
sentence ("the same harness, one more step kind") becomes true as written; no wording change.
Land the deletion only once the new gate tests are green.

## Tests

Red-green, in order:

1. `apply_migration_executes_plan_statements_in_order` (`smelt-runtime`, unit next to
   `definition_delta.rs`) — the extracted library apply executes exactly `plan.statements`, in
   order, against the passed backend and authors nothing of its own.
2. `migrate_step_applies_plan_and_recovers_new_definition_equivalence`
   (`maintenance_conformance/gate.rs`, pinned) — `PassThrough` + `ModelEdit::AddPayloadColumn`:
   two windows run under the original body, then `MigrateModel`, then assert equivalence against
   the **rewritten** body's oracle with **no intervening run**; a following ordinary windowed run
   must keep it equal.
3. `migrate_step_refuses_and_full_refreshes_when_no_technique_admits` (pinned) — `AdditiveAgg` +
   `ModelEdit::AddGroupingColumn` (a skeleton change): the derived plan admits no in-place
   statements, so the step takes the full-refresh route the CLI's own refusal message names, and
   equivalence still holds. Asserts the refusal leg actually ran (not the applied leg).
4. `definition_edit_pool_upholds_new_definition_equivalence` (generative) — deterministic-seeded
   sample over the partition append-only pool restricted to recipes with a non-empty
   `evolution`; schedule from the new `arb_schedule_with_definition_edit`; drive and assert after
   every step. Fails if zero cases took the **applied** leg (no vacuous pass), mirroring
   `admission_rate_stays_above_floor`'s anti-vacuity discipline.

## Tasks

1. Extract the statement-execution half of `smelt-cli/src/commands/migrate.rs::apply_plan` into
   `smelt_runtime::definition_delta::apply_migration(backend: &dyn Backend, plan:
   &MigrationPlan) -> Result<()>`; the CLI keeps approval gating, schema recording and rendering
   and calls the new function for the loop. No new statement authoring (maintenance-plan purity).
2. Add `ConformanceStep::MigrateModel { edit: ModelEdit }` to
   `smelt-maintenance-testkit/src/schedule_gen.rs` with a doc comment stating its contract
   (rewrite on disk → derive → apply-or-refuse → the new-definition oracle holds at once), and
   correct `RewriteModel`'s now-stale "that classification is unbuilt" paragraph to point at
   `MigrateModel` as the migrated counterpart.
3. Add a shared driver helper in the testkit (next to the existing `render`/`s_tracker` helpers)
   that performs the step: write `render_model_file_with_edit`, assemble `derive_plan`'s inputs
   the way `gate.rs::derive_plan_with_real_deployed_schema` already assembles a maintenance
   plan's (real `FileStore` deployed schema, discovered `ModelFile`s, `SourcesConfig`), then
   either `apply_migration` + `schema_evolution::save_deployed_schema`, or — when
   `plan.statements` is empty — a full-refresh run. Returns which leg ran so the gates can assert
   on it.
4. Add `arb_schedule_with_definition_edit(recipe)` to `schedule_gen.rs`: an `arb_schedule_for`
   schedule with a `MigrateModel { edit }` inserted after the first window's run, drawn from
   `recipe.evolution`. **Do not touch `arb_schedule_for` itself** — every existing pool
   (including the Spark/BigQuery twins that consume the same generator) must keep generating
   byte-identical schedules.
5. Handle `MigrateModel` in `maintenance_conformance/gate.rs`'s `drive_and_assert`: run the task-3
   helper, set `current_edit`, then `assert_equivalence_with_edit` straight away.
6. Handle it identically in `smelt-maintenance-testkit/src/families/gate.rs` (the exhaustive match
   breaks otherwise) by calling the same task-3 helper — a real arm, never a silent skip, even
   though those pools do not generate the step today.
7. Write tests 1–4; then delete the spec bullet (spec delta above).

## Verification

- `bash .claude/scripts/verify-phase.sh` — full gate (fmt, clippy both feature sets, `cargo test`,
  example_diagnostics).
- `cargo test -p smelt-cli --test maintenance_conformance 2>&1 | tail -40` — the extended suite.
- `cargo test -p smelt-runtime --test statement_parity` — no new statement authoring outside
  `smelt-logical`.
- `cargo test -p smelt-cli --test migrate_plan` — the CLI's apply contract is unchanged by the
  extraction.
- Hardening baseline must stay unchanged (no new production `unwrap`/`expect`/`println!`); if the
  extraction shifts a count, re-run `--update` and note it in the summary.

## Commit message

`test(conformance): stage definition edits through smelt migrate in the generative gate`

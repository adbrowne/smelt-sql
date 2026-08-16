# Phase 3 plan — `--apply` execution

## Objective

Make `smelt migrate <model> --apply` on a matching plan hash actually execute the approved plan's
statements against the backend, re-record the deployed definition, and resume group-by-group when
re-invoked after an interruption. Completes success criterion 2 (the gate half landed in phase 2)
and removes the "`--apply` does not execute statements yet" divergence.

## Spec delta (made first, by the implementer)

`docs/specs/definition_deltas.md`:

- §Surface "`smelt migrate`" — extend "Approve and apply" and "Resume" with what apply actually
  does: on a hash match it executes each column group's **first presented candidate** (the plan is
  deterministic and the hash covers every candidate, so approving the plan approves that
  selection), one transactional statement group per column group in plan order; on success it
  re-records the deployed definition so the next plan step is eclipsed; a re-invocation after
  every group applied reports "already applied" and exits `0`. Fail-closed admission: a plan
  containing a skeleton-change group, a group with no admissible candidate, or a destructive
  (`ColumnDrop`) candidate executes **nothing** and exits `3` with the named reason — the honest
  route is `smelt build --full-refresh` / a rebuild.
- §Known Divergences — delete the "`--apply` does not execute statements yet" bullet and the
  execution clause of the "synthesis layer's execution half is unwired" bullet; add two narrower,
  honest bullets: (a) migration resume is recorded **per column group**, not per region — the
  per-region frontier reset §"Frontier semantics" describes is per-cell frontier addressing, which
  this outcome lists under "Out of scope"; (b) destructive legs are refused rather than executed
  because their verification probes (§"The migration plan", "Destructive legs are verified") are
  not emitted yet.

`docs/specs/cli.md` §"Exit codes" — widen exit `3`'s wording from "unapproved" to "a non-trivial
migration remains pending" so it also covers an approved-but-refused-to-execute plan.

`docs/specs/run_state.md` — `migration-approvals.json` entries gain `applied_groups` /
`applied_at` (resume record).

## Tests

- `smelt-logical` (`backbuild/emit.rs` or `plan.rs` module tests)
  - `candidate_statement_group_is_transactional_and_preserves_emitter_text` — the new
    `statement_group_for_candidate` wraps the candidate's `statements` verbatim, `transactional:
    true`, no re-authoring.
- `smelt-runtime` (`migrate.rs` module tests, fake `Backend` recording executed SQL)
  - `apply_executes_first_candidate_per_group_in_plan_order` — executed text == emitted text,
    group order preserved.
  - `apply_refuses_skeleton_change_group_without_executing` — zero statements executed, named error.
  - `apply_refuses_group_with_no_admissible_candidate` — likewise.
  - `apply_refuses_destructive_candidate` — `ColumnDrop`/`CostClass::Destructive` executes nothing.
  - `apply_skips_groups_already_recorded_applied` — resume: pre-seeded applied labels are not
    re-executed; the remainder is.
  - `apply_reports_already_applied_when_every_group_is_recorded` — no statements, `Ok`.
- `smelt-cli` (`tests/migrate.rs`, `#![cfg(feature = "duckdb")]`, real DuckDB)
  - `apply_backfills_added_column_and_rerecords_definition` — build v1, edit to v2 (added derived
    column), plan (exit 3), `--apply` (exit 0); the DuckDB table has the column with backfilled
    values; a following `smelt migrate` reports eclipsed and exits `0`.
  - `apply_is_idempotent_on_reinvocation` — a second `--apply` executes nothing, exits `0`.
  - `apply_with_stale_hash_leaves_the_table_untouched` — the phase-2 staleness refusal still
    executes nothing (row snapshot unchanged), exit `3`.
  - `apply_refuses_a_skeleton_change_plan` — a `GROUP BY`-altering edit refuses, exit `3`, table
    unchanged.

## Tasks

1. Land the spec delta above (spec-first).
2. `smelt-logical`: add `backbuild::statement_group_for_candidate(&TechniqueCandidate) ->
   StatementGroup` (`transactional: true`), the sole conversion from candidate statements to the
   executed group — keeps statement single-ownership inside `smelt-logical`.
3. `smelt-state`: `MigrationApproval` gains `#[serde(default)] applied_groups: Vec<String>` and
   `applied_at: Option<DateTime<Utc>>`; `MigrationApprovalStore::record_applied_group(model,
   label, at)`. `record` (new hash) clears the applied record — a different plan resumes nothing.
4. `smelt-runtime::migrate`: add `MigrationApplyRefusal` (skeleton change / no admissible
   candidate / destructive candidate — each naming the group) and
   `apply_migration_plan(backend, &plan, already_applied: &BTreeSet<String>, on_group_applied)`:
   admission check over **all** groups first (execute nothing on refusal), then per group in plan
   order execute `statement_group_for_candidate(&group.candidates[0])` via
   `Backend::execute_statement_group`, invoking the callback after each group commits.
5. `smelt-cli::commands::migrate`: qualify `facts.table` with the target schema so emitted
   statements address the deployed table; on the `--apply` + matching-hash branch open a backend
   via `CliBackendFactory` (restore the `--database` override flag), call `apply_migration_plan`,
   persist each applied group to the approval store as it commits, then `save_deployed_schema`
   with the new SQL + current columns (version bumped) and exit `0`. Refusals map to
   `CliError::PendingMigration` (exit `3`); `--json` gains `applied_groups` and `applied`.
6. Extend the `migrate --json` doc block and the human render with the applied/refused lines.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test migrate --test exit_codes --features duckdb`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity`
- `cargo test -p smelt-state --quiet`
- `cargo test -p smelt-core --test hardening_budget` (update the `println!` baseline only for
  genuinely user-facing CLI output, with the reason in the commit message)

## Commit message

`feat(migrate): execute the approved plan under --apply, re-record the definition, resume per group`

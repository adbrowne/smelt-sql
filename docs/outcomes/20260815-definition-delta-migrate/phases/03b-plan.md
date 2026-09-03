# Phase 3b — `smelt run` refuses to fold over a pending definition delta; surface it in `explain`

## Objective

Close the detection half of `definition_deltas.md` §Detection: a model with a pending,
non-eclipsed definition delta must refuse to fold data deltas rather than maintain a table whose
definition no longer matches its contents, and the delta must be visible ahead of a run via
`smelt explain`. Advances success criterion 1 (the mechanism is reachable outside `smelt migrate`)
and criterion 2 (the approval store is what gates execution, not just the migrate verb).

## Spec delta (made first, by the implement step)

1. `docs/specs/definition_deltas.md` §Detection — sharpen the existing refusal sentence into a
   normative rule and name its conditions: the refusal applies to a **maintained (incremental)
   model whose stored table already exists and carries a recorded `model_sql`**; a
   `--full-refresh` run is not a fold and proceeds under the new definition; "approved" means an
   approval on record whose `plan_hash` equals the freshly re-derived plan hash; an approval
   marked `in_progress` folds under §"Mid-migration data folds" rather than refusing. Name the
   exit code (`3`, per `cli.md` §"Exit codes").
2. `docs/specs/definition_deltas.md` §Diagnostics — add a row for `DefinitionDeltaPending`
   (Error): fires when a run would fold a data delta over an unapproved non-eclipsed definition
   delta; the fix is `smelt migrate <model>` (review) then `--apply`, or `--full-refresh`.
3. `docs/specs/diagnostics.md` — catalogue `DefinitionDeltaPending` in the same table as the
   `Maintenance*` codes, owned by `definition_deltas.md`.
4. `docs/specs/cli.md` §"Exit codes" — extend the code-`3` row and add a **`smelt run`
   specifics** line: exits `3` (not `1`) when a selected model refuses on a pending definition
   delta; the run is otherwise a correctly-derived state awaiting review.
5. `docs/specs/definition_deltas.md` §"`smelt migrate`" (or §Detection) — one sentence that
   `smelt explain <model>` reports a pending definition delta and its whole-plan verdict without
   deriving or executing anything.

## Tests (red-green)

- `smelt-runtime` unit, `definition_delta.rs`: `no_recorded_model_sql_is_not_a_delta` — a model
  with no `model_sql` on record yields `Status::Unknown`, never a refusal.
- `smelt-runtime` unit: `identical_definition_is_no_delta` — byte-identical SQL → `Status::None`.
- `smelt-runtime` unit: `eclipsed_delta_does_not_gate` — a trivia-only/eclipsed change → `Eclipsed`.
- `smelt-runtime` unit: `non_eclipsed_unapproved_delta_is_pending` — added column, no approval →
  `Pending { verdict, hash }`.
- `smelt-runtime` unit: `matching_approval_is_approved` — same hash on record → `Approved`.
- `smelt-cli` integration `definition_delta_gate.rs`: `run_refuses_incremental_fold_over_pending_delta`
  — build an incremental model, edit its select list, re-run → non-zero exit, message names the
  model, `DefinitionDeltaPending`, and `smelt migrate`.
- same file: `run_refusal_exits_3` — the refusal's process exit code is `3`.
- same file: `full_refresh_run_proceeds_over_pending_delta` — `--full-refresh` succeeds.
- same file: `run_proceeds_after_migrate_plan_records_approval` — `smelt migrate <m>` then
  `smelt run` folds normally.
- same file: `non_incremental_model_never_refuses` — a table/view model with a changed definition
  runs.
- `smelt-cli` integration `explain_definition_delta.rs`: `explain_reports_pending_definition_delta`
  (text) and `explain_json_carries_definition_delta` (`--json` field present, verdict named);
  plus `explain_omits_definition_delta_when_none`.
- `smelt-cli` `migrate_apply.rs`: `apply_resumes_rerun_safe_in_progress_plan` — the untested leg
  the phase-3 summary flagged (`in_progress: true` **and** `all_rerun_safe()`) re-executes rather
  than refusing.

## Tasks

1. Write the spec deltas above (spec-first) before touching code.
2. Add `crates/smelt-runtime/src/definition_delta.rs`: `DefinitionDeltaStatus`
   (`Unknown | None | Eclipsed | Pending { verdict, plan_hash } | InProgress | Approved`) and
   `detect_definition_delta(file_store, project_dir, model_file, all_models, db, sources)` —
   moved verbatim from `commands/migrate.rs` steps 5–7 (parse both sides → `definition_diff` →
   `BackbuildInputs` assembly → `derive_migration_plan` + `plan_hash`), then cross-checked against
   the approval store. Keep the existing fail-closed source-facts rule and its comment.
3. Refactor `commands/migrate.rs` to call the new module for derivation (no behaviour change;
   `migrate_plan.rs` + `migrate_apply.rs` must stay green unchanged apart from the new test).
4. Add `DefinitionDeltaPending` to `DiagnosticCode` in `smelt-db` (catalogue gate will demand the
   `diagnostics.md` row from task 1).
5. Gate in `crates/smelt-runtime/src/execute.rs`, immediately before the schema-evolution gate
   (~L1595): for `plan.incremental.is_some()`, the table exists, and neither `request.full_refresh`
   nor `request.dry_run` — call `detect_definition_delta` and return a named error on `Pending`.
   `Unknown`/`None`/`Eclipsed`/`InProgress`/`Approved` proceed. Detection failures degrade to a
   `tracing::warn!` and proceed (never break a run on a diff the module cannot factor).
6. Classify the refusal as exit `3` in the CLI: extend `commands/run.rs`'s error path the way
   `migrate::exit_code_for` does, re-using one shared error type rather than a string match.
7. Surface in `commands/explain.rs` `explain_maintenance_plan`: add a definition-delta line to the
   always-built diagnostics section and a corresponding field in
   `smelt_cli::explain::build_maintenance_plan_json`.
8. Add the `apply_resumes_rerun_safe_in_progress_plan` test (phase-3 summary follow-up).
9. `docs-site/docs/reference/cli.md` — one line under `smelt run` naming the refusal and the fix
   (the narrative rewrite stays phase 8).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test definition_delta_gate --test explain_definition_delta --test migrate_plan --test migrate_apply`
- `cargo test -p smelt-runtime --lib definition_delta`
- `cargo test -p smelt-db --test integration diagnostics_catalogue`
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity`
- `cargo test -p smelt-cli --test maintenance_conformance`

## Commit message

`feat(migrate): refuse data-delta folds over a pending definition delta and surface it in explain`

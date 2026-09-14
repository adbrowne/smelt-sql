# Phase 4 plan — Wire the absence: downgrade, never refuse

## Objective

Make the code perform the degradation the spec now describes for Trino. Today
`smelt_backend::maintenance_dialect(SqlDialect::Trino)` returns `Err`, and three seams treat
that as a fatal refusal — `smelt explain` aborts the whole command, `smelt-runtime`'s
`profile.rs` drops the model into `out.failures`, `--dry-run` silently `continue`s — while
`smelt-db`'s `backend_dialect_for("trino")` returns `None` and reaches the right availability
answer only by an `unwrap_or_default()` accident. Advances criteria 3 and 4: every dependent
cell resolves to its recompute equivalent with `MaintenanceStateDowngraded`, derived **once**
by the pure resolver, with the ideal plan still materialised as `state_downgrade.original`.

## Design decision (record in the decision log)

**Phase 4 does not add a `MaintenanceDialect::Trino` variant.** That enum has ~40 exhaustive
arms across `emit/{fingerprint,succession,probes,partition_bucket,hash,merge,bootstrap}.rs`,
and every one of them is a *statement spelling* — the half `20260913-trino-incremental` (T4)
owns by the maintenance-plan invariant's own seam. Phase 4 instead makes the **plan, the
downgrade and the report** independent of the maintenance-statement dialect: the absence of a
`MaintenanceDialect` stops being "you cannot see the plan" and becomes "this one statement
cannot be rendered yet", named. Rendering polish for `smelt explain` (text + `--json` layout,
docs-site) stays phase 10's; phase 4 only removes the abort so phase 10 has something to
render.

## Spec delta

`docs/specs/multi_backend.md` §"Parity contract" — the Trino paragraph currently ends with the
forward-pointer "`maintenance_dialect` currently returns `Err` … and is being corrected …
(phase 4)". Replace with the settled statement: availability resolution and the maintenance-plan
report are dialect-independent and downgrade on Trino; only maintenance *statement* rendering is
still unavailable there, refused by name, pending `20260913-trino-incremental`. **Must keep the
literal tokens `maintenance_dialect` and `SqlDialect::Trino`** — `crates/smelt-cli/tests/
trino_emission_spec_freshness.rs::parity_contract_states_trino_scope` asserts both are present
(this went red once already in phase 2).

## Tests (red first)

1. `crates/smelt-db/src/queries/maintenance/write_pin.rs` (unit) —
   `backend_dialect_for_recognises_trino`: `Some(SqlDialect::Trino)`; an unknown name is still
   `None`, so the fail-loud `unwrap_or_default()` is not being widened.
2. `crates/smelt-logical/tests/maintenance_availability/resolution.rs` —
   `trino_downgrades_every_dependent_cell_to_its_recompute_equivalent`: for each cell shape with
   a `required_state_structure`, resolving under `StateAvailability::resolve(Allowed,
   &realisable_state_structures(SqlDialect::Trino))` yields `recompute_equivalent(cell)`, a
   recorded `StateDowngrade`, and `original` still naming the ideal technique.
3. `crates/smelt-cli/tests/` (new, offline) `trino_explain_downgrade.rs` —
   `explain_on_a_trino_target_reports_instead_of_aborting`: a fixture workspace with a Trino
   target and a ledger-dependent maintained model; `smelt explain <model>` exits 0 and its
   `--json` carries a `state_downgrade` with `original` ≠ the executed technique. No live tier.
4. Same file — `explain_show_sql_on_trino_refuses_by_name`: `--show-sql` (the statement leg)
   names Trino and the missing maintenance-statement support rather than printing nothing or
   falling back to another dialect's spelling.
5. `crates/smelt-runtime/tests/availability_seam/main.rs` —
   `a_trino_target_model_is_profiled_not_failed`: the model does not appear in
   `out.failures`, and its cells carry the downgrade.
6. `crates/smelt-cli/tests/` dry-run leg — `dry_run_on_trino_names_the_gap`: the silent
   `continue` at `execute/project/dry_run.rs:244` becomes a named, user-visible line (fail-loud
   discipline: a skipped statement must not look like an absent one).

## Tasks

1. Add `"trino" => SqlDialect::Trino` to `backend_dialect_for`; confirm the two
   `maintenance_plan_diagnostics` loops now reach Trino by mapping, not by fallback.
2. Widen the `dialect` parameter of `smelt-runtime::diagnostics::build_model_diagnostics` and
   `probe_plan::probe_plan_for_model` to `Option<MaintenanceDialect>` (or obtain it lazily at
   the two statement-rendering leaves) — `None` = statement rendering unavailable, never a
   substituted dialect.
3. `crates/smelt-cli/src/commands/explain.rs:~556` — stop `?`-ing on `maintenance_dialect`;
   resolve availability and build the report/JSON unconditionally; carry the `Err` into the
   `--show-sql` and probe-plan legs as a by-name refusal.
4. `crates/smelt-runtime/src/profile.rs:~201` — same: resolve availability and emit the profile;
   omit the dialect-dependent probe entries with a named note instead of `out.failures`.
5. `crates/smelt-runtime/src/execute/project/dry_run.rs:~244` — replace the silent `continue`
   with a named omission line.
6. Update the `smelt_backend::maintenance_dialect` doc comment and the stale
   `execute/targets.rs` header ("`20260913-trino-incremental` narrows the error away") to say
   what the error now means: statements only, never the plan.
7. Apply the spec delta; re-run the freshness gates.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test maintenance_availability --test state_realisability_docs`
- `cargo test -p smelt-runtime --test availability_seam --test statement_parity`
- `cargo test -p smelt-cli --test trino_explain_downgrade --test trino_emission_spec_freshness`
- `cargo test -p smelt-core --test trino_docs_freshness`
- `bash .claude/scripts/large-file-check.sh` — no baseline bump without a sign-off note.

## Commit message

`feat(state): Trino downgrades rather than refuses — availability resolution reaches the plan without a maintenance dialect`

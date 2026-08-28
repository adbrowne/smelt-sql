# Phase 3 plan — Approval store + `--apply` + `--json`

## Objective

Turn `smelt migrate` from plan-only into the plan-and-approve verb §Surface specifies: the plan
step persists its hash, `--apply` executes only a plan whose freshly re-derived hash matches the
persisted one (refusing and reprinting otherwise), and `--json` plus a distinct exit code makes a
pending unapproved migration a checkable CI state. Advances success criteria 2 and 9, and closes
the "No approval store exists" divergence.

## Spec delta (lands first)

- `docs/specs/cli.md` §"Exit codes" — add row `3`: "The command ran correctly and found a state
  requiring human approval. Today: `smelt migrate`/`smelt migrate --json` with a derived,
  non-eclipsed, unapproved migration, and `smelt migrate --apply` refusing on a stale or absent
  approval." Add the one-line "`smelt migrate` specifics" paragraph alongside the existing
  `diff`/`test`/`check` ones, and state that `3` is distinct from `1` (a problem found in
  data/models) and `2` (a bad invocation).
- `docs/specs/definition_deltas.md` §Surface "`smelt migrate`" — make the CI-mode bullet name the
  concrete codes (0 / 3 / 2) instead of "a distinct non-zero exit", and state that the plan step
  records the hash to a per-target approval store. Amend §Surface "Resume" to say resume is by
  re-invocation of the same approved plan (identical hash ⇒ identical script).
- `docs/specs/definition_deltas.md` §Known Divergences — delete "No approval store exists";
  narrow "`smelt migrate --apply` and `--json` do not exist yet" to the `smelt rebuild` rename
  (phase 4) and the still-live column-additions-only maintenance-driver mechanism; narrow
  "Execution and run-time refusal are unwired" to the run-time refusal only, retargeted at phase
  3b. Add one honest bullet: resume is approval-marker-based, not frontier-region-scoped per
  §"Frontier semantics" — a partially-applied non-`rerun_safe` script refuses rather than
  resuming.

## Tests (red first)

**`crates/smelt-state/src/migration_approvals.rs` (unit)**
1. `approval_store_round_trips` — save then load returns the same per-model hash + marker.
2. `missing_approval_file_loads_empty` — no file ⇒ empty store, not an error.
3. `recording_a_new_hash_replaces_the_previous` — approval is of the *most recent* plan only.

**`crates/smelt-logical/tests/migration_plan.rs` (extend)**
4. `plan_carries_assembled_statements` — `derive_migration_plan` populates `statements` from
   `assemble` over the first admitted option per atom.
5. `skeleton_change_plan_has_no_statements` — any group with zero admitted options ⇒ empty
   `statements` (no partial application ever offered).
6. `plan_hash_covers_statements` — two plans differing only in assembled statements hash
   differently; re-deriving the same plan hashes identically.

**`crates/smelt-cli/tests/migrate_apply.rs` (new, real-DuckDB subprocess)**
7. `plan_then_apply_executes_and_clears_the_delta` — plan, `--apply` exits 0, a following
   `smelt migrate` reports eclipsed/no delta and the stored table carries the new column values.
8. `apply_without_a_prior_plan_refuses` — exit 3, message names the plan step.
9. `apply_after_the_model_changed_refuses_and_reprints` — edit the SQL between plan and apply ⇒
   exit 3, the new plan is printed, nothing executed.
10. `apply_on_a_skeleton_change_refuses` — exit 1, message points at a full refresh.
11. `json_eclipsed_exits_zero` — formatting-only edit ⇒ exit 0, JSON `verdict: "eclipsed"`.
12. `json_pending_migration_exits_three` — non-trivial delta unapproved ⇒ exit 3, JSON
    `approved: false`, per-group verdict/technique/statement_count present.
13. `json_after_approval_exits_zero` — plan step then `--json` ⇒ exit 0, `approved: true`.
14. `interrupted_non_rerun_safe_apply_refuses_on_reinvocation` — in-progress marker present and a
    chosen option is not `rerun_safe` ⇒ refuse with the full-refresh route named.

## Tasks

1. Land the spec delta above (cli.md, definition_deltas.md) — spec-first.
2. `crates/smelt-state/src/migration_approvals.rs`: `MigrationApproval { plan_hash, in_progress }`,
   `MigrationApprovalStore` (BTreeMap keyed by canonical model path), + `FileStore::{load,save}_migration_approvals`
   following the `landed_deltas.rs` / `source_postures.rs` shape exactly.
3. `smelt-logical` `backbuild/plan.rs`: add `MigrationPlan::statements`, filled by calling the
   existing `assemble(&BackbuildOptions, Selection::Targeted { atom_choices: all-zero })` — no new
   statement authoring anywhere (statement single-ownership); empty when any atom admits nothing.
   Fold `statements` into `plan_hash`.
4. `MigrateArgs`: add `--apply` and `--json` flags (both `bool`); wire through `commands::migrate`.
5. Plan path: after rendering, record `{plan_hash, in_progress: false}` for the model; return the
   phase's exit code (0 when the plan is eclipsed/empty, else 3) via a typed error the existing
   `smelt_cli::exit_code_for` maps — mirror `commands::list::exit_code_for`'s pattern with a
   `commands::migrate::exit_code_for`.
6. `--apply` path: re-derive the plan, compare hashes, refuse (exit 3) on absent/mismatched
   approval and print the freshly derived plan; refuse (exit 1) when `statements` is empty
   (skeleton change); otherwise open the backend via `helpers::create_backend` and
   `execute_sql` each statement in order, setting `in_progress: true` before the first and
   clearing the approval after the last.
7. On successful apply, write the new `model_sql` (and refreshed columns) into the deployed
   schema via `FileStore::save_schema` so the delta is cleared — this is what makes test 7's
   second invocation report eclipsed.
8. `--json` renderer: `{model, table, verdict, plan_hash, approved, groups: [{columns, verdict,
   technique, statement_count, refusals}], statements}` — canonical `smelt.<path>` model naming
   per `cli.md` §"Canonical-display rule". Same exit codes as the human path.
9. `docs-site/docs/reference/cli.md`: add the two flags and the exit-3 row (the narrative guide
   page is phase 8's job, not this phase's).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-state --lib migration_approvals`
- `cargo test -p smelt-logical --test migration_plan`
- `cargo test -p smelt-cli --test migrate_plan` (must still pass — hashes are derived, not literal)
- `cargo test -p smelt-cli --test migrate_apply`
- `cargo test -p smelt-runtime --test statement_parity`
- `cargo test -p smelt-logical --test walk_coverage`

## Commit message

`feat(migrate): persist plan approvals and add smelt migrate --apply/--json`

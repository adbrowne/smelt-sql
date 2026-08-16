# Phase 2 plan — Approval gate: plan hash, approval store, `--json`, exit codes, `--apply` refusal

## Objective

Make the printed plan *approvable*: hash the plan data (plus the facts that justified it),
persist that hash per model/target, and give `smelt migrate` the CI surface the spec promises —
`--json`, a distinct non-zero exit when a non-trivial migration is pending, and an `--apply`
that refuses a stale or unrecorded plan and prints the fresh one instead. Advances success
criterion 2 (the gate half; execution lands in phase 3) and criterion 8's exit-code honesty.
`--apply` executes nothing in this phase — on a hash match it reports the approved plan and
exits `0`.

## Spec delta (implement step makes these edits first)

1. `docs/specs/cli.md` §"Exit codes" — add row `3` to the normative table: *"A non-trivial
   migration is pending and unapproved (`smelt migrate`, §`definition_deltas.md`). The command
   ran correctly; the deploy changes what a table means and no approved plan covers it."* Add a
   **`smelt migrate` specifics** paragraph next to the existing per-verb paragraphs: exit `0` when
   there is no definition delta or the delta is eclipsed-only; exit `3` when a non-eclipsed plan
   is derived and unapproved, or when `--apply` finds the recorded hash absent/stale; `2` for the
   usual usage/config errors.
2. `docs/specs/definition_deltas.md` §Surface "`smelt migrate`" — name exit `3` explicitly in the
   **CI mode** bullet (replacing "a distinct non-zero exit"), and state where approval lives:
   `.smelt/targets/<target>/migration-approvals.json`, one recorded plan hash per model, written
   by the plan step.
3. `docs/specs/definition_deltas.md` §Known Divergences — narrow the "No approval store exists"
   bullet to what remains true after this phase: the store, hash, and refusal gate exist;
   `--apply` does not yet execute statements (phase 3 removes the bullet).
4. `docs/specs/run_state.md` — document the `migration-approvals.json` store alongside the other
   per-target state files.

## Tests (red first)

**`crates/smelt-logical/src/backbuild/hash.rs` (unit)**
- `plan_hash_is_stable_across_repeated_derivation` — same inputs → byte-identical hash twice.
- `plan_hash_changes_when_statement_text_changes` — same verdict/technique shape, different
  emitted SQL → different hash (approval binds to exact statements).
- `plan_hash_changes_when_source_facts_change` — identical plan shape, upstream `unique_key`
  differs → different hash (the justification is hashed, per §Design).
- `plan_hash_changes_when_verdict_changes` — skeleton vs. backfill-in-place → different hash.
- `plan_hash_is_order_independent_over_sources` — `BTreeMap` iteration is canonical; two inputs
  built in different insertion order hash equal.

**`crates/smelt-state/src/migration_approvals.rs` (unit)**
- `approval_store_round_trips` — save then load returns the recorded hash per model.
- `missing_approval_file_reads_empty` — absent file → empty store, not an error (fail-closed:
  empty means "nothing approved").
- `recording_a_hash_replaces_the_previous_one_for_that_model` — one live approval per model.

**`crates/smelt-cli/tests/migrate.rs` (integration, real DuckDB fixture)**
- `plan_step_records_hash_and_exits_three` — changed definition → plan printed, hash recorded,
  exit `3`.
- `eclipsed_delta_exits_zero` — unchanged definition → "eclipsed", exit `0`.
- `json_flag_emits_plan_hash_and_verdicts` — `--json` output parses, carries `model`,
  `eclipsed`, `plan_hash`, per-group `verdict`/`technique`, and `approved`.
- `apply_without_recorded_plan_refuses` — `--apply` before any plan step → prints the plan,
  records it, exit `3`, executes nothing.
- `apply_refuses_when_definition_changed_since_plan` — plan, then edit the model, then `--apply`
  → refusal naming the hash mismatch, fresh plan printed and recorded, exit `3`.
- `apply_with_matching_hash_is_accepted` — plan then `--apply` unchanged → exit `0`, reports the
  approved plan (no execution this phase).

## Tasks

1. Land the four spec edits above.
2. `crates/smelt-logical/src/backbuild/hash.rs`: length-prefixed canonical encoder (mirror
   `smelt-fingerprint/src/hash.rs`'s framing; that crate's `Encoder` is `pub(crate)`, so this is a
   local copy, not a widened export) + `pub fn plan_hash(inputs: &BackbuildInputs, plan:
   &MigrationPlan) -> String` returning `sha256:<hex12>`. Add `sha2` to `smelt-logical`'s deps.
   Pure — no I/O, no clock.
3. Add `statements: Vec<String>` to `TechniqueCandidate` (populated from `BackbuildOption`;
   keep `statement_count` derived from it so the two cannot drift) so the plan is
   self-sufficient for hashing and for phase 3's executor.
4. `crates/smelt-state/src/migration_approvals.rs`: `MigrationApprovalStore { approvals:
   BTreeMap<String, MigrationApproval> }`, `MigrationApproval { plan_hash, recorded_at }`;
   `FileStore::load_migration_approvals` / `save_migration_approvals` writing
   `.smelt/targets/<target>/migration-approvals.json` (follow `save_landed_deltas`'s idiom).
5. `crates/smelt-cli/src/main.rs`: add `--apply` and `--json` to `MigrateArgs`.
6. `crates/smelt-cli/src/errors.rs`: `CliError::PendingMigration(String)`; map it to `3` in
   `exit_code_for`, with the doc comment citing `cli.md` §"Exit codes".
7. `crates/smelt-cli/src/commands/migrate.rs`: compute the hash, load the approval store, and
   branch — plan mode records the hash and returns `PendingMigration` for a non-eclipsed plan;
   `--apply` compares the recorded hash to the freshly derived one and returns
   `PendingMigration` (printing the fresh plan and re-recording) on absence or mismatch, or
   prints "approved — nothing to execute yet (execution lands with the apply path)" and exits `0`
   on a match. Add the JSON renderer (`serde_json::Value` built in the CLI; the plan types stay
   serde-free).
8. Refresh `.claude/hardening-baseline.txt` only if the new user-facing `println!`s in
   `smelt-cli` move the counts (`smelt-cli` stdout is legitimate).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-state --quiet 2>&1 | tail -20`
- `cargo test -p smelt-cli --test migrate --test exit_codes --quiet 2>&1 | tail -20`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity --quiet 2>&1 | tail -20`

## Commit message

`feat(migrate): plan-hash approval store, --json, and the pending-migration exit code`

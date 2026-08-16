# Phase 2 summary — Approval gate

## Shipped

- `smelt_logical::backbuild::plan_hash` (`crates/smelt-logical/src/backbuild/hash.rs`): pure,
  length-prefixed canonical hash over `BackbuildInputs` + `MigrationPlan`, `sha256:<hex12>`.
  Exhaustive tag matches over every enum it encodes (fail-loud: a new `Technique`/`Verdict`/…
  variant fails to compile here).
- `TechniqueCandidate` (`crates/smelt-logical/src/backbuild/plan.rs`) now carries its own
  `statements: Vec<String>`; `statement_count()` is derived from it, never stored separately.
- `MigrationApprovalStore`/`MigrationApproval` (`crates/smelt-state/src/migration_approvals.rs`)
  + `FileStore::{load,save}_migration_approvals` writing
  `.smelt/targets/<target>/migration-approvals.json`, gated by `StateMode` like `landed_deltas`.
- `smelt migrate <model> --apply` and `--json` flags (`crates/smelt-cli/src/main.rs`); the command
  now computes the plan hash, loads/records the approval store, and branches: eclipsed → exit 0
  always; plan mode on a non-eclipsed plan → records the hash, exits 3 (`CliError::PendingMigration`
  → exit code 3 in `errors.rs`); `--apply` with a matching recorded hash → prints "approved —
  nothing to execute yet", exits 0; `--apply` with an absent/stale hash → re-records the fresh
  plan, prints it, exits 3.
- `derive_migration_plan_for_model` (`crates/smelt-runtime/src/migrate.rs`) now returns
  `(BackbuildInputs, MigrationPlan)` so the CLI can hash exactly the facts the plan was derived
  from, without reconstructing them.
- Spec edits: `cli.md` exit code `3` + `smelt migrate` specifics; `definition_deltas.md` §Surface
  names exit `3` and the approval-store path, §Known Divergences narrowed (the "No approval store
  exists" bullet is gone — the remaining gap is `--apply` not executing statements, phase 3);
  `run_state.md` documents `migration-approvals.json` in the directory layout, atomic-write list,
  and fixed-layout constraint.
- 9 integration tests in `crates/smelt-cli/tests/migrate.rs` covering plan-mode recording,
  `--json` shape, and all three `--apply` branches (absent, stale, matching hash); the two
  pre-existing tests were updated for the new exit-3 semantics.

## Decisions

- Plan mode always records the hash it just derived (even though the command still exits `3`) —
  recording is not the same as approval; a human reviewing the printed plan and later running
  `--apply` is the approval act. This is what makes CI's `3` mean "review pending", not "an
  invalid state".
- `approved` (JSON field) reflects whether the store already held this exact hash *before* this
  invocation wrote anything — same computation feeds both plan-mode and `--apply` branching, so
  the JSON output and the exit code always agree.
- Eclipsed plans never touch the approval store (§Surface "Plan-and-approve": "nothing to
  approve") — both plan mode and `--apply` short-circuit to exit 0 before any read/write.
- `plan_hash`'s Encoder is a local copy of `smelt-fingerprint::hash::Encoder` (that crate's is
  `pub(crate)`), per the plan's explicit instruction — not a widened export.

## For the next planner

- Phase 3 (`--apply` execution) needs: a `Backend` connection, per-technique statement execution
  in H-slot order, re-recording the deployed-schema snapshot, and resume semantics. The approval
  gate this phase built is exactly what phase 3's executor checks before running anything — no
  rework expected there.
- Discovered and reverted (not committed): a proptest-property failure in
  `smelt-db/tests/type_property_tests.rs` (`prop_multi_model_type_inference`) —
  `PERCENTILE_DISC(0.5) WITHIN GROUP (ORDER BY <SmallInt>)` infers `Double` where DuckDB returns
  `SmallInt`. Unrelated to this phase (pure smelt-db type inference, no backbuild/migrate code
  touched); the regression file update was discarded to keep this phase's diff scoped. Worth a
  follow-up: `divergences.rs` entry or an inference fix for `PERCENTILE_DISC`'s window/ordered-set
  aggregate return type.
- Phase 1's "For the next planner" item (one `SourceRef` per direct upstream, not per FROM-tree
  alias — self-joins/multi-alias upstreams not yet distinguished) is still open; unaffected by
  this phase.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `cargo test -p smelt-logical --test walk_coverage` — 4 passed
- `cargo test -p smelt-state --quiet` — 277 + 5 + 3 passed
- `cargo test -p smelt-cli --test migrate --test exit_codes --features duckdb` — 9 + 4 passed
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` — 4 + 23 passed
- `cargo test -p smelt-core --test hardening_budget` — OK (baseline updated for smelt-cli's
  legitimate new user-facing `println!`s: 176 → 179)

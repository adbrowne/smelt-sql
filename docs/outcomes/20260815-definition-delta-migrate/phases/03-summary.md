# Phase 3 summary — Approval store + `--apply` + `--json`

## Shipped

- `crates/smelt-state/src/migration_approvals.rs` — `MigrationApproval{plan_hash, in_progress}`,
  `MigrationApprovalStore` (record/get/clear), + `FileStore::{load,save}_migration_approvals`
  persisting to `.smelt/targets/<target>/migration_approvals.json`.
- `MigrationPlan::statements` (`crates/smelt-logical/src/backbuild/plan.rs`) — the targeted
  script assembled once via the existing `assemble`/`Selection::Targeted`, folded into
  `plan_hash`. `MigrationPlan::all_rerun_safe()` added alongside it.
- `smelt migrate --apply` and `--json` (`crates/smelt-cli/src/commands/migrate.rs`, `main.rs`
  `MigrateArgs`): plan step records approval unconditionally and reports whether it was already
  on record; `--apply` re-derives, compares hashes, executes via `helpers::create_backend` +
  `Backend::execute_sql`, records the new schema via `smelt_runtime::schema_evolution::
  save_deployed_schema`, then clears the approval.
- `commands::migrate::exit_code_for` — `3` for pending-approval/apply-refusal, `1` for
  full-refresh-required, else the shared classifier; wired into `main.rs` alongside `init`/`list`.
- `docs/specs/cli.md` §"Exit codes" gains code `3` + `smelt migrate` specifics;
  `docs/specs/definition_deltas.md` §Surface "`smelt migrate`" and §Known Divergences updated;
  `docs-site/docs/reference/cli.md` gains exit code `3` and a new `## smelt migrate` section.
- Tests: 3 in `smelt-state` (unit), 3 new in `smelt-logical/tests/migration_plan.rs`, 8 in new
  `smelt-cli/tests/migrate_apply.rs`; 2 existing `migrate_plan.rs` assertions updated for the
  exit-3 contract.

## Decisions

See outcome.md "## Decision log", 2026-08-29 phase-3-implementation entry — approval semantics
(unconditional record + before-call hash match for the exit code), `--apply` never
self-approving on refusal, `all_rerun_safe()` addition, and the hardening-baseline update are
recorded there rather than duplicated here.

## For the next planner

- Phase 3b (`smelt run` refusal + `smelt explain`/plan surfacing) is next in table order and was
  explicitly *not* touched here — this phase only wires the migrate verb itself.
- Resume is coarser than `definition_deltas.md` §"Frontier semantics" describes (marker-based,
  not per-region) — already flagged as a known divergence tracking phase 11; nothing new here.
- `docs-site/docs/guide/backbuild-synthesis.md` still needs its narrative rewrite (phase 8) — the
  reference-doc flags/exit-code additions in this phase are deliberately minimal, per task 9's
  scope note.
- Not exercised by any test: `--apply` on a plan whose in-progress marker is `true` AND
  `all_rerun_safe()` is `true` (should resume, i.e. re-execute from the start) — the plan's test
  list only covers the non-rerun-safe refusal leg. Worth a follow-up test if a future phase
  touches this path.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings both feature
  sets, full workspace `cargo test`, `example_diagnostics`)
- `cargo test -p smelt-state --lib migration_approvals` — 3 passed
- `cargo test -p smelt-logical --test migration_plan` — 11 passed
- `cargo test -p smelt-cli --test migrate_plan` — 4 passed
- `cargo test -p smelt-cli --test migrate_apply` — 8 passed
- `cargo test -p smelt-runtime --test statement_parity` — 23 passed
- `cargo test -p smelt-logical --test walk_coverage` — 4 passed

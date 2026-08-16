# Phase 3 summary — `--apply` execution

**Shipped:**
- `smelt migrate <model> --apply` on a matching plan hash now executes: each column group's
  first presented candidate, one transactional `StatementGroup` per group, in plan order
  (`smelt_logical::backbuild::statement_group_for_candidate`,
  `crates/smelt-logical/src/backbuild/plan.rs`).
- `smelt_runtime::migrate::apply_migration_plan` (`crates/smelt-runtime/src/migrate.rs`): checks
  admission over **every** group first (`first_refusal`) — a skeleton-change group, a
  candidate-less group, or a destructive (`ColumnDrop`) first candidate refuses the whole plan,
  executing nothing (`MigrationApplyRefusal`). Otherwise executes group by group, skipping labels
  already in `already_applied`, invoking `on_group_applied` after each commit so a caller can
  persist resume progress incrementally (including on a later group's `MigrationApplyError::Backend`
  failure).
- `MigrationApprovalStore` (`crates/smelt-state/src/migration_approvals.rs`) gained
  `applied_groups: Vec<String>` / `applied_at` (`#[serde(default)]`, fail-closed for pre-existing
  approval files) and `record_applied_group`; `record` (a new hash) clears both — a different plan
  resumes nothing.
- `commands/migrate.rs`: restored `--database`, qualifies `facts.table` with `target_config.schema`
  (consistently for both plan and apply, so the plan hash doesn't drift between them), opens a
  real backend via `CliBackendFactory` under `--apply`, persists applied-group progress to the
  approval store as groups commit, and on full success calls `save_deployed_schema` so the next
  plan step is eclipsed. `--json` gained `applied` / `applied_groups`.
- Spec delta landed in `docs/specs/definition_deltas.md` (Surface, Known Divergences — two
  bullets deleted, two narrower ones added), `docs/specs/cli.md` (exit `3` wording widened to
  cover an approved-but-refused-to-execute plan), `docs/specs/run_state.md` (approvals file
  comment).

**Decisions:**
- Table qualification (`schema.table`) had to move to *before* plan derivation, not just inside
  the apply branch — otherwise the plan hash would differ between plan mode and apply mode and
  `--apply` would never see `previously_approved == true`.
- A backend failure mid-loop (`MigrationApplyError::Backend`) is distinct from a refusal
  (`MigrationApplyError::Refused`): refusals happen before anything executes and map to exit `3`;
  a backend failure after some groups already committed bubbles up as a plain error (not exit `3`)
  since the honest state now is "partially applied, needs a human", not "unapproved".

**For the next planner:**
- CLI-level crash/resume durability (killing `smelt migrate --apply` between two groups, then
  re-invoking) is implemented but not covered by an integration test — the `smelt-runtime`
  `apply_skips_groups_already_recorded_applied` test proves the pure function's resume logic; a
  CLI-level test would need to inject a mid-loop failure into a real backend, which the current
  test harness doesn't support cheaply. Worth adding if this path gets exercised in anger.
- `already_applied` labels are `ColumnGroupPlan::label` strings (e.g. `"added column 'x'"`) —
  stable only as long as the diff/classify pipeline's `atom_label` output doesn't change; a label
  rename would silently orphan any in-flight resume record (old labels no longer match). Not a
  problem today since a hash change already clears `applied_groups`, but worth a comment if
  `atom_label` is ever refactored.
- Destructive legs (`ColumnDrop`) remain refused rather than executed — their verification probes
  aren't emitted yet (§Known Divergences). That's still open, tracked at the outcome level, not
  scoped into this phase.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `cargo test -p smelt-cli --test migrate --test exit_codes --features duckdb` — 13 + 4 passed
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` — 23 + 4 passed
- `cargo test -p smelt-state --quiet` — 277 + 3 + 5 passed
- `cargo test -p smelt-core --test hardening_budget` — passed after updating
  `.claude/hardening-baseline.txt` (`smelt-cli println 179 → 181`, two new genuinely user-facing
  `--apply` output lines: the applied-group summary and "definition recorded")

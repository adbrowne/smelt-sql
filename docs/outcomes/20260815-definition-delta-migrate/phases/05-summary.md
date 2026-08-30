# Phase 5 summary — definition-edit step kind in the generative conformance suite

**Shipped:**
- `smelt_runtime::definition_delta::apply_migration(backend, plan)` (`crates/smelt-runtime/src/definition_delta.rs`) — extracted statement-execution core; `smelt-cli/src/commands/migrate.rs::apply_plan` now calls it instead of looping inline.
- `ConformanceStep::MigrateModel { edit }` (`crates/smelt-maintenance-testkit/src/schedule_gen.rs`) — rewrite → derive → apply-or-refuse → assert immediately, via the real `smelt migrate` backbuild path (not the live `Trigger::ColumnAdded` driver path).
- Shared driver `run_migrate_step` in new `crates/smelt-maintenance-testkit/src/migrate_step.rs`, returning `MigrateStepOutcome::{Applied, FullRefreshed}`; consumed identically by `maintenance_conformance/gate.rs::drive_and_assert` and `families/gate.rs::drive_and_assert_for`.
- `arb_schedule_with_definition_edit(recipe)` — new generator, `arb_schedule_for`/`build_schedule` untouched (Spark/BigQuery twins unaffected).
- 4 new tests (1 unit in `smelt-runtime`, 3 in `smelt-cli/tests/maintenance_conformance/gate.rs`: 2 pinned + 1 generative). All green, including the anti-vacuity "at least one case took the Applied leg" check.
- `docs/specs/definition_deltas.md` §Known Divergences — "The conformance harness has no definition-edit step kind yet" bullet removed.

**Decisions:**
- The `MigrateModel` step drives the shipped `smelt migrate` backbuild mechanism, deliberately distinct from `RewriteModel`'s pre-migrate contract (whatever's on disk compiles on the next run) — both step kinds now coexist, `RewriteModel`'s doc comment corrected to stop calling the classification "unbuilt."
- `smelt-state` moved from `smelt-maintenance-testkit`'s dev-dependencies to normal dependencies since the driver lives in `src/`, not `tests/`.
- Real bug fix folded into this phase rather than deferred: the generative test surfaced two genuine `smelt_logical::backbuild` bugs (aggregate-shaped column-add wrongly admitted `SelfDerivedColumnAdd`; `try_b5`'s re-aggregation subquery spliced unresolved `smelt.<path>` ref syntax). Both fixed — `expr_contains_aggregate` gating on B1/B3, and a new `requalify::requalify_source_refs` CST pass plus a `build_sources_map` fallback to the default materialization name. This is implementation-correctness work the outcome's existing success criteria already own (the migrate plan must be actually executable), not scope creep.

**For the next planner:**
- No follow-up work identified outside this phase's scope; all listed verification gates pass with zero regressions across the 74-case `maintenance_conformance` suite, `statement_parity` (23), `migrate_plan` (4), `migrate_apply` (9), and the full workspace `verify-phase.sh`.
- The backbuild classifier fix (aggregate-shaped adds, ref-requalification) touched `crates/smelt-logical/src/backbuild/classify.rs` and `requalify.rs`, and `crates/smelt-parser/src/ast.rs` (new `.syntax()` accessors on `FromClause`/`WhereClause`) — worth a note for phase 9's validate sweep in case it surfaces related drift.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` (full) — PASS (fmt, clippy both feature sets, full `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test maintenance_conformance` — 74/74 pass.
- `cargo test -p smelt-runtime --test statement_parity` — 23/23 pass.
- `cargo test -p smelt-cli --test migrate_plan` — 4/4 pass.
- Hardening baseline unchanged (verified via `hardening_budget` gate inside `verify-phase.sh`).

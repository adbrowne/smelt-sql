## Drift Report: definition_deltas

**Spec**: docs/specs/definition_deltas.md (last_reviewed: 2026-09-03)
**Date**: 2026-09-04

### Automated checks
- cargo fmt — PASS (run once for this outcome per prior automated pass; not re-run here)
- cargo clippy — PASS (run once for this outcome per prior automated pass; not re-run here)
- cargo test — PASS (run once for this outcome per prior automated pass; not re-run here)
- example_diagnostics — PASS (run once for this outcome per prior automated pass; not re-run here)

### Surface drift
- ✅ `smelt migrate <model>` / `--apply` / `--json` exist: `crates/smelt-cli/src/commands/migrate.rs`, wired via `crates/smelt-cli/src/main.rs` `Commands::Migrate` / `MigrateArgs`; documented at `docs-site/docs/reference/cli.md` §"smelt migrate" (worked example matches the spec's `order_facts` example, including plan-hash-gated `--apply`).
- ✅ `smelt rebuild --event-time-start/--event-time-end [selectors]` exists and is documented as the data-side ranged re-run verb, disjoint from `smelt migrate`, at `docs-site/docs/reference/cli.md` §"smelt rebuild".
- ✅ Diagnostics `MaintenanceSkeletonChanged` and `DefinitionDeltaPending` are implemented (found across `smelt-db`, `smelt-runtime`, `smelt-cli`, `smelt-lsp`) and exit-code contract (`3` for pending approval / stale approval) matches `crates/smelt-cli/src/commands/migrate.rs::MigrateError` mapping and `docs-site/docs/reference/cli.md`'s exit-code table.
- ✅ `MaintenanceColumnAddNotBackfillable` (Warning posture for non-backfillable column adds) exists in code and has a dedicated test (`crates/smelt-logical/tests/maintenance_column_add_not_backfillable.rs`).
- ✅ Verdict vocabulary (eclipsed / backfill in place / re-derive / skeleton change) appears consistently in `docs-site/docs/guide/backbuild-synthesis.md` (e.g. "backfill in place SelfDerivedColumnAdd").
- ✅ Plan-hash / approval-store mechanics (`crates/smelt-state/src/migration_approvals.rs` consumed via `MigrationApprovalStore` in `migrate.rs`) match §"`smelt migrate`"'s "Approve and apply" / "Resume" description.

### Semantics drift
- ✅ Per-column verdict classification: `crates/smelt-logical/src/backbuild/classify.rs`, exercised by backbuild module tests and `crates/smelt-cli/tests/maintenance_conformance/gate.rs` (`ConformanceStep::RewriteModel`, `definition_edit_pool_upholds_new_definition_equivalence`, `migrate_step_applies_plan_and_recovers_new_definition_equivalence`).
- ✅ Skeleton-change refusal: `crates/smelt-logical/src/maintenance/skeleton.rs` + `crates/smelt-logical/tests/maintenance_skeleton.rs`.
- ✅ Atomicity rule (schema + backfill in one statement group, rerun-safe reconciliation without transactional DDL): `BackbuildOption::rerun_safe` / `MigrationPlan::all_rerun_safe()` in `crates/smelt-logical/src/backbuild/{mod,plan}.rs`; approval-in-progress / resume path in `migrate.rs`.
- ✅ Conformance gate covers definition edits mid-history (§"The oracle" / Constraints & Invariants last bullet): `crates/smelt-cli/tests/maintenance_conformance/gate.rs` has an explicit `RewriteModel` step kind and `definition_edit_pool_upholds_new_definition_equivalence`.
- ✅ Plan-and-approve / plan-hash-covers-data-not-just-SQL: `plan_hash()` in `crates/smelt-logical/src/backbuild/plan.rs` hashes the `MigrationPlan`+`BackbuildInputs` structure, matching §Design "The plan hash covers the plan data structure, not only rendered SQL."
- ✅ No test-uncovered normative rule was found among the References → Tests set; all five listed test paths exist and are non-trivial (`maintenance_skeleton.rs`, `maintenance_tracer_evolution.rs`, `tracer_evolution.rs`, `targeted_column_backfill.rs`, plus the backbuild module tests and the conformance gate).

### Invariant drift
- ✅ "Nothing destructive runs unapproved" — `migrate.rs` gates `--apply` on `already_approved()` matching the freshly re-derived hash; refuses otherwise (`MigrateError::ApplyRefused`).
- ✅ "One frontier" / "one statement author" — migration statements route through the same backbuild emitters (`crates/smelt-logical/src/backbuild/emit.rs`) as maintenance statements; no separate authoring path found.
- ✅ "Skeleton changes are never migrated in place" — verified above via `maintenance/skeleton.rs`.
- ✅ "Admission is fail-closed" — `classify.rs`/`plan.rs` fall back to full-refresh presentation per group; consistent with `crates/smelt-cli/tests/maintenance_conformance/gate.rs` phase-9/backfill assertions.
- ⚠️ "One frontier … no second bookkeeping system" for the snapshot-reconcile-model transient frontier (§"The catch-up unit") was not independently re-derived from a live probe in this pass — accepted on the strength of the existing conformance-gate coverage rather than a fresh manual trace; flag for manual review if a future audit has more budget.

### Timeless-oracle drift
- ✅ No phase-vocabulary leakage detected in spec body (`rg -n "Phase [A-Z0-9]+" docs/specs/definition_deltas.md` — zero hits).
- ✅ No phase-vocabulary leakage in the two referenced docs-site pages (`backbuild-synthesis.md`, `reference/cli.md` — zero hits).
- ✅ The one Known Divergences bullet ("Resume is approval-marker-based, not frontier-region-scoped") cites `docs/outcomes/20260815-definition-delta-migrate/outcome.md` phase 12 by name — an outcome-doc tracking link, the form this spec's own header sanctions ("Implementation status lives in §Known Divergences … or §References → Plans"). Verified the outcome doc explicitly still lists frontier-region-scoped resume as a stated divergence (line 862: "frontier-region-scoped resume per §'Frontier semantics' stays a stated divergence"), so the bullet is accurate, not stale.
- ✅ DD-01 through DD-07 from `docs/outcomes/20260815-incremental-spec-closure-confirm/baseline-inventory.md` are all `closed <sha>` and no longer present in the spec body — consistent, not drift. — flagged-open: none apply (all closed).

### Freshness
- last_reviewed: 2026-09-03
- most recent code change among §References → Code paths: `crates/smelt-runtime/src/maintenance_driver.rs` at 2026-09-04T06:40:57+10:00 (commit `6bb11ffc`, "refuse a partition_column rename with a named diagnostic") — post-dates last_reviewed by <1 day, but the diff only threads an unrelated extra `None` parameter through `resolve_incremental_strategy`/sibling functions for a new `MaintenancePartitionColumnChanged` diagnostic (partition-column rename detection, owned by `incremental_shapes.md`/`schema_evolution.md`, not definition-delta semantics) — not substantive to this spec's content. All definition-delta-specific paths (`backbuild/*.rs`, `analysis/definition_change.rs`) last changed 2026-08-29/08-30/08-12, well before last_reviewed.
- Verdict: fresh

### Summary
- Drift items: 0 confirmed drift (0 surface, 0 semantics, 0 invariant, 0 timeless-oracle). One invariant (⚠️) flagged as not independently re-verified rather than as a defect — no fix applied, no phase row needed.
- No inline doc fixes were required (no files edited).
- Nothing needs a phase row or a product decision.
- Recommended next step: none — spec is fresh and matches implementation, tests, and docs-site.

# Plan: Maintenance-plan implementation (surface cut + M1–M6 + graph layer)

**Date**: 2026-07-07
**Spec**: [`docs/specs/maintenance_plan.md`](../specs/maintenance_plan.md) — with [`models.md`](../specs/models.md) (refresh axis / grain / contract), [`sources.md`](../specs/sources.md) (world-facts admission consumes), [`model_transforms.md`](../specs/model_transforms.md) (technique primitives), [`model_maintenance.md`](../specs/model_maintenance.md) (the invariant the plan refines)
**Spec diff**: `3f65a671` + `aa326a3f` + `fb9a5977` (the refresh-as-maintenance-plan spec landing); migration ordering ratified in `docs/research/20260705-refresh-as-maintenance-plan/08-code-placement.md` §2.8 (M0 done: `7d1b4f17`; F1 done: `770c77f1`; F4 done: `25c04a70`)
**Tracking PR / branch**: `worktree-incremental`
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/maintenance_plan.md` **completely** — it is the correctness oracle. Read the §Surface/§Semantics sections of `models.md` and `sources.md` that the phase you're executing cites. Do not re-open settled spec decisions (ratification record: `docs/research/20260705-refresh-as-maintenance-plan/09-spec-readiness.md` §1).
2. Confirm you are on branch `worktree-incremental`. If not, ask the user before continuing.
3. Confirm `DUCKDB_LIB_DIR` and `LD_LIBRARY_PATH` are exported (DuckDB equivalence tests skip green without them — that is a silent hole, not a pass).
4. Find the next phase whose status is `pending` in the Progress tracking table. If every phase is `done`, run "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule (fail-loud, purity, parity — see below).
- A spec assumption turns out to be wrong (run `/smelt:spec` first).
- `cargo test` / `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**

- Real-fixture tests in `examples/`, not just unit tests. Red-green TDD.
- Atomic per-phase commits with the phase's `Commit.` line verbatim. Never `--no-verify`.
- Don't widen scope: a phase may not reach into a later phase's scope.
- **Architectural invariants (CLAUDE.md) bind every phase**: Salsa purity (queries are thin wrappers over pure functions); run-pipeline parity (all lowering behind `execute_project`; `cargo test -p smelt-runtime --test execute_parity` green); layered single-ownership (`smelt-db` never depends on `smelt-planner`; the plan model lives in `smelt-logical`); fail-loud (new refusals are named diagnostics; hardening budgets hold); **plan purity** (the maintenance plan is pure data derived by pure functions in `smelt-logical/src/maintenance/`; no consumer re-derives it).
- **Timeless-oracle rule.** Spec/user-doc edits are timeless; each phase that narrows a §Known Divergences entry in `maintenance_plan.md`/`models.md`/`sources.md` updates that entry in behavioural terms.
- The tracer suites (`maintenance_tracer*.rs`, `tracer_*.rs` property-discovery legs) are the regression floor: they stay green through every phase; when a phase promotes a hand-supplied tracer input to a real derivation, it converts the corresponding hand-supplied fixture into an asserted derivation, never deletes coverage.

---

## Context

The spec set now describes the derived maintenance plan (`maintenance_plan.md`), the refresh trichotomy + declared grain (`models.md`), and the source world-facts admission consumes (`sources.md`). Code-side, everything is either pre-landed substrate (fundamentals F1–F15; the L3 declarations; the tracer v0 in `crates/smelt-logical/src/maintenance/`) or not yet wired: `RefreshStrategy` still parses the removed mode names, `resolve_strategy` returns a constant, the three hardest derivations are hand-supplied, the ledger substrate is frontier-only per-model, and never-fold-twice is a confirmed live violation (G-12). This plan drives code + user docs to the spec along the ratified M-ladder, then the graph layer.

## Scope

### In scope (spec coverage)

- `models.md` §"Refresh axis": the surface cut — `full | incremental | materialized_view` + `grain:`, old names hard-error with fix-its; `columns.<c>.contract`.
- `sources.md` §"`mutation_profile` — the structured block": structured parse + trust-rule validation.
- `maintenance_plan.md` §Surface (frontmatter, CLI, diagnostics), §Semantics (matrix, admission, K8, definition-change, ledger, graph layer) — the M1–M6 ladder plus forward/backward propagation.
- `docs-site/` refresh-surface rewrite (seeded from the worked example catalogue).

### Explicitly deferred

- Sub-day propagation grain; keyed-grain dirt-sets; time-unrolled self-edges in the graph (designed, refusing fail-loud — `maintenance_plan.md` §Known Divergences; revisit after this plan).
- Straddle attribution without locality (ledger v1 is locality-or-explicit-footprint by spec).
- CDF/snapshot-diff delta detection (v1 is append-only interval diff per P10; `change_feed` admission arms land but their delta *detection* trails).
- Backend-derived source facts (Known Divergence → `multi_backend.md`).
- The `on_column_add:` policy knob (noted-not-surface in `models.md`).
- Re-cut shape-profile compositions from the superseded L4 sub-plans (versioned SCD-2 executor, native-IVM delegation) — re-scaffolded from evidence after this plan lands the plan machinery.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| MP1   | done     | `b1cde7d9` | 2026-07-08 |
| MP2   | done     | `e58a9de6` | 2026-07-08 |
| MP3   | done     | `db2da533` | 2026-07-08 |
| MP4   | done     | `bf0db5d4` | 2026-07-08 |
| MP5   | done     | `16acc036` | 2026-07-08 |
| MP6   | done     | `c410d4bd` | 2026-07-09 |
| MP7   | done     | `fbd0141d` | 2026-07-09 |
| MP8   | done     | `25d397c5` | 2026-07-09 |
| MP9   | done     | `89676f9e` | 2026-07-10 |
| MP10  | done     | `0a1cab1f` | 2026-07-10 |
| MP11  | pending  |        |      |
| MP12  | pending  |        |      |
| MP13  | pending  |        |      |
| MP14  | pending  |        |      |
| MP15  | pending  |        |      |
| MP16  | pending  |        |      |
| MP17  | pending  |        |      |

---

### Phase MP1: The refresh-axis surface cut

**Goal.** `RefreshStrategy` becomes the trichotomy; `grain: partition | key | key_per_partition` is parsed and required with `incremental`; the removed names (`batched`, `keyed`, `cumulative`, `versioned`) hard-error with fix-its naming the replacement (`models.md` §"Refresh axis"). Internal dispatch re-keys off `(refresh, grain)` — behaviourally `Batched ≡ (Incremental, Partition)` and `Keyed ≡ (Incremental, Key)`; no execution change.

**Pre-conditions.** None (all substrate landed).

**TDD tests to write first.**
- `crates/smelt-core/tests/config_refresh_axis.rs::incremental_requires_grain` — `refresh: incremental` without `grain:` is a config error naming the missing key.
- `crates/smelt-core/tests/config_refresh_axis.rs::removed_names_error_with_fixit` — each of `batched`/`keyed`/`cumulative`/`versioned` errors, message contains `refresh: incremental` and `grain:`.
- `crates/smelt-cli/tests/example_diagnostics.rs` (existing) — stays green after examples migrate.
- One real fixture: `examples/timeseries/` migrated to `refresh: incremental` + `grain: partition` builds identically (existing e2e assertions unchanged).

**Implementation shape.** `smelt-core/src/config.rs`: enum → `Full | Incremental | MaterializedView`; new `grain` field; serde-level rejection with fix-it text. Sweep every `RefreshStrategy::Batched/Keyed` match site to `(Incremental, grain)`. Migrate `examples/` (including the `examples/huge` generator) and test fixtures mechanically.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-core/src/config.rs`; every `RefreshStrategy` match site (`smelt-logical`, `smelt-runtime`, `smelt-db`, `smelt-cli`, `smelt-planner`)
- `examples/**` (mechanical migration), test fixtures

**Docs touched.** *Timeless.*
- `docs/specs/models.md` — §Known Divergences: narrow the "refresh surface unimplemented" entry (parse state now matches spec).

**Review checklist**:
- [ ] TDD tests exist and assert the fix-it text
- [ ] No behaviour change: e2e suites green unmodified (except frontmatter)
- [ ] `example_diagnostics` + `execute_parity` green
- [ ] Spec Known-Divergence narrowed, timeless

**Commit.** `feat(config): refresh trichotomy + declared grain; removed mode names hard-error with fix-its`

---

### Phase MP2: docs-site refresh-surface rewrite

**Goal.** User docs describe only the trichotomy + grain. The ~9 stale pages (`index.md`, `guide/incremental-models.md`, `guide/sql-models.md`, `guide/materializations.md`, `concepts/how-it-works.md`, `reference/timeseries.md`, `reference/smelt-yml.md`, `reference/cumulative-aggregate.md`, `reference/cli.md`) are rewritten; `reference/cumulative-aggregate.md` becomes the key-grain patterns page. Seed worked examples from `docs/research/20260705-refresh-as-maintenance-plan/07-example-catalogue.md`.

**Pre-conditions.** MP1 (never document surface the parser rejects).

**TDD tests to write first.**
- `rg -n 'refresh: (batched|keyed|cumulative|versioned)' docs-site/docs/` → zero matches (red now, green after).
- Any doc-snippet extraction test the site build runs stays green.

**Critical files (allowed to touch in this phase).**
- `docs-site/docs/**` (not `docs-site/site/` — build artifact)

**Docs touched.** *Timeless.* The pages above; `docs/specs/maintenance_plan.md` §Known Divergences — narrow the "user docs do not exist" entry.

**Review checklist**:
- [ ] Zero removed-mode occurrences in docs-site/docs
- [ ] Examples in pages parse under the new surface
- [ ] Timeless (no plan/phase vocabulary)

**Commit.** `docs(site): refresh-surface rewrite — trichotomy + grain across all pages`

---

### Phase MP3: Structured `mutation_profile` parse + trust-rule validation

**Goal.** The `sources.md` §Surface block parses: structured `mutation_profile` (kind + sub-facts, `key_recurrence` subsumed), `watermark`, composite `unique_key`, `retention`; the trust rule (widening trusted / narrowing verified) validated; bare-string shorthand + `source_lateness` alias with double-declare error. Admission (MP5+) reads these; runtime tripwires trail with the phases that execute the licensed techniques.

**Pre-conditions.** None (independent of MP1).

**TDD tests to write first.**
- `crates/smelt-core/tests/source_world_facts.rs::structured_block_parses_all_kinds` — the three kinds + sub-facts round-trip.
- `crates/smelt-core/tests/source_world_facts.rs::lateness_alias_double_declare_errors`
- `crates/smelt-core/tests/source_world_facts.rs::unique_key_is_composite_valued` — single string ≡ one-element list.
- Real fixture: a source `.yml` in `examples/timeseries/` carries a structured block; `example_diagnostics` green.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-core/src/` (source config), `crates/smelt-db/src/` (validation diagnostics), example source yml

**Docs touched.** *Timeless.*
- `docs/specs/sources.md` — §Known Divergences: narrow the parse-state entry.

**Review checklist**:
- [ ] Unrecognised keys fail loud (no silent default posture)
- [ ] Old flat `mutation_profile`/L3 shape migrated, not dual-supported

**Commit.** `feat(sources): structured mutation_profile block + trust-rule validation; key_recurrence subsumed`

---

### Phase MP4: Mutation-sensitivity column grouping + skeleton-role extraction (M1a)

**Goal.** The two hardest derivations, as pure functions in `crates/smelt-logical/src/maintenance/`: per-column mutation-sensitivity from column provenance × source `mutation_profile` with fail-closed group merging and surfaced degenerate collapse (`maintenance_plan.md` §"The plan matrix"), and skeleton-role extraction (membership/grouping/dedup/ordering positions). The tracer's hand-supplied groups/roles become asserted derivations.

**Pre-conditions.** MP3 (mutation profiles as input).

**TDD tests to write first.**
- `crates/smelt-logical/tests/maintenance_grouping.rs::immutable_at_creation_reference_contributes_no_sensitivity` — the load-bearing append-only case (§Design "Factoring by mutation-sensitivity").
- `crates/smelt-logical/tests/maintenance_grouping.rs::two_source_projection_merges_groups_fail_closed`
- `crates/smelt-logical/tests/maintenance_grouping.rs::degenerate_collapse_is_surfaced` — collapse reported, never silent.
- `crates/smelt-logical/tests/maintenance_skeleton.rs::group_by_and_dedup_columns_are_skeleton`
- Convert ≥ 3 `maintenance_tracer.rs` fixtures from hand-supplied to derived (EX-02/07/13 shapes) with unchanged expected plans.

**Implementation shape.** `maintenance/grouping.rs` + `maintenance/skeleton.rs`; consume existing provenance/AST analysis from `smelt-logical/src/analysis/`. Pure; no Salsa, no I/O.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/`, `crates/smelt-logical/tests/`

**Docs touched.** *Timeless.*
- `docs/specs/maintenance_plan.md` — §Known Divergences: narrow "the three hardest derivations are unbuilt" (two of three).

**Review checklist**:
- [ ] Pure functions; unrecognised constructs merge/fail closed, never default
- [ ] Tracer suite green with derived (not hand-fed) inputs

**Commit.** `feat(logical): mutation-sensitivity column grouping + skeleton-role extraction`

---

### Phase MP5: Production `derive_maintenance_plan` describing today's behaviour (M1b)

**Goal.** `derive_maintenance_plan(analysis facts × source declarations × declared shape/grain) → MaintenancePlan | refusals` promoted from tracer to production: full per-cell admission (§"Per-cell admission" obligations 1–6), partition-locality verdicts, per-column guarantee ledger — *describing* what execution does today (every partition-grain cell recompute-region; key-grain fold via the driver). No execution change. `input_delta_discovery` and `fan_out` become admission inputs (the FIX-2 tripwire inverts to a consumption assertion).

**Pre-conditions.** MP4.

**TDD tests to write first.**
- `crates/smelt-logical/tests/maintenance_plan_conformance.rs::described_technique_matches_execution` — for each `model_shapes.rs` catalogue shape: derived cell technique == what `execute_project` actually emits (DuckDB leg; skips loudly without `DUCKDB_LIB_DIR`).
- `crates/smelt-logical/tests/maintenance_plan_admission.rs::holistic_combiner_leaves_recompute_only` (obligation 3)
- `crates/smelt-logical/tests/maintenance_plan_admission.rs::retractions_into_noninvertible_fail_faithful_fold` (obligation 2's independence)
- `crates/smelt-logical/tests/input_delta_consumed.rs::plan_derivation_consumes_input_delta_discovery` — replaces the dead-code tripwire, same SC-2 pointer in the failure message.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/{mod,derive}.rs`, tests; delete `input_delta_discovery_dead_code_tripwire.rs` (replaced)

**Docs touched.** *Timeless.*
- `docs/specs/maintenance_plan.md` — §Known Divergences: narrow "specified-and-unwired" (derivation now production; consumers still pending).

**Review checklist**:
- [ ] Admission is fail-closed per obligation, each refusal a named reason (diagnostic codes wired in MP6)
- [ ] Conformance test proves description, not aspiration
- [ ] Tripwire inverted, not dropped

**Commit.** `feat(logical): production derive_maintenance_plan — admission, locality verdicts, guarantee ledger`

---

### Phase MP6: Salsa query, `Maintenance*` diagnostics, `maintenance:` frontmatter (M2a)

**Goal.** Thin `maintenance_plan(file)` Salsa query in `smelt-db`; plan refusals and declared-shape/grain mismatches fold into `file_diagnostics()` as the `Maintenance*` codes (`maintenance_plan.md` §Diagnostics); the `maintenance:` frontmatter block parses (`defaults.prefer`, `cells[]` with the two-group column error, `scan_bounds` with K8 defaults `require: partition_local` + `on_violation: error`, project-level baseline in `smelt.yml`). `columns.<c>.contract` parses and feeds the guarantee ledger.

**Pre-conditions.** MP5.

**TDD tests to write first.**
- `crates/smelt-db/tests/maintenance_diagnostics.rs::unbounded_scan_refuses_by_default` — a cross-axis source with no derivable predicate → `MaintenanceScanUnbounded`; `allow_full_scan: true` clears it.
- `crates/smelt-db/tests/maintenance_diagnostics.rs::grain_mismatch_is_error_never_silent` — declared `grain: key` vs partition-shaped plan → error.
- `crates/smelt-db/tests/maintenance_diagnostics.rs::cells_columns_spanning_groups_error`
- Real fixture: a model in `examples/broken/` exercising `MaintenanceScanUnbounded`; `example_diagnostics` asserts the expected diagnostic set.
- `crates/smelt-db/tests/diagnostics_catalogue.rs` — green with the new enum variants (rows landed in the spec-alignment plan).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/` (query + diagnostics), `crates/smelt-core/src/` (frontmatter/config), diagnostic enum + `map_metadata_error_to_diagnostic` exhaustiveness, examples

**Docs touched.** *Timeless.*
- `docs/specs/diagnostics.md` — flip catalogue rows from specified-unimplemented; `docs/specs/maintenance_plan.md` §Known Divergences narrowed.

**Review checklist**:
- [ ] Salsa purity: the query only assembles inputs and calls the pure derivation
- [ ] Every new refusal path has a catalogued code; hardening budgets hold
- [ ] LSP sees the diagnostics (cargo LSP example-workspace test green)

**Commit.** `feat(db): maintenance_plan Salsa query + Maintenance* diagnostics; maintenance/scan_bounds frontmatter`

---

### Phase MP7: `smelt explain <model>` (M2b)

**Goal.** The plan-report CLI (`maintenance_plan.md` §CLI): cells, per-cell corner/technique, derived clamps, locality verdicts, per-column guarantee ledger, inbound edges. Read-only; consumes the MP6 query path (no re-derivation).

**Pre-conditions.** MP6.

**TDD tests to write first.**
- `crates/smelt-cli/tests/explain_maintenance.rs::explain_prints_cells_clamps_locality` — golden-ish assertion over `examples/timeseries` (assert structure/required substrings, not byte-exact layout).
- `crates/smelt-cli/tests/explain_maintenance.rs::degenerate_plan_visibly_reported` — the collapsed-group model prints the whole-model story with the collapse called out.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/src/commands/`, `crates/smelt-runtime` only if a shared report type is needed

**Docs touched.** *Timeless.*
- `docs/specs/cli.md` — the `explain` surface; `docs-site/docs/reference/cli.md` — user page.

**Review checklist**:
- [ ] No re-derivation (consumes the query/pure fn)
- [ ] Output names cells by column-group members + trigger, readable to a data engineer

**Commit.** `feat(cli): smelt explain — the maintenance plan report`

---

### Phase MP8: Testkit graduation + standing equivalence gate (M3)

**Goal.** `run_schedule.rs`, `oracle.rs`, `link_c_harness.rs` graduate into a dev-only workspace crate `smelt-maintenance-testkit` (publish = false); the Link-C schedule suite over the `model_shapes.rs` catalogue becomes a standing CI gate ("emitted maintenance ≡ full refresh over adversarial schedules"); `EXPERIMENTAL(property-discovery)` tags come off graduated pieces; per-cell probe tests stay disposable.

**Pre-conditions.** MP5 (conformance suite exists to gate).

**TDD tests to write first.**
- The moved suites themselves are the tests: green from the new crate as dev-dependencies of `smelt-cli` (and `smelt-runtime` if no dep-cycle).
- `crates/smelt-cli/tests/property_discovery/` probe cells still compile/run under the narrowed gate script.

**Critical files (allowed to touch in this phase).**
- new `crates/smelt-maintenance-testkit/`, `Cargo.toml` workspace, `.claude/scripts/property-experimental-gate.sh`, moved test files

**Docs touched.** *Timeless.*
- `docs/specs/maintenance_plan.md` §References → Tests updated.

**Review checklist**:
- [ ] No production dependency on the testkit anywhere
- [ ] Gate is addressable in CI (named test target), documented in CLAUDE.md commands if needed

**Commit.** `test(maintenance): graduate the schedule/oracle harness into smelt-maintenance-testkit; standing equivalence gate`

---

### Phase MP9: Reconciliation ledger, frontier grade (M4)

**Goal.** `smelt-state/src/reconciliation.rs`: entries keyed `(output-region × column-group)` with per-input processed vectors, frontier grade; the two operations — fold-precondition check and **recompute-reset** (a region recompute resets every intersecting entry to exactly the input it read) — exposed to the runtime; subsumes `intervals.rs`'s role for plan-managed models (`maintenance_plan.md` §"The reconciliation ledger"). Frontier state stays in the `.smelt/` file store.

**Pre-conditions.** MP5 (the plan derives each model's ledger schema).

**TDD tests to write first.**
- `crates/smelt-state/tests/reconciliation.rs::recompute_resets_intersecting_entries_exactly`
- `crates/smelt-state/tests/reconciliation.rs::fold_extends_frontier_monotonically`
- `crates/smelt-state/tests/reconciliation.rs::entries_keyed_region_by_group` — two groups over one region advance independently.
- Testkit leg: a fold-then-recompute-then-fold schedule over a real DuckDB model equals full refresh (the safe direction of the interchangeability theorem).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-state/src/`, `crates/smelt-runtime/src/execute.rs` (write the ledger where intervals are written today)

**Docs touched.** *Timeless.*
- `docs/specs/maintenance_plan.md` §Known Divergences — narrow "the ledger substrate is the degenerate case"; `docs/specs/run_state.md` if it catalogues state files.

**Review checklist**:
- [ ] Existing intervals-based behaviour unregressed (idempotency e2e green)
- [ ] Ledger schema derived from the plan, not declared

**Commit.** `feat(state): reconciliation ledger — (region × group) frontier grade with fold-check and recompute-reset`

---

### Phase MP10: Composite unique keys (G-10)

**Goal.** `JoinContext`/`fan_out` generalized to composite keys, consuming the composite `unique_key` from MP3; ≥ 2-column keys prove one-to-one joins. The precondition for the first `fan_out` consumer (MP11), per ratified F2.

**Pre-conditions.** MP3.

**TDD tests to write first.**
- `crates/smelt-logical/tests/join_shape_composite.rs::two_column_key_proves_one_to_one`
- `crates/smelt-logical/tests/join_shape_composite.rs::partial_key_match_is_fan_out` — matching on a strict subset of the composite key must NOT prove one-to-one.
- Lift the ledger's parked G-10 probe cell (`docs/research/property-discovery/catalog.md`) into a testkit schedule test.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/join_shape.rs`, tests

**Docs touched.** *Timeless.* None beyond `sources.md` Known-Divergence narrowing if applicable.

**Review checklist**:
- [ ] Subset-of-key is the reviewed hazard (fail-closed)

**Commit.** `feat(logical): composite unique keys in join-shape analysis (G-10)`

---

### Phase MP11: First targeted-write cell — column-scoped merge behind admission (M5)

**Goal.** `resolve_strategy`'s constant `DeleteInsert` is replaced by reading the chosen technique off the plan cell; `dimension_horizon_merge` (F15's transform) is wired as the column-scoped re-derivation corner **only** where a cell admits it (enrichment-join delta on a mutation-sensitive group, bounded footprint, partition-local or accepted). The driver loop (`maintenance_driver.rs`) becomes the per-cell technique executor. First live cell where execution differs by column group.

**Pre-conditions.** MP5, MP6, MP9, MP10.

**TDD tests to write first.**
- Testkit schedule: a fact+dimension model where a dimension delta triggers column-scoped MERGE on the enrichment group only, then equivalence vs full refresh (the EX-13/EX-24 family from the example catalogue).
- `crates/smelt-runtime/tests/technique_lowering.rs::unadmitted_cell_never_lowers_targeted_write` — a cell failing bounded-footprint refuses at plan time (`MaintenanceUnboundedFootprint`), no runtime fallback.
- `execute_parity` green (parity rule).
- Real fixture: an `examples/` workspace with the fact+dimension shape, exercised end-to-end.

**Implementation shape.** Lowering reads `MaintenancePlan` cells; `smelt-backend`'s column-scoped merge primitive gets the trait-level home with refusing defaults (capability gap drops the technique from admission at plan time).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/{execute,maintenance_driver,dimension_horizon_merge}.rs`, `crates/smelt-backend/src/lib.rs`, `crates/smelt-backend-duckdb`, examples

**Docs touched.** *Timeless.*
- `docs/specs/maintenance_plan.md` + `model_transforms.md` §Known Divergences narrowed; docs-site incremental-models page gains the enrichment-delta story.

**Review checklist**:
- [ ] Technique only ever chosen from the admitted set (validator-not-chooser holds)
- [ ] Parity + example_diagnostics + full tracer/testkit suites green
- [ ] Backend capability gap = plan-time drop, not runtime surprise

**Commit.** `feat(runtime): plan-driven technique lowering; column-scoped merge live behind admission`

---

### Phase MP12: Per-delta ledger grade + fold-refusal — closes G-12 (M6a)

**Goal.** Additive groups record delta identities (warehouse-resident smelt-managed state tables via `smelt-state`'s backend-DDL seam, transactional with the fold); every fold consults the ledger and **refuses a delta already reflected** (`maintenance_plan.md` §Constraints "Never fold a delta already reflected in the state"). The G-12 pinned live violation flips from documented-violation to enforced.

**Pre-conditions.** MP9, MP11.

**TDD tests to write first.**
- Flip `crates/smelt-cli/tests/property_discovery/g_12_keyed_merge_reprocessed_window.rs` from pinning the double-fold to asserting the refusal (keep the ledger-entry pointer in its docstring).
- `crates/smelt-state/tests/reconciliation.rs::per_delta_grade_lives_in_warehouse` — delta-identity state table created transactionally with the fold (DuckDB).
- Testkit schedule: re-run of an already-merged window is a no-op-with-diagnostic, then equivalence vs full refresh.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-state/src/`, `crates/smelt-runtime/src/` (fold path), `crates/smelt-backend*` (ledger DDL/DML)

**Docs touched.** *Timeless.*
- `docs/specs/maintenance_plan.md` §Known Divergences — remove the never-fold-twice violation entry (now enforced); `docs/research/property-discovery/ledger.md` G-12 row updated.

**Review checklist**:
- [ ] Refusal is transactional with the merge (no check-then-act race across the write)
- [ ] Frontier-grade models unaffected (no warehouse tables for idempotent-only plans)

**Commit.** `feat(state+runtime): per-delta ledger grade; fold refuses already-reflected deltas (closes G-12)`

---

### Phase MP13: Cost-model defaults, override ladder, `smelt bakeoff` (M6b)

**Goal.** Where a cell admits multiple techniques: the default choice heuristics, the `defaults.prefer → cells[].prefer → cells[].technique` ladder (narrower wins; `technique:` bypasses the cost model), and `smelt bakeoff <model> [--cells ...] [--pin]` measuring admissible techniques over a representative window via the testkit's run-schedule driver (`maintenance_plan.md` §Surface, §Design "Offline cost measurement is first-class").

**Pre-conditions.** MP11, MP12 (two live techniques to choose between).

**TDD tests to write first.**
- `crates/smelt-logical/tests/maintenance_choice.rs::technique_pin_bypasses_cost_model_but_not_admission` — pinning an unadmitted technique is an error, not an override.
- `crates/smelt-logical/tests/maintenance_choice.rs::ladder_narrower_scope_wins`
- `crates/smelt-cli/tests/bakeoff.rs::bakeoff_reports_measured_cost_per_admissible_technique` (DuckDB; skips loudly without lib).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/`, `crates/smelt-cli/src/commands/`, testkit

**Docs touched.** *Timeless.*
- `docs/specs/cli.md`, docs-site reference page for `bakeoff`.

**Review checklist**:
- [ ] Choice changes only freshness at fixed S, never observable bits (spec §"Interchangeability and choice")

**Commit.** `feat(cli+logical): technique choice ladder + smelt bakeoff`

---

### Phase MP14: Landed-delta recording (P10 v1)

**Goal.** Per-source landed deltas recorded as partition intervals on the source's own axis in the state store (append-only landing / interval diff — the v1 mechanism), plus `granularity` **checking** for models (declared `timeseries.granularity` validated against the SQL's `date_trunc`-style grouping — the P3 classifier check that retires the tracer's edge-declared-grain shortcut).

**Pre-conditions.** MP9 (state store shape), MP3 (postures).

**TDD tests to write first.**
- `crates/smelt-state/tests/landed_deltas.rs::append_only_landing_records_interval_diff`
- `crates/smelt-db/tests/grain_check.rs::declared_granularity_contradicted_by_grouping_errors`
- `crates/smelt-state/tests/landed_deltas.rs::mutable_snapshot_delta_is_whole_table`

**Critical files (allowed to touch in this phase).**
- `crates/smelt-state/src/`, `crates/smelt-runtime/src/execute.rs` (record on landing), `crates/smelt-db` (grain check), `crates/smelt-logical` (check classifier)

**Docs touched.** *Timeless.*
- `docs/specs/sources.md` §Known Divergences — landed-delta recording no longer model-only; `maintenance_plan.md` — grain checking built.

**Review checklist**:
- [ ] Recording is derived-and-recorded, no new declared surface
- [ ] Unclocked source → whole-table delta (never silent nothing)

**Commit.** `feat(state): per-source landed-delta recording + declared-grain checking`

---

### Phase MP15: Forward propagation — `smelt run --since-upstream`

**Goal.** The graph layer's forward direction live (`maintenance_plan.md` §"The graph layer"): topological dirt reflection through per-edge clamps with outward grain ceiling, per-edge dirt keying the trigger cell, per-model dirt for consumers; `smelt run --since-upstream` prints the dirty set then runs exactly the propagated per-edge regions. Cyclic/self-referential/keyed-grain nodes refuse (`MaintenanceGraphUnsupportedNode`).

**Pre-conditions.** MP11, MP14.

**TDD tests to write first.**
- Promote `crates/smelt-logical/tests/maintenance_tracer_propagation.rs` scenarios (S1–S12 minus refused ones) from tracer-fed to production-path.
- `crates/smelt-cli/tests/since_upstream.rs::runs_exactly_the_propagated_regions` — two sources landing in one tick drive different cells over different regions of one model (P4); partitions outside the dirty set never scheduled (assert via reporter).
- `crates/smelt-cli/tests/since_upstream.rs::sufficiency_equals_full_refresh` — testkit equivalence after a propagated run.
- `crates/smelt-cli/tests/since_upstream.rs::self_referential_node_refuses_fail_loud`

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/propagate.rs` (production promotion), `crates/smelt-runtime`, `crates/smelt-cli/src/commands/run*.rs`

**Docs touched.** *Timeless.*
- `docs/specs/cli.md`, docs-site (the scheduling story), `maintenance_plan.md` Known Divergences narrowed.

**Review checklist**:
- [ ] Widen-never-narrow at every interval operation (reviewer spot-checks the ceilings)
- [ ] Dirty set printed before acting; opt-in flag

**Commit.** `feat(runtime+cli): forward propagation — smelt run --since-upstream`

---

### Phase MP16: Backward resolution — `smelt build --period --include-upstreams`

**Goal.** The graph layer's backward direction: reverse-topological clamp application yielding per-ancestor required slices + build order; print, and optionally execute, the bounded test/validation build. Adjointness (`forward(backward(P)) ⊇ P`) asserted.

**Pre-conditions.** MP15 (shared edge objects).

**TDD tests to write first.**
- `crates/smelt-cli/tests/include_upstreams.rs::resolved_slices_suffice` — staging exactly the resolved slices and building bottom-up equals a build over complete history for the target period (testkit).
- `crates/smelt-cli/tests/include_upstreams.rs::unclocked_ancestor_requires_whole_table`
- `crates/smelt-logical/tests/maintenance_propagation_adjoint.rs::forward_backward_containment`

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/propagate.rs`, `crates/smelt-cli/src/commands/build*.rs`, `crates/smelt-runtime`

**Docs touched.** *Timeless.*
- `docs/specs/cli.md`, docs-site (test/validation builds), `maintenance_plan.md` Known Divergences narrowed.

**Review checklist**:
- [ ] One edge object, two directions (no second clamp implementation)
- [ ] Build order ancestor-first, target last

**Commit.** `feat(cli): backward resolution — smelt build --period --include-upstreams`

---

### Phase MP17: Coverage-matrix conformance sweep

**Goal.** The plan-conformance fixture set inhabits **every inhabited cell of the research coverage
matrix** (`docs/research/20260705-refresh-as-maintenance-plan/07-example-catalogue.md` §"Coverage
matrix": 22 construct rows × 7 source-property columns): each EX machine header is translated into a
testkit shape (`model_shapes.rs`'s catalogue, per `08-code-placement.md` §2.7), and each shape asserts
one of exactly two honest outcomes — the derived plan matches the catalogue's expected cell/technique
and (where the technique is live) the emitted maintenance ≡ full refresh over adversarial schedules,
**or** the shape refuses with the named diagnostic (no third, silent outcome). This lifts the 19
lift-ready probe cells (07 §"Candidate probe cells") and doubles the named single-example thin spots
(the composite-key column, LEFT JOIN, fan-out, LAG/LEAD, ROW_NUMBER dedup, self-referential,
GROUP-BY-coarser, correlated first-value).

**Pre-conditions.** MP8 (testkit is the home), MP11+MP12 (both technique families live so HOLDS cells
can assert execution, not just description). Cells whose catalogue verdict is UNSUPPORTED-TODAY assert
the refusal diagnostic and are annotated with the catalogue id so later work flips them deliberately.

**TDD tests to write first.**
- `smelt-maintenance-testkit`: one shape per unclaimed matrix cell, named by catalogue id (`ex_03_late_passthrough`, `ex_14_cdc_retraction_sum`, …), each with its expected-plan or expected-refusal assertion from the catalogue's machine header.
- `crates/smelt-logical/tests/maintenance_plan_conformance.rs::coverage_matrix_is_inhabited` — a meta-test: every inhabited `(construct × source-property)` cell of the matrix (encoded as a table in the testkit) has ≥ 1 registered shape; adding a matrix row without a shape fails.
- The `INTERSECT`/`EXCEPT` set-operation shapes assert today's honest verdict (refusal — set-op distribution classifies `UNION ALL` only, `model_properties.md` §Known Divergences), pinning the classification gap.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-maintenance-testkit/`, `crates/smelt-logical/tests/`, `crates/smelt-cli/tests/`

**Docs touched.** *Timeless.*
- `docs/specs/maintenance_plan.md` §References → Tests: name the coverage-matrix meta-test as the standing inventory gate; §Known Divergences: record any UNSUPPORTED-TODAY cell worth a future flip in behavioural terms.

**Review checklist**:
- [ ] No cell asserted optimistically: HOLDS requires the equivalence leg, not just plan description
- [ ] Refusal assertions name the diagnostic code, not just "errors"
- [ ] The meta-test makes the matrix additive-only (a new construct row must bring a shape)

**Commit.** `test(maintenance): coverage-matrix conformance sweep — every construct×source cell asserted or refused by name`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

- `cargo test --quiet 2>&1 | tail -40` — full workspace green (with `DUCKDB_LIB_DIR` exported).
- `cargo test -p smelt-runtime --test execute_parity`; `cargo test -p smelt-cli --test example_diagnostics`; `cargo test -p smelt-lsp --test example_workspaces` — all green.
- The standing testkit equivalence gate green over the shape catalogue; the coverage-matrix
  meta-test (`coverage_matrix_is_inhabited`) green — every inhabited construct×source cell of
  `07-example-catalogue.md`'s matrix has a shape asserting derived-plan-or-named-refusal.
- `rg -n 'refresh: (batched|keyed|cumulative|versioned)' examples/ docs-site/docs/` → zero.
- `/smelt:validate maintenance_plan`, `/smelt:validate models`, `/smelt:validate sources` — Known Divergences reduced to exactly the deliberately-deferred list in §Scope.
- `smelt explain`, `smelt run --since-upstream`, `smelt build --period --include-upstreams`, `smelt bakeoff` all exercised against `examples/timeseries`.

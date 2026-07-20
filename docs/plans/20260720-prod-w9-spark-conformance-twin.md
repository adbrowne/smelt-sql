# Plan: W9 — Spark twin of the generative maintenance-conformance harness

**Date**: 2026-07-20
**Spec**: [`docs/specs/multi_backend.md`](../specs/multi_backend.md) (parity contract; equivalence invariant defined in [`docs/specs/incremental_models.md`](../specs/incremental_models.md) §"The equivalence invariant")
**Spec diff**: Phase 1 of this plan writes it (generative oracle becomes dual-backend in §"Parity contract"); later phases implement against it
**Tracking PR / branch**: `worktree-production` (PR #165)
**Docs**: code+docs
**Master**: [`docs/plans/20260719-production-readiness.md`](20260719-production-readiness.md) — sub-plan **W9**. Closes the W4 Phase 5 gap table ([`20260719-prod-w4-spark.md`](20260719-prod-w4-spark.md)); promotion criterion for the D1 supported-vs-beta label.

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/multi_backend.md` and `docs/specs/incremental_models.md` §"The equivalence invariant" — they are the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `worktree-production`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**CRITICAL PREREQUISITE — live Spark server.** Phases 3–6 need a live Delta-enabled Spark Connect server (`bash scripts/spark-up.sh` + `source scripts/spark-env.sh`). If a phase below is marked **[needs Spark]** and `SPARK_CONNECT_URL` is unset, **emit `<<PHASE_BLOCKED>>` with the reason** — never let Spark-targeted assertions skip green and call the phase done. (Default `cargo test` skip-when-unset semantics per the spec's "Default `cargo test` is backend-agnostic" constraint are fine for the committed tests; the *phase completion claim* requires a live-server green run.)

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.
- A Spark leg surfaces a genuine equivalence divergence (a real incremental-correctness bug on Spark): fix it red-green if it is a smelt bug; if it looks like engine behaviour, stop and present the evidence — do not ledger an equivalence failure like a type divergence.

**Conventions every phase:** red-green TDD; real-fixture tests; verification gate is `bash .claude/scripts/verify-phase.sh`; atomic per-phase commits with the phase's `Commit.` line verbatim; never `--no-verify`; don't widen scope; honor `CLAUDE.md` invariants (maintenance-plan purity: the harness *verifies* emitted statements, never authors maintenance SQL); Timeless-oracle rule — phase vocabulary stays in this file.

---

## Context

W4 left one gap keeping Spark below DuckDB's verification bar: the equivalence invariant (`incremental_models.md` §"The equivalence invariant") is enforced generatively only on DuckDB, via the `maintenance_conformance` harness; Spark has fixed-recipe smoke parity only (full gap table: `20260719-prod-w4-spark.md` §"Phase 5 conformance-gap table"). This plan retargets the existing harness at Spark as a **dual-execution mode** — single owner of recipes, schedules, and oracle logic; the backend under test becomes a parameter — rather than a duplicated Spark harness. The exploration finding this plan builds on: execution already flows through a `BackendFactory` seam (`crates/smelt-cli/tests/link_c_harness.rs`), and the multiset oracle is portable SQL (`EXCEPT ALL`), but four testkit surfaces are typed on concrete `duckdb::Connection` — staging DDL (`smelt-maintenance-testkit/src/render.rs::stage`), per-row source inserts (`gate.rs::insert_row`), S-materialization (`s_tracker.rs::materialize_s`), and the comparison (`oracle.rs::multiset_equal`) — and `smelt-maintenance-testkit/Cargo.toml` hard-depends on DuckDB.

## Scope

### In scope (spec coverage)
- `multi_backend.md` §"Parity contract": the generative equivalence oracle runs on both backends; CI tiering for the Spark leg.
- `multi_backend.md` §Known Divergences: resolve "The generative maintenance-conformance oracle has no Spark twin"; correct the ledger-count statement (24 → 22).
- The recipe-pool, admission-rate, DAG, probe, and pinned legs from the W4 gap table — each either running on Spark or explicitly dispositioned as harness-internal (N/A).

### Explicitly deferred
- The D1 label change itself (Andrew's call; this plan produces the promotion evidence).
- Spark-over-**Parquet** profile in the generative sweep — Delta is the parity baseline (spec §Design); Parquet stays smoke-covered.
- Per-PR execution of the Spark conformance leg — it is nightly/label/paths-gated like `spark-parity`; runtime cost of the generative pool over Spark Connect makes unconditional per-PR runs impractical.
- Cross-engine (Spark→DuckDB exchange) recipes in the generative pool — exchange stays covered by `cross_engine_parity.rs`.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | (this commit) | 2026-07-20 |
| 2     | done     | (this commit) | 2026-07-20 |
| 3     | done     | (this commit) | 2026-07-20 |
| 4     | done     | (this commit) | 2026-07-20 |
| 5     | done     | (this commit) | 2026-07-20 |
| 6     | pending  |        |      |

## Phase detail

### Phase 1: Spec diff — dual-backend generative oracle

**Goal.** Land the normative text: §"Parity contract" states the equivalence invariant is verified generatively on every supported backend (DuckDB per-PR; Spark in the gated tier), with the harness as single owner of recipes and oracle. Known Divergences: rewrite the "no Spark twin" row to describe the rollout gap in behavioural terms (which legs remain DuckDB-only until later phases land), and correct "(24 entries)" → 22 in the ledger row.

**Pre-conditions.** None (docs-only).

**TDD tests to write first.** None (spec phase). Gate: `/smelt:validate multi_backend` after the plan completes reports the new text implemented, not drifted.

**Implementation shape.** Edit `docs/specs/multi_backend.md` §"Parity contract" (supported-surface statement gains the generative leg per backend + which CI tier runs it), §Known Divergences (rewrite/correct the two rows). Cite `incremental_models.md` §"The equivalence invariant" — do not restate it.

**Critical files (allowed to touch in this phase).**
- `docs/specs/multi_backend.md`

**Docs touched.** Spec only; docs-site lands in Phase 6 once behaviour exists.

**Review checklist** (material findings only):
- [ ] Parity-contract text names the enforcing test suites (falsifiable)
- [ ] Known Divergences rows are behavioural, no phase vocabulary
- [ ] Ledger count corrected to 22
- [ ] Equivalence invariant cited, not restated

**Commit.** `spec(multi-backend): generative equivalence oracle is dual-backend; correct ledger count`

### Phase 2: Testkit backend seam (pure refactor, DuckDB behaviour unchanged)

**Goal.** `smelt-maintenance-testkit`'s four DuckDB-typed surfaces are re-expressed against the `smelt_backend::Backend` trait, and a `ConformanceTarget` parameter (mirroring `common::TargetKind`) selects the `smelt.yml` target block and backend factory. All existing DuckDB gates pass unchanged; no Spark code yet.

**Pre-conditions.** None.

**TDD tests to write first.**
- Existing standing gates are the red-green net: `cargo test -p smelt-cli --no-default-features --features duckdb --test maintenance_conformance` must be green before and after, with **zero assertion or schedule changes** — the refactor is behaviour-preserving by construction.
- `crates/smelt-maintenance-testkit/src/oracle.rs::multiset_equal_routes_through_backend_trait` — unit test proving `multiset_equal` executes via `Backend::execute_sql` (DuckDB impl), not a raw `duckdb::Connection`, and still detects a seeded divergence (mirror of `harness_self_check.rs::oracle_flags_a_seeded_divergence` at the trait level).

**Implementation shape.** In `smelt-maintenance-testkit`: introduce `ConformanceTarget { DuckDb, SparkDelta }`; change `render::render_smelt_yml` to emit the target block per `ConformanceTarget` (Spark block shape copied from `crates/smelt-cli/tests/common/mod.rs::targets_yaml`); route `render::stage`'s source DDL, `insert_row`'s appends, `s_tracker::materialize_s`, and `oracle::multiset_equal` through `&dyn Backend` (`execute_sql` for DDL/DML/`EXCEPT ALL`, `load_table` where Arrow batches fit better than row DML). Keep DuckDB as the only constructed target this phase. Add an optional `spark` feature to `smelt-maintenance-testkit/Cargo.toml` (dep on `smelt-backend-spark`, off by default) but wire nothing to it yet. `LinkCProject::run` gains factory selection by `ConformanceTarget` (DuckDB arm only, Spark arm `unimplemented!` until Phase 3 — never reachable without the feature).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-maintenance-testkit/src/{render,s_tracker,oracle,recipe}.rs`, `Cargo.toml`
- `crates/smelt-cli/tests/link_c_harness.rs`, `crates/smelt-cli/tests/maintenance_conformance/gate.rs` (call-site signature updates only)

**Docs touched.** None (internal refactor; spec text landed in Phase 1).

**Review checklist:**
- [ ] Standing DuckDB gates green with unchanged assertions/schedules
- [ ] No maintenance SQL authored by the harness (purity rule intact — harness only seeds sources and compares)
- [ ] `EXCEPT ALL` oracle SQL unchanged, only its execution channel moved
- [ ] Default build (`--features duckdb`) has no Spark dependency

**Commit.** `refactor(testkit): backend-trait seam for the maintenance-conformance harness (DuckDB behaviour unchanged)`

### Phase 3: First Spark leg — append-only partition pool **[needs Spark]**

**Goal.** `append_only_partition_pool_upholds_equivalence` runs green against a live Spark Connect server via a new `maintenance_conformance_spark` test binary, with a reduced deterministic sample, per-schema isolation, and skip-when-unset semantics matching the spec constraint.

**Pre-conditions.** Phase 2 seam. Live Spark server (else `<<PHASE_BLOCKED>>`).

**TDD tests to write first.**
- `crates/smelt-cli/tests/maintenance_conformance_spark/main.rs::append_only_partition_pool_upholds_equivalence_on_spark` — same recipe pool and schedule driver as the DuckDB leg, `ConformanceTarget::SparkDelta`, case count from `SMELT_CONFORMANCE_SPARK_CASES` (default 4), dedicated schema (`smelt_conf_gen`), `drop`-before-seed idempotency per the parity-test convention. Red first: written against the Phase 2 seam with the Spark factory arm still `unimplemented!`.
- Spark twin of the harness self-check: `oracle_flags_a_seeded_divergence_on_spark` — a deliberately wrong maintained state must fail the oracle on Spark too (proves the comparison has teeth on the new backend before any equivalence claim).

**Implementation shape.** New test binary `crates/smelt-cli/tests/maintenance_conformance_spark/` gated `#![cfg(feature = "spark")]` (the existing `maintenance_conformance` binary and its `#![cfg(feature = "duckdb")]` gate stay untouched — the standing per-PR DuckDB gate must not change). Implement the Spark arms: backend factory (`SparkBackend::new` per `common/mod.rs` conventions), source seeding via `load_table`/`execute_sql` appends (never a host-path read — spec §"Loading data into a backend"), `materialize_s` into a per-run temp/scratch table Spark-side, `multiset_equal` via `Backend::execute_sql`. Source-table between-step appends use Delta `INSERT INTO`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/tests/maintenance_conformance_spark/**` (new)
- `crates/smelt-maintenance-testkit/src/**` — Spark arms behind the `spark` feature
- `crates/smelt-cli/Cargo.toml` — feature plumbing for the new test target

**Docs touched.** None yet (rollout still partial; spec Known Divergences row from Phase 1 already describes the staged state).

**Review checklist:**
- [ ] Self-check leg proves the Spark oracle detects a seeded divergence
- [ ] `SPARK_CONNECT_URL` unset ⇒ skip (default suite green); live run recorded in the commit message
- [ ] No host-filesystem load path
- [ ] DuckDB standing gate untouched

**Commit.** `test(spark): generative append-only conformance leg green on live Spark (dual-execution harness)`

### Phase 4: Recipe-pool breadth — keyed, mutable, redelivery, interleave, boundary, schema-evolution **[needs Spark]**

**Goal.** The remaining recipe-pool legs from the gap table run on Spark: `keyed_pool_upholds_end_state_equivalence`, `mutable_pool_settles_to_full_refresh`, `redelivery_of_processed_window_is_idempotent`, `full_refresh_interleave_resets_state_correctly`, `boundary_rows_within_reach_are_reflected`, `column_add_between_runs_recovers_equivalence`, plus the admission-rate statistics legs (`admission_rate_stays_above_floor`, keyed/composed/delta-restriction variants as the pool permits).

**Pre-conditions.** Phase 3 green. Live Spark server (else `<<PHASE_BLOCKED>>`).

**TDD tests to write first.** One Spark twin per leg above in `maintenance_conformance_spark/`, same recipes/schedules as the DuckDB originals, reduced case counts via `SMELT_CONFORMANCE_SPARK_*_CASES` knobs (defaults sized so the full binary stays under ~20 minutes against the containerized server — measure and record). Red first where the seam lacks an arm (e.g. keyed staging, `RewriteModel` on-disk rewrite driving Spark schema evolution).

**Implementation shape.** Mostly wiring: each leg reuses its existing pool strategy and `drive_and_assert`. Expected genuinely-new ground: schema-evolution recipes on Delta (column add via the real `execute_project` migration path), and admission-rate counting over the Spark-driven pool. Any leg that cannot run on Spark for a *structural* reason must be recorded in "Deferred during implementation" with the reason — no silent omission.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/tests/maintenance_conformance_spark/**`
- `crates/smelt-maintenance-testkit/src/**` (Spark arms only)

**Docs touched.** None yet.

**Review checklist:**
- [ ] Every gap-table recipe-pool row now Spark-covered or explicitly dispositioned in this plan
- [ ] Case counts + wall-clock recorded in the commit message
- [ ] No `#[ignore]`, no skip-green on a live server

**Commit.** `test(spark): recipe-pool conformance legs (keyed/mutable/redelivery/interleave/boundary/schema-evolution) green on live Spark`

### Phase 5: Structural legs + dispositions — composed pools, DAGs, probes, pinned, change-feed **[needs Spark]**

**Goal.** The structural legs run on Spark or receive a recorded disposition: `composed_keyed_pool_upholds_equivalence` (+ its admission-rate twin), `delta_restriction_admission_rate_stays_above_floor`, the `dags.rs` propagation legs, the `probes.rs` legs, `pinned.rs` catalogue coverage, and the change-feed legs (`change_feed_source_admits_recompute_only`, `feed_declared_source_upholds_equivalence_via_recompute` — need a Spark change-feed source fixture). `retained_departed_keys_adjusts_the_oracle` and `oracle_flags_a_seeded_divergence` are harness-internal (oracle mechanics, not backend conformance) — disposition **N/A**, asserted by the Phase 3 Spark self-check instead.

**Pre-conditions.** Phase 4 green. Live Spark server (else `<<PHASE_BLOCKED>>`).

**TDD tests to write first.** Spark twins per leg in `maintenance_conformance_spark/`, red first. The change-feed fixture is the one genuinely new fixture: a Delta source staged with feed-declared metadata driving the recompute-only admission path.

**Implementation shape.** As Phase 4. DAG legs exercise multi-model staged projects — verify per-schema isolation still holds with several models per project. Pinned catalogue: run the pinned recipes against Spark; a pinned recipe that is DuckDB-specific by content gets a disposition line, not a port.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/tests/maintenance_conformance_spark/**`
- `crates/smelt-maintenance-testkit/src/**` (Spark arms + change-feed fixture)

**Docs touched.** None yet.

**Review checklist:**
- [ ] W4 gap table fully dispositioned: every row → Spark-covered | N/A-with-reason | deferred-with-reason
- [ ] Change-feed fixture loads via backend client API only
- [ ] Wall-clock for the full Spark binary recorded

**Commit.** `test(spark): structural conformance legs (composed/DAG/probes/pinned/change-feed) green on live Spark; gap table dispositioned`

### Phase 6: CI leg, docs, gap-table closure

**Goal.** A `maintenance-conformance-spark` job in `.github/workflows/compat.yml` runs the Spark binary in the gated tier; docs-site and spec reflect the dual-backend oracle; the W4 gap table and master plan record closure.

**Pre-conditions.** Phases 1–5 done.

**TDD tests to write first.** CI empirical red-green (per the W4 Phase 7 precedent — *actually run it*): the job must appear and pass on the tracking PR when triggered via the `run-docker-tests` label or a Spark-relevant path change; record the `gh pr checks` outcome in the commit message or PR comment. Docs gate: `cargo test -p smelt-cli --test example_diagnostics` stays green; docs-site build succeeds.

**Implementation shape.** Clone the `spark-parity` job pattern (server provisioning, Ivy cache, env exports, `if: ${{ !cancelled() && (schedule || label || needs.changes.outputs.spark == 'true') }}`, `always()` teardown) into `maintenance-conformance-spark`, running `cargo test -p smelt-cli --features smelt-cli/spark --test maintenance_conformance_spark` with the reduced default case counts; keep the DuckDB soak job untouched. Update `docs-site/docs/guide/targets.md` limitations table (rows flip to covered). Spec: move the Phase 1 Known-Divergences rollout row to resolved. Append closure notes to the W4 gap table ("Disposition" paragraph) and the master plan's D1 evidence brief (one line: generative oracle now dual-backend — promotion criterion met).

**Critical files (allowed to touch in this phase).**
- `.github/workflows/compat.yml`
- `docs-site/docs/guide/targets.md`, `docs/specs/multi_backend.md`
- `docs/plans/20260719-prod-w4-spark.md`, `docs/plans/20260719-production-readiness.md`

**Docs touched.**
- `docs-site/docs/guide/targets.md` — limitations table (timeless: describe coverage, not phases)
- `docs/specs/multi_backend.md` — Known Divergences closure

**Review checklist:**
- [ ] CI job empirically observed running (not just `if:` inspection) — outcome recorded
- [ ] `!cancelled()` guard present on the new job
- [ ] Limitations table claims no more than the legs prove
- [ ] No phase vocabulary in spec/docs-site edits

**Commit.** `ci(spark): gated maintenance-conformance-spark job; docs + gap-table closure for the dual-backend oracle`

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- **2026-07-20, Phase 4:** `keyed_pool_upholds_end_state_equivalence_on_spark` covers only `KeyedCombiner::Idempotent`, not `KeyedCombiner::Additive` (the DuckDB leg samples both via `arb_keyed_combiner()`). Reason: `KeyedCombiner::Additive` grades `Grade::Additive`, whose never-fold-twice reconciliation ledger (`maintenance_driver.rs`'s `Grade::Additive` arm, MP12) has only a DuckDB dialect (`smelt_state::ddl_duckdb`) implemented today; the driver already fails loud (`BackendError::unsupported("additive-fold windowed-keyed maintenance ledger (never-fold-twice)")`) for any non-DuckDB backend rather than silently mishandling it — confirmed live against Spark. This is a pre-existing backend gap, not a W9-introduced one; a Spark ledger dialect is out of this plan's scope (it would be its own plan). The Idempotent-combiner family (`Grade::Idempotent`, watermark-only ledger, no dialect restriction) is unaffected and is what the Spark leg exercises.
- **2026-07-20, Phase 4 (fixed, not deferred):** while porting the keyed leg, found and fixed a real Spark bug: `smelt_logical::maintenance::emit::emit_create_table_as` (the first-run bootstrap `CREATE TABLE … AS` for any merge-based maintenance cell) ignored its `MaintenanceDialect` parameter and never emitted a format clause, so the bootstrapped table on Spark was a plain (non-Delta) managed table — every subsequent `MERGE INTO` against it then failed with `UnsupportedOperationException: MERGE INTO TABLE is not supported temporarily`. Fixed to emit `USING DELTA` for `MaintenanceDialect::Spark` (DuckDB output unchanged, `MaintenanceDialect::DuckDb` emits no format clause as before). Covered by a new `create_table_as_spark_dialect_specifies_delta_format` test in `crates/smelt-logical/tests/emit_statements.rs`; confirmed the fix live against Spark (the keyed leg's `Idempotent`-combiner cases, which bootstrap via this path and then `MERGE`, went from failing to green).
- **2026-07-20, Phase 5 (fixed, not deferred):** while porting the composed pool's route-3 (recurrence-bounded) leg, found and fixed a second real Spark dialect bug: `smelt_logical::maintenance::emit::emit_recurrence_bound_probe` (the route-3 checked-merge out-of-slice match probe) hardcoded DuckDB's unsized `CAST(... AS VARCHAR)` and `STRING_AGG`, neither of which Spark SQL accepts (`DATATYPE_MISSING_SIZE` — Spark's `VARCHAR` requires a length; Spark has no `STRING_AGG` builtin at all) — every route-3 step failed with a `ParseException` before the merge itself ever ran. Fixed by adding a `dialect: MaintenanceDialect` parameter (threaded through `WindowedKeyedRule::recurrence_probe_sql` and its `CumulativeClassification` impl, supplied by the driver from `backend.dialect()` the same way `emit_create_table_as`'s call site already does): `MaintenanceDialect::Spark` emits `CAST(... AS STRING)` and `CONCAT_WS(', ', COLLECT_LIST(...))`; `MaintenanceDialect::DuckDb` output is unchanged. Covered by a new `recurrence_bound_probe_spark_dialect_uses_string_and_concat_ws` test in `crates/smelt-logical/tests/emit_statements.rs`; confirmed the fix live against Spark (route 3's per-window checked-merge went from a parser error to green).
- **2026-07-20, Phase 5:** `composed_keyed_pool_upholds_equivalence_on_spark` drives only `ComposedRoute::RecurrenceBounded` (route 3) — routes 1 (`KeyEmbedded`, via `execute_project`) and 2 (`KeyDetermined`, direct-driver) are excluded from the equivalence-drive strategy. Reason: both routes' body is `SELECT id, ... , SUM(val) AS total ...` — a `SUM` cross-partition combiner, which grades `Grade::Additive` (same never-fold-twice reconciliation-ledger gap as the Phase 4 entry above: no Spark dialect for MP12's ledger DDL yet). Confirmed live: route 1's `execute_project` drive failed with the driver's own fail-loud `BackendError::unsupported("additive-fold windowed-keyed maintenance ledger (never-fold-twice)")` on its very first fold. Route 3's body (`MAX(d) AS last_seen`) grades `Grade::Idempotent` and is unaffected — it is what the Spark leg exercises. Admission (`composed_keyed_admission_rate_stays_above_floor_on_spark`) is pure classification, never execution, so it still samples all three routes at the DuckDB leg's 90% floor.
- **2026-07-20, Phase 5:** `delta_restriction_admission_rate_stays_above_floor` is dispositioned **N/A** rather than ported: the test (`enrichment_edge_closed(recipe.join_kind)`) is a pure classification-rate check over `EnrichmentJoinKind` with zero backend I/O anywhere in its body — no staging, no connection, no execute — so its outcome cannot differ by backend. The DuckDB run already fully covers it; a Spark-suffixed duplicate would assert the identical pure function and add no coverage.
- **2026-07-20, Phase 5:** `feed_declared_source_upholds_equivalence_via_recompute` is **not ported** this pass (`change_feed_source_admits_recompute_only` is — see `change_feed_source_admits_recompute_only_on_spark`, classify-only). Reason: unlike the admission leg, this test drives genuine incremental runs interleaved with dimension mutations and checks the result against a change-log replay oracle built from `smelt_maintenance_testkit::feed::apply_feed_step`/`replay_feed`, both hardcoded to a raw `duckdb::Connection` (not the `Backend` trait). Porting that oracle-replay machinery to a Backend-routed form is real, new work — the plan's own "Implementation shape" for this phase names the change-feed fixture as "the one genuinely new fixture"; this pass ships that fixture's classify-only slice (a new `stage_feed_keyed_for_target`/`create_feed_tables_via_backend` Spark arm in `smelt-maintenance-testkit/src/feed.rs`) and defers the execution-driven leg's replay-oracle port as follow-up.
- **2026-07-20, Phase 5:** `probes.rs`'s two legs (`window_order_permutations_converge`, `probe_skips_are_counted_never_silent`) plus the two `#[tokio::test]` probes underneath (`compiled_sql_filter_matches_derived_clamp`, `rows_outside_write_window_are_byte_unchanged`, `technique_pins_agree_at_fixed_s`) are **not ported** this pass. Reason: every one of them drives through `smelt_maintenance_testkit::probes::CaseContext`, whose staging and read-back (`read_full_output_as_text`, the reachability-report bookkeeping) are hardcoded to a raw `duckdb::Connection`, not the `Backend` trait — the same class of rewrite `render.rs`/`dag.rs`/`feed.rs` already got in Phases 2–5 has not yet reached `probes.rs`. `technique_pins_agree_at_fixed_s` additionally compares the fold family against `KeyedCombiner::Additive`, which hits the same `Grade::Additive` reconciliation-ledger gap documented above regardless of the `CaseContext` rewrite. Tracked as follow-up (a `CaseContext`-generalization pass mirroring this plan's Phase 2 seam, scoped to `smelt-maintenance-testkit/src/probes.rs`).
- **2026-07-20, Phase 5:** `pinned.rs`'s `hazard::keyed_merge_reprocessed_window` case is **not reproduced** on Spark (`pinned_spark.rs`'s `hazard_schedules_are_pinned_on_spark` ports every other hazard case). Reason: it specifically pins `KeyedCombiner::Additive`'s never-fold-twice REFUSAL — on Spark, the very first fold in its schedule fails loud with the ledger's own `BackendError::unsupported` (the Phase 4 gap above) rather than the hazard's own `KeyedReprocessedWindow` refusal, so there is no equivalent Spark behaviour to pin.
- **2026-07-20, Phase 5:** the whole `maintenance_conformance_spark` binary must be run with `-- --test-threads=1`. Every recipe's `model_name` is deterministic (by construct/route, not per-test-run-unique), and the Spark/Delta warehouse (`SPARK_CONFORMANCE_SCHEMA`) persists across the whole binary rather than getting a fresh temp DuckDB file per case; two tests racing on the same physical Delta table (or even two sequential tests without a clean interval — observed as a `SIGSEGV` in the underlying PySpark/py4j bridge during this phase's development) is a real, not-yet-removed hazard. Documented in `main.rs`'s module doc comment; a proper fix (per-test schema/table namespacing) is tracked as follow-up, not blocking this plan's phases — Phase 6's CI job must invoke the binary with this flag.

## Verification

How to confirm the spec is satisfied at the end:
- Live server: `cargo test -p smelt-cli --features smelt-cli/spark --test maintenance_conformance_spark --quiet` green with `SPARK_CONNECT_URL` set, zero skips.
- Standing DuckDB gate unchanged and green: `cargo test -p smelt-cli --no-default-features --features duckdb --test maintenance_conformance`.
- `maintenance-conformance-spark` observed green on the tracking PR (`gh pr checks`).
- Every row of the W4 Phase 5 gap table dispositioned (covered / N/A / deferred-with-reason).
- `bash .claude/scripts/verify-phase.sh`
- `/smelt:validate multi_backend` reports zero drift on the sections this plan owns.

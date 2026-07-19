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
| 1     | pending  |        |      |
| 2     | pending  |        |      |
| 3     | pending  |        |      |
| 4     | pending  |        |      |
| 5     | pending  |        |      |
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

## Verification

How to confirm the spec is satisfied at the end:
- Live server: `cargo test -p smelt-cli --features smelt-cli/spark --test maintenance_conformance_spark --quiet` green with `SPARK_CONNECT_URL` set, zero skips.
- Standing DuckDB gate unchanged and green: `cargo test -p smelt-cli --no-default-features --features duckdb --test maintenance_conformance`.
- `maintenance-conformance-spark` observed green on the tracking PR (`gh pr checks`).
- Every row of the W4 Phase 5 gap table dispositioned (covered / N/A / deferred-with-reason).
- `bash .claude/scripts/verify-phase.sh`
- `/smelt:validate multi_backend` reports zero drift on the sections this plan owns.

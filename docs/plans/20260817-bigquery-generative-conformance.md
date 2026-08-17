# Plan: BigQuery generative maintenance-conformance leg

**Date**: 2026-08-17
**Spec**: [`docs/specs/multi_backend.md`](../specs/multi_backend.md)
**Spec diff**: none yet — the spec edits ride with Phases 2, 5 and 6 of this plan (§"Generative equivalence coverage", §"CI tiering", §"Parity contract" supported-surface statement, and the BigQuery entry in §Known Divergences)
**Tracking PR / branch**: `bigquery-backend-research`
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec at `docs/specs/multi_backend.md` — it is the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `bigquery-backend-research`, in the worktree `/home/andrew/smelt-sql/.claude/worktrees/bigquery`. If not, ask the user before continuing.
3. Read `docs/handoffs/2026-08-16-bigquery-backend.md` §"Working constraints that cost time to rediscover" before running anything that touches GCP. In particular: `gcloud` and `bq` are denied to agents, `bq` is broken on this host, and every BigQuery session needs a human to run `bash scripts/bigquery-auth.sh` first.
4. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan. One is already known: `cargo test -p smelt-logical --test contract_lattice_spec` fails at `HEAD` for a missing spec heading and predates this branch — do not fix it here.
- A phase needs a live credential. Phases 1, 5 and 6 cannot complete without a human-minted token; stop and ask rather than inventing a fallback.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/`.
- Red-green TDD: failing test before any implementation.
- Verification gate is `bash .claude/scripts/verify-phase.sh` (one call: fmt + clippy + tests + example_diagnostics, failures-only output) — do not run the four commands separately.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md`.
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to `docs/specs/multi_backend.md` and `docs-site/docs/...` describe the feature as if it has always existed.

---

## Context

`docs/specs/multi_backend.md` §"Generative equivalence coverage" requires the equivalence invariant to be verified generatively **on every supported backend**, through a single harness in which "the backend under test is a parameter, not a duplicated implementation". BigQuery is a supported backend with no generative leg, and the harness's second leg is in practice a ~2,600-line re-derivation rather than a parametrization — so this plan closes the coverage gap and makes the spec's parametrization claim true at the same time, since adding a third copy would make it false.

## Scope

### In scope (spec coverage)
- §"Generative equivalence coverage" — a BigQuery leg over the same recipe pool, schedules, and S-restricted oracle as the DuckDB and Spark legs.
- §"Generative equivalence coverage" — the shared test families extracted into one target-parametrized owner; Spark and BigQuery both instantiate it.
- §"CI tiering" — where the BigQuery leg runs, and why it is not per-PR.
- §"Parity contract" supported-surface statement — BigQuery's incremental coverage becomes generative, not fixed-recipe-only.
- §Known Divergences — retire the fixed-recipe-only BigQuery entry.

### Explicitly deferred
- **Folding the DuckDB leg into the parametrized families.** It is the reference leg, runs per-PR on every `cargo test`, and owns families the other two do not (`contract_points`, `fact_violations`, `probes`, `registry`, `repair`). Restructuring it risks per-PR CI for no BigQuery gain.
- **The `Additive`-combiner keyed/composed folds and the probe/feed execution-driven legs.** Already DuckDB-only for reasons independent of any backend rollout (§Known Divergences); BigQuery inherits the same exclusion.
- **A CI job.** BigQuery verification is local-gated with no CI tier by standing decision (`docs/research/20260816-bigquery-backend.md`); this plan does not change that.
- **`supports_pipe_syntax` live coverage** and **cross-engine BigQuery pairs** — separate work items in the handoff's next-steps list.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 2538c9af | 2026-08-17 |
| 2     | done     | 49d5375c | 2026-08-17 |
| 3     | done     | cc3eda18 | 2026-08-17 |
| 4     | done     | f330210d | 2026-08-17 |
| 4b    | done     |        | 2026-08-17 |
| 5     | pending  |        |      |
| 6     | pending  |        |      |

---

### Phase 1: Measure the warehouse's modification and dataset limits

**Goal.** Establish, by running statements against the live warehouse, the three facts that size every later phase: at what spacing consecutive modifications to one table are refused, whether creating/dropping one dataset per case hits a dataset-level rate limit, and whether a realistic sweep can reach the daily per-table cap.

**Pre-conditions.** A human has run `bash scripts/bigquery-auth.sh` in this session.

**This is a measurement phase.** Its deliverable is facts, not a feature. The probe script carries its own assertions; the recorded numbers are the phase's output and Phase 3's input. This follows the standing rule that capability values come from the warehouse and never from documentation (`docs/research/20260816-bigquery-backend.md` §decision 4) — the same way `scripts/bigquery-probe-merge.sh` converted a red parity leg into a fact.

**TDD tests to write first.**
- `scripts/bigquery-probe-quota.sh` — asserts a *specific* refusal, not merely "something failed": N consecutive `CREATE OR REPLACE TABLE` against one table at spacing S must either all succeed or fail with the `quota for table update operations` shape, and the script fails loud on any other error. Sweeps S so the binding spacing is a measured number rather than a guess. **Do not let the loop's own round-trip latency set the spacing** — that mistake already produced one wrong reading of this limit (handoff §"Two findings that change downstream sizing").
- Same script — creates and drops K datasets back-to-back and asserts either clean success or a recognised dataset-rate refusal.

**Implementation shape.** A `scripts/bigquery-probe-quota.sh` in the shape of the existing `bigquery-probe*.sh` family: reads the minted token off disk, talks to the REST API with `curl`, prints a findings block. No Rust.

**Critical files (allowed to touch in this phase).**
- `scripts/bigquery-probe-quota.sh` — new.
- `docs/research/20260816-bigquery-backend.md` — record the measured numbers under §"Measured against the live warehouse".

**Docs touched.**
- `docs/research/20260816-bigquery-backend.md` — the measured limits, as findings.

**Review checklist** (material findings only):
- [ ] The probe distinguishes the quota refusal shape from every other error, and fails loud on the latter
- [ ] Spacing is controlled by the probe, not by its own request latency
- [ ] Measured numbers are recorded with the statement that produced them
- [ ] No `bq` or `gcloud` invocation (both unusable/denied — handoff §"Working constraints")

**Commit.** `probe: measure BigQuery's per-table modification and dataset-creation limits`

---

### Phase 2: A BigQuery arm on the ConformanceTarget seam

**Goal.** `ConformanceTarget` can name a BigQuery dataset, and every staging path renders and stages a BigQuery-targeted project. No test families yet.

**Pre-conditions.** None beyond `HEAD`.

**TDD tests to write first.**
- `crates/smelt-maintenance-testkit/src/render.rs::bigquery_target_block_matches_the_parity_harness_shape` — `render_smelt_yml_for(ConformanceTarget::BigQuery{..}, ..)` emits a `bq:` block whose body matches `crates/smelt-cli/tests/common/mod.rs::bq_target_body`'s shape for the same dataset.
- `crates/smelt-maintenance-testkit/src/recipe.rs::conformance_dataset_is_derived_not_threaded` — two independent `bq_conformance_dataset(family, case)` calls in the same process agree, and differ across both `family` and `case`. This is the property that lets staging and the assertion loop compute the same name without threading state, exactly as `common::bq_dataset` does for the parity suites.
- `crates/smelt-maintenance-testkit/src/render.rs::duckdb_and_spark_rendering_is_unchanged` — the rendered `smelt.yml` for `DuckDb` and `SparkDelta` is byte-identical to before the enum change.

**Implementation shape.** `ConformanceTarget::BigQuery { dataset: String }`; the enum drops `Copy` and keeps `Clone` (the compiler names every site that relied on `Copy`). `bq_conformance_dataset(family, case) -> String` = `<base>_conf_<family>_<pid>_<case>`, mirroring `common::bq_dataset`'s derivation. BigQuery arms in `render_target_block`, `stage_for_target`, `stage_keyed_for_target`, `stage_composed_for_target`, `stage_feed_keyed_for_target`, `stage_dag_for_target`, and `LinkCProject::{run_with_target, backend_for_target}`. `stage_for_target`'s `#[cfg(feature = "spark")]` becomes `#[cfg(any(feature = "spark", feature = "bigquery"))]` with per-arm gating inside. A fresh dataset per case makes staging idempotent by construction, so the BigQuery arm needs no drop-before-seed step (unlike Spark's persistent warehouse).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-maintenance-testkit/Cargo.toml` — new `bigquery` feature, optional `smelt-backend-bigquery` dep.
- `crates/smelt-maintenance-testkit/src/recipe.rs` — enum, dataset derivation.
- `crates/smelt-maintenance-testkit/src/{render,dag,feed,link_c_harness}.rs` — BigQuery arms.
- `crates/smelt-cli/Cargo.toml` — `bigquery` feature forwards to `smelt-maintenance-testkit/bigquery`.

**Docs touched.**
- `docs/specs/multi_backend.md` §"Generative equivalence coverage" — state that the harness is parametrized over the backend under test, naming the target seam. Behaviour, not phase.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] DuckDB and Spark rendering is asserted byte-identical, not assumed
- [ ] Dataset name is derived independently by staging and readback, never threaded
- [ ] `cargo tree -p smelt-maintenance-testkit --no-default-features -i smelt-backend-bigquery` finds no edge
- [ ] Spec edit is timeless

**Commit.** `feat: a BigQuery arm on the conformance-target seam`

---

### Phase 3: Token preflight and quota-refusal classification

**Goal.** A BigQuery sweep refuses to start if it cannot finish, and a quota refusal is a classified, bounded condition rather than something the harness absorbs.

**Pre-conditions.** Phase 1's measured numbers.

**TDD tests to write first.**
- `crates/smelt-maintenance-testkit/src/bigquery_session.rs::preflight_refuses_a_window_too_short_for_the_sweep` — given a token expiry stamp closer than the sweep's estimated duration, preflight returns an error naming the shortfall.
- `..::preflight_refuses_an_absent_or_unparseable_expiry_stamp` — a missing stamp is a refusal, never an assumed-good window.
- `..::expired_token_is_classified_despite_having_no_http_status` — the google-auth client-side message an expired BigQuery token actually produces (no 401, no 403, no HTTP status at all) is classified as an auth failure. This exact shape already defeated the type-oracle leg once and is why `check_types_against_oracle` now fails on anything outside an allow-list.
- `..::quota_refusal_is_allow_listed_and_everything_else_fails` — the `quota for table update operations` shape is retryable-with-backoff up to a bounded count; an unrecognised 4xx is not.
- `..::retry_exhaustion_fails_the_leg` — backoff that never succeeds ends as a failure, never as a skip or a silent pass.

**Implementation shape.** A `bigquery_session` module in the testkit: `preflight(estimated_duration) -> Result<()>` reading the expiry stamp `bigquery-auth.sh` writes beside the token; `classify_bq_error(&Error) -> BqErrorClass { QuotaRetryable, Auth, Other }`; a `pace(delay)` hook whose delay comes from Phase 1's measurement. Bounded retry only for `QuotaRetryable`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-maintenance-testkit/src/bigquery_session.rs` — new.
- `crates/smelt-maintenance-testkit/src/lib.rs` — module wiring.

**Docs touched.** None (internal harness mechanics; the spec's user-visible claim lands in Phase 6).

**Review checklist** (material findings only):
- [ ] Every tolerated error shape is allow-listed by name; the default is failure
- [ ] No path converts a quota or auth failure into a green skip
- [ ] Pacing delay traces to Phase 1's measured number, cited in a comment
- [ ] Retry is bounded and exhaustion fails loud

**Commit.** `feat: token preflight and quota-refusal classification for BigQuery sweeps`

---

### Phase 4: Extract the shared families into one parametrized owner

**Goal.** The test families the Spark leg re-derives become target-generic functions in the testkit, behind a small backend trait. The Spark leg binary becomes thin wrappers. No BigQuery yet — this phase must be provable against Spark alone.

**Pre-conditions.** Phase 2's seam.

**TDD tests to write first.**
- `crates/smelt-maintenance-testkit/src/families/mod.rs::extracted_families_stage_byte_identical_projects_for_spark` — for a fixed seed, the extracted path stages the same `smelt.yml`, model SQL, and source YAML as the pre-extraction Spark path. This is the cheap standing guard on the move; it does not replace the live re-run below.
- `crates/smelt-cli/tests/maintenance_conformance_spark/*.rs` — every existing `#[test]` name survives the extraction unchanged, so a failure still names one family on one backend. Assert by keeping the test list and letting the compiler enforce it.
- **Live gate (not a `cargo test` assertion):** the full Spark leg re-run, green, per "Verification".

**Implementation shape.** A `families` module in the testkit exposing `async fn run_<family>(b: &dyn ConformanceBackend) -> Result<()>` for each of the nine shared families, plus:

```rust
trait ConformanceBackend {
    fn target(&self, case: usize) -> ConformanceTarget;  // per-case dataset for BigQuery
    fn skip_reason(&self) -> Option<String>;             // env unset → green skip
    fn corrupt_sql(&self, recipe: &ModelRecipe) -> String; // Delta refuses subqueries; GoogleSQL differs again
    async fn before_step(&self);                          // pacing hook (Phase 3)
}
```

The Spark binary keeps its `#[test]` functions; each constructs a `SparkConformanceBackend` and calls the corresponding `run_<family>`. Backend-specific deviations stay as explicit trait methods, never as `match target` inside a family body — a family that branches on its target is not parametrized.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-maintenance-testkit/src/families/` — new.
- `crates/smelt-cli/tests/maintenance_conformance_spark/*.rs` — reduced to wrappers.

**Docs touched.**
- `docs/specs/multi_backend.md` §"Generative equivalence coverage" — the shared families have one owner; per-backend deviations are declared, not branched on.

**Review checklist** (material findings only):
- [ ] No family body matches on `ConformanceTarget`
- [ ] Every pre-existing Spark test name still exists
- [ ] The DuckDB leg is untouched
- [ ] Live Spark re-run recorded green in the Progress table's Commit notes
- [ ] Spec edit is timeless

**Commit.** `refactor: one parametrized owner for the shared conformance families`

---

### Phase 4b: Finish the parametrization the families only half-took

**Goal.** No shared family assumes a backend's dialect. The extraction moved the families to one
owner but left three assumptions frozen inside them, which the "does a family `match` on its
target?" check passes over precisely because a hardcoded assumption is not a branch.

**Why this exists.** The BigQuery leg cannot run against the warehouse until it lands, and each
item was found by trying: the equivalence oracle's multiset difference is `EXCEPT ALL`, which
GoogleSQL does not have; the mixed-dimension and pinned families' source DDL says `USING DELTA`;
the DAG helpers name `SPARK_CONFORMANCE_SCHEMA` directly, so a non-Spark run targets the wrong
schema silently rather than failing. A fourth is a plain compile break: those helpers are gated
`#[cfg(feature = "spark")]` while `families::dags` is gated `any(spark, bigquery)`.

**Pre-conditions.** Phase 4.

**TDD tests to write first.**
- `crates/smelt-maintenance-testkit/src/families/mod.rs::no_family_hardcodes_a_backend_dialect` — a
  source-level assertion over the family modules and the oracle: no occurrence of `EXCEPT ALL`,
  `USING DELTA`, or `SPARK_CONFORMANCE_SCHEMA` outside a `ConformanceBackend` implementation. This
  is the check Phase 4's "no `match` on target" test should have been; a hardcoded dialect is the
  same defect as a branch, wearing different clothes.
- `..::multiset_difference_is_backend_supplied_and_stays_a_multiset` — the oracle's difference is
  supplied by the backend, and every backend's form detects a *duplicate-only* divergence (two
  copies of a row on one side, one on the other). `EXCEPT ALL`'s multiset semantics are
  load-bearing for the oracle — `oracle.rs` documents why — so BigQuery, which has only
  `EXCEPT DISTINCT`, must emulate the multiset difference (rank the duplicates within each row
  group and difference on the ranked rows) rather than silently degrade to set semantics. Falling
  back to `EXCEPT DISTINCT` is admissible only if the emulation is shown impractical, and then
  only as a §Known Divergences entry naming the class of divergence BigQuery's leg stops
  detecting — never as an unrecorded weakening.
- `cargo check -p smelt-cli --features bigquery --tests` compiles — the standing gate the current
  cfg mismatch fails.

**Implementation shape.** Widen `ConformanceBackend` with the seams the families actually need:
the set-difference operator, the source-table storage clause (empty for engines without one), and
the schema a case targets. `open_backend` gains the `case` argument BigQuery's per-case dataset
requires. `dag.rs`'s backend-driven helpers take their schema and storage clause as arguments and
are gated `any(feature = "spark", feature = "bigquery")` like their caller.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-maintenance-testkit/src/families/*.rs`, `oracle.rs`, `dag.rs`.
- `crates/smelt-cli/tests/maintenance_conformance_spark/backend.rs` and
  `crates/smelt-cli/tests/maintenance_conformance_bigquery/backend.rs` — trait-impl updates only.

**Docs touched.**
- `docs/specs/multi_backend.md` §Known Divergences — what BigQuery's set-difference comparison
  gives up relative to the multiset oracle the other legs run.

**Review checklist** (material findings only):
- [ ] No family body hardcodes a dialect keyword, asserted by a test rather than by inspection
- [ ] DuckDB and Spark still get `EXCEPT ALL`; the multiset weakening is BigQuery-only and recorded
- [ ] The live Spark leg re-runs green — this is a refactor of the code that leg exercises
- [ ] `cargo check -p smelt-cli --features bigquery --tests` passes
- [ ] Spec edit is timeless

**Commit.** `refactor: the last backend assumptions out of the shared conformance families`

---

### Phase 5: The BigQuery leg

**Goal.** `maintenance_conformance_bigquery` runs the shared families against the live warehouse, with per-case dataset lifecycle and Phase 3's pacing, and its oracle is proved non-vacuous.

**Pre-conditions.** Phases 2–4. A human-minted token.

**TDD tests to write first.**
- `crates/smelt-cli/tests/maintenance_conformance_bigquery/harness_self_check_bigquery.rs::oracle_flags_a_seeded_divergence_on_bigquery` — after a green run, corrupt one maintained row via `Backend::execute_sql` (never a raw write) and assert the S-restricted oracle reports inequality. **Write this first, before the family wrappers**: without it a green leg is indistinguishable from a vacuous one, which is the whole reason the Spark leg carries the same test.
- `..::each_case_gets_a_fresh_dataset` — two cases in one run resolve to different datasets, and each is dropped on the way out.
- `..::skips_green_when_SMELT_BQ_PROJECT_is_unset` — the leg is absent, not failing, without credentials.
- One wrapper per shared family, named `<family>_on_bigquery`.

**Implementation shape.** A `maintenance_conformance_bigquery` binary, `#![cfg(feature = "bigquery")]`, mirroring the Spark binary's wrapper shape. A `BigQueryConformanceBackend` implementing `ConformanceBackend`: `target(case)` returns `BigQuery { dataset: bq_conformance_dataset(family, case) }`, `before_step` applies Phase 3's pacing, `corrupt_sql` uses a GoogleSQL-valid unconditional update. Dataset drop on case exit, with `bigquery-env.sh`'s default table expiration as the backstop for an interrupted run — the same two-layer cleanup the parity harness uses.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/tests/maintenance_conformance_bigquery/` — new.
- `crates/smelt-cli/Cargo.toml` — test target registration.

**Docs touched.**
- `docs/specs/multi_backend.md` §"Parity contract" supported-surface statement — BigQuery's incremental coverage is generative, not fixed-recipe-only.

**Review checklist** (material findings only):
- [ ] The self-check test exists and was observed failing the oracle before the family wrappers landed
- [ ] Corruption goes through the Backend trait, never a raw write
- [ ] Every case's dataset is dropped, and an interrupted run still expires
- [ ] Credentials absent ⇒ green skip; credentials invalid ⇒ loud failure
- [ ] Spec edit is timeless

**Commit.** `test: a BigQuery leg on the generative maintenance-conformance gate`

---

### Phase 6: Runner, spec, and coverage statement

**Goal.** One command runs the leg, and the spec's coverage claims match reality.

**Pre-conditions.** Phase 5 green against the live warehouse.

**TDD tests to write first.**
- `scripts/bigquery-conformance.sh` — fails loud with a named reason when no token is present, rather than running a sweep that will die mid-case.
- `cargo test -p smelt-cli --test tutorial_freshness` and `example_diagnostics` stay green (the standing docs/workspace gates).

**Implementation shape.** `scripts/bigquery-conformance.sh` alongside `bigquery-parity.sh`, deliberately *not* invoked by it: parity is a bounded sweep you run routinely; conformance may want the whole token window. Record the measured full-sweep wall-clock in the handoff so the next session can size its token against a number rather than a guess.

**Critical files (allowed to touch in this phase).**
- `scripts/bigquery-conformance.sh` — new.
- `docs/handoffs/2026-08-16-bigquery-backend.md` — refresh.

**Docs touched.**
- `docs/specs/multi_backend.md` §"CI tiering" — where the BigQuery leg runs and why it is not per-PR; §Known Divergences — retire the fixed-recipe-only BigQuery entry, and record the token-window constraint as the live gap it is.
- `docs-site/docs/guide/targets.md` — a sentence on BigQuery's verification coverage, written as a feature description.

**Review checklist** (material findings only):
- [ ] The retired Known Divergence is deleted, not marked landed (a fully-landed entry is not a divergence — `docs/specs/CLAUDE.md`)
- [ ] The token-window constraint is recorded as a gap with its tracking plan
- [ ] Measured sweep wall-clock recorded
- [ ] Spec and user-doc edits are timeless

**Commit.** `docs: BigQuery generative conformance coverage and runner`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- **`hardening-budget.sh` mis-classifies the testkit crate.** The gate's directory heuristic treats
  `crates/*/src` as production and `*/tests/*` as test code. `smelt-maintenance-testkit` is
  test-support code all the way down, so moving the shared families out of
  `smelt-cli/tests/maintenance_conformance_spark/` and into the testkit's `src/` moved ~126
  `.expect(` sites from uncounted to counted without changing a line of their behaviour, and the
  baseline was raised to match. The counts are real but the signal is not: the same relocation
  would inflate the baseline again as further backends' family code lands. The right fix is an
  exclusion for the crate (or for `src/families/`) in `.claude/scripts/hardening-budget.sh`, which
  is a change to the gate itself and outside this plan's scope.

## Verification

- `bash scripts/bigquery-auth.sh` (human), then `bash scripts/bigquery-conformance.sh` — green against the live warehouse.
- **Live Spark re-run — the gate on Phase 4.** `bash scripts/spark-up.sh` **from this worktree** (the container binds to whichever worktree last ran it; a stale binding gives silent path mismatches rather than an error), `source scripts/spark-env.sh`, then `cargo test -p smelt-cli --features smelt-cli/spark --test maintenance_conformance_spark -- --test-threads=1`, then `bash scripts/spark-down.sh`.
- `cargo test -p smelt-cli --test maintenance_conformance` — the DuckDB leg, unchanged.
- `bash .claude/scripts/verify-phase.sh`.
- `/smelt:validate multi_backend` reports zero drift.

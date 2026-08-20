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
| 4b    | done     | a5df9508 | 2026-08-17 |
| 5     | done     | 7f2e27a9 | 2026-08-17 |
| 5b    | done     | 4b3aa719 | 2026-08-18 |
| 5c    | done     | e028596e | 2026-08-18 |
| 5d    | done     | 76913b37 | 2026-08-18 |
| 6     | done     | a683054b | 2026-08-18 |
| 7     | done     | aee11375 | 2026-08-18 |
| 8     | done     | 4ad7a898 | 2026-08-19 |
| 9     | done     | 6dd2e78c | 2026-08-19 |
| 10    | done     | 970ef87a | 2026-08-19 |

Phase 5d's live re-run of `append_only_partition_pool_upholds_equivalence_on_bigquery` shows the
`MEDIAN` gap closed (and with it a `_col2` gap the lowering exposed in the compiler's type-cast
wrapper); the case now reaches the next, unrelated harness gap — `CREATE OR REPLACE TEMPORARY
VIEW` for the S-restricted oracle relation, which BigQuery refuses. That belongs with the other
oracle-relation portability work, not with Phase 5d.

Live Spark re-run — the standing gate on every change to shared harness code — is green at
`e028596e`: all 19 `maintenance_conformance_spark` tests pass in 190s, covering both the
twin-target seam and the dialect-aware row-set owner.

**Whole-sweep close-out (2026-08-21).** The single uninterrupted BigQuery sweep the plan could not
take on its own — Phase 10 recorded it as the one owed measurement — is now taken and green:
`bash scripts/bigquery-conformance.sh`, `--test-threads=1`, **21 passed / 0 failed / 0 ignored,
2190.85s**. Every case that had only ever been verified by a targeted run is confirmed in the same
sweep as every other. Two things the measurement corrected: an all-green sweep costs almost twice
a failing one (2190.85s against 1142.10s, because a failing case exits fast), so the "large
headroom" the credential-window divergence claimed was an artefact of measuring a red suite; and
the `dags` family alone takes ~14 minutes of the window, so pace extrapolated from it overstates
the total. Both are recorded in `docs/specs/multi_backend.md` §Known Divergences.

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

### Phase 5b: The two harness defects the first live run exposed

**Goal.** The conformance harness itself stops emitting SQL BigQuery cannot run, and stops
colliding two projects onto one dataset.

**Pre-conditions.** Phase 5's live run (7/21 passing, 2026-08-17).

**TDD tests to write first.**
- `crates/smelt-maintenance-testkit/src/s_tracker.rs` — `materialize_s_as_view` emits a row set
  every supported dialect accepts. It builds `SELECT * FROM (VALUES …) AS t(cols)` today, and
  GoogleSQL has no table-value constructor in `FROM` position (`400 Syntax error: Expected
  keyword JOIN but got ')'`, measured 2026-08-17). The portable form is chained
  `SELECT … UNION ALL SELECT …`; assert the emitted text for both the populated and the zero-row
  guard case.
- `crates/smelt-maintenance-testkit/src/families/dags.rs` — a case's incremental project and its
  full-refresh twin resolve to *different* targets. Both call `b.target(case)` with the same
  `case` today, which is harmless where a target is a local file and fatal where it is a shared
  dataset (`409 Already Exists` on the twin's first table create).

**Critical files.** `crates/smelt-maintenance-testkit/src/s_tracker.rs`,
`crates/smelt-maintenance-testkit/src/families/dags.rs`, and the `ConformanceBackend` seam in
`families/mod.rs` if the twin needs its own target hook.

**Review checklist** (material findings only):
- [ ] The row-set form is asserted portable by test, not by inspection
- [ ] The twin's target is distinct by construction, not by a naming convention a caller must honour
- [ ] The live Spark leg re-runs green — this is shared-harness code

**Commit.** `fix: portable row sets and per-twin datasets in the conformance harness`

---

### Phase 5c: One dialect-aware owner for inline row sets

**Goal.** smelt stops emitting `FROM (VALUES …)` to backends that cannot parse it. This is a
product defect the generative leg caught, not a test-harness one.

**Pre-conditions.** Phase 5b.

**Why a seam rather than four patches.** Four production paths build an inline row set and none is
dialect-aware: `smelt-core/src/seeds/ephemeral.rs::build_values_cte` (the only shared helper, used
by ephemeral seeds through `execute_project`), `smelt-runtime/src/maintenance_driver.rs`'s
repair-keys literal, `smelt-logical/src/maintenance/emit.rs`'s append-only baseline probe, and
`smelt-cli/src/test_compiler.rs`'s mock-data compiler. Patching them individually would leave the
fifth author to rediscover the gap, so the row-set constructor gets a single dialect-aware owner
and the four call sites consume it.

**TDD tests to write first.**
- A rejection test in the row-set owner: for `SqlDialect::BigQuery` the emitted form is not a
  `FROM (VALUES …)` constructor, and for DuckDB/Spark it is byte-identical to today's output.
- A test per call site that its emitted SQL routes through the owner rather than formatting its own.

**Critical files.** `crates/smelt-core/src/seeds/ephemeral.rs` (or wherever the owner lands),
`crates/smelt-runtime/src/maintenance_driver.rs`, `crates/smelt-logical/src/maintenance/emit.rs`,
`crates/smelt-cli/src/test_compiler.rs`, `crates/smelt-dialect/src/dialect.rs` if a capability flag
is the chosen seam (note: `BackendCapabilities` has no `Default`, so a new field is a compile
error in all five constructors until each is updated — that is the intended forcing function).

**Docs touched.**
- `docs/specs/multi_backend.md` §"Capability matrix" if a flag is added; §Semantics for the rule
  that an inline row set has one dialect-aware owner.

**Review checklist** (material findings only):
- [ ] No production path formats its own `FROM (VALUES …)` after this phase
- [ ] DuckDB and Spark output is asserted byte-identical
- [ ] The capability table and its constructors agree (the table is normative)
- [ ] Spec edit is timeless

**Commit.** `fix: one dialect-aware owner for inline row sets`

---

### Phase 5d: `MEDIAN` on BigQuery — lower it or declare it

**Goal.** Decide, from measurement, whether `MEDIAN` can be lowered faithfully to GoogleSQL, and
implement whichever answer the warehouse gives.

**Pre-conditions.** `scripts/bigquery-probe-lowering.sh` run against a live token.

**The question.** GoogleSQL has no `MEDIAN`. `PERCENTILE_CONT` is analytic-only, so it cannot
stand in the `GROUP BY` position the recipe pool generates; `APPROX_QUANTILES` is an aggregate but
approximate. Substituting an approximate function under an equivalence oracle would make the
oracle report divergences that are artefacts of the substitution, or hide real ones — a silent
weakening of exactly the kind the fail-loud discipline forbids. The printer's existing
`remap_function_name` is rename-only and cannot express either candidate.

**The two admissible outcomes.** Either a faithful lowering exists (an exact, aggregate-position
form measured to agree with DuckDB's `MEDIAN` on an even-count fixture, where interpolating and
nearest-rank answers differ), and it lands with that measurement recorded; or it does not, and
`MEDIAN` becomes a declared unsupported construct on BigQuery that **fails loud at compile time**
with a diagnostic naming the backend and the function. Silent approximation is not an option.

**Critical files.** `crates/smelt-dialect/src/printer.rs`, `crates/smelt-types/src/signatures.rs`
if the registry carries the per-dialect availability, plus the recipe pool's holistic-aggregate
generator if the construct must be withheld from BigQuery cases.

**Docs touched.**
- `docs/specs/multi_backend.md` §Known Divergences or §Semantics, depending on the outcome.

**Review checklist** (material findings only):
- [ ] The decision cites a measurement, not documentation
- [ ] If lowered: agreement with DuckDB shown on a fixture where interpolating and nearest-rank differ
- [ ] If declared: the failure is loud and names both the backend and the function
- [ ] Spec edit is timeless

**Commit.** `feat: MEDIAN on BigQuery, lowered or declared`

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

### Phase 7: The S-restricted oracle relation and the composed family's row set

**Goal.** Close the two remaining GoogleSQL gaps a live sweep on 2026-08-18 measured as the ONLY
causes of the leg's 11 failures (10 passed / 11 failed / 886.60 s): `STracker::materialize_s_as_view`'s
`CREATE OR REPLACE TEMPORARY VIEW` (10 of the 11) and `gate_composed.rs`'s hand-rolled
`(VALUES …) AS t(id, d, val)` row set (the 11th). Both are test-harness code in
`crates/smelt-maintenance-testkit/`; no production path is implicated.

**Pre-conditions.** Phase 6's live sweep result (2026-08-18), which isolated these as the only two
remaining defects.

**Gap 1 — the S-restricted oracle relation.** `STracker::materialize_s_as_view`
(`crates/smelt-maintenance-testkit/src/s_tracker.rs`) issues `CREATE OR REPLACE TEMPORARY VIEW
oracle_<source>`; BigQuery refuses this outright (`400 CREATE TEMP VIEW is unsupported`). Its
callers (`families/gate.rs`, `families/gate_keyed.rs`, `families/gate_mixed.rs`) then reference the
relation by the unqualified name `oracle_<source>`. Closed with a seam on `ConformanceBackend`
(`oracle_relation`), not a rewrite of the existing path: DuckDB/Spark's default implementation
reproduces today's behaviour byte-for-byte (materialize the temp view, return its bare name);
BigQuery's override issues no DDL at all and returns an inline derived table `(<the S_k SELECT>)
AS oracle_<source>` instead, built from `STracker::s_select_sql` — the same portable row-set query
`materialize_s_as_view` already uses (Phase 5b), now exposed as a public accessor rather than
duplicated. `gate_mixed.rs`'s fact-join template supplies its own trailing alias (`f`) immediately
after the substitution point, so splicing in a second `AS oracle_<name>` alias would double-alias;
that one call site strips the derived table's `AS oracle_<name>` suffix back to a bare `(...)`
before splicing (`bare_relation_for_alias`) rather than pushing the generic form through unmodified.

**Gap 2 — the composed family's hand-rolled row set.** `gate_composed.rs`'s
`composed_delta_values_sql` builds `format!("(VALUES {}) AS t(id, d, val)", …)` directly into the
staged model SQL, so BigQuery fails with `400 Syntax error: Expected keyword JOIN but got ","`.
Routed through the existing dialect-aware owner
`smelt_core::sql::row_set::build_row_set_table` (Phase 5c) instead, parametrized over a new
`ConformanceBackend::dialect` accessor (DuckDB by default; Spark and BigQuery override it).
DuckDB/Spark output stays byte-identical.

**TDD tests to write first.**
- `families/mod.rs` — the trait default `oracle_relation` still materializes the temp view and
  returns the bare name (executed against a real DuckDB backend); a BigQuery-shaped override
  issues NO backend calls at all (a call-counting/panicking `Backend` double) and returns a
  parenthesised derived table, which is itself valid queryable SQL.
- `gate_composed.rs` — `composed_delta_values_sql` is byte-identical for DuckDB/Spark and contains
  no `(VALUES ` under the BigQuery dialect.
- `families/mod.rs`'s existing `no_family_hardcodes_a_backend_dialect` gate extended with a
  `"(VALUES "` needle, so a hand-rolled table-value constructor in a family body fails the same
  gate `EXCEPT ALL`/`USING DELTA` already fail.

**Implementation shape.** No live BigQuery access in this phase (`gcloud`/`bq` denied to agents,
per the handoff) — the offline tests above carry the weight; the live re-run is the orchestrator's
job in Verification.

**Critical files.** `crates/smelt-maintenance-testkit/src/s_tracker.rs`,
`crates/smelt-maintenance-testkit/src/families/mod.rs`,
`crates/smelt-maintenance-testkit/src/families/{gate,gate_keyed,gate_mixed,gate_composed}.rs`,
`crates/smelt-cli/tests/maintenance_conformance_bigquery/backend.rs`,
`crates/smelt-cli/tests/maintenance_conformance_spark/backend.rs`.

**Docs touched.** None — the §Known Divergences entries these gaps map to in
`docs/specs/multi_backend.md` describe measured live behaviour and are retired only once the
orchestrator's live re-run confirms both gaps closed, which is outside this phase.

**Review checklist** (material findings only):
- [ ] DuckDB's standing gate (`cargo test -p smelt-cli --test maintenance_conformance`) is
      byte-identical, not merely green
- [ ] The Spark leg's default `oracle_relation`/`dialect` reproduce prior behaviour exactly
- [ ] `gate_mixed.rs`'s trailing-alias adaptation is tested, not asserted by inspection
- [ ] The BigQuery override issues no backend calls (proven, not assumed)
- [ ] `no_family_hardcodes_a_backend_dialect`'s new needle actually would have caught the
      pre-fix `gate_composed.rs`

**Commit.** `fix: portable S-restricted oracle relation and composed row set for BigQuery`

**Measured result.** Before (2026-08-18, Phase 6's live sweep): 10 passed / 11 failed, 886.60s.
After (2026-08-18, `bash scripts/bigquery-conformance.sh`, `--test-threads=1`): **13 passed / 8
failed / 0 ignored, 1142.10s**. Both targeted gaps are confirmed closed — no case fails on
`CREATE OR REPLACE TEMPORARY VIEW` or the composed family's hand-rolled `(VALUES …)` row set any
more, and `harness_self_check_bigquery::oracle_flags_a_seeded_divergence_on_bigquery` now passes,
the first live proof BigQuery's leg has a non-vacuous oracle. Eight cases still fail, none on
either gap this phase targeted: one product-side dialect gap
(`build_cumulative_merge_sql`, `crates/smelt-runtime/src/cumulative.rs:621`, hardcodes
`MaintenanceDialect::DuckDb` so a keyed-fold `MERGE`'s not-matched arm emits `INSERT *` instead of
`INSERT ROW` on BigQuery), one harness-side gap where the oracle's raw-SQL rendering bypasses
smelt's `MEDIAN` lowering, one harness-side gap where the default `Backend::execute_model`
(`crates/smelt-backend/src/lib.rs:216`) unconditionally issues `DROP VIEW IF EXISTS` against a
`TABLE`, one harness-side staging collision in the `pinned` family, and two cases whose failure
cause was not captured live (needs a fresh run). Full breakdown in
`docs/specs/multi_backend.md` §Known Divergences and `docs/handoffs/2026-08-16-bigquery-backend.md`.
None of the eight was fixed as part of this phase — recording them was the scope.

---

### Phase 8: Close the four characterised causes

**Goal.** Land an offline fix for each of the four causes Phase 7 characterised, leaving only the
two uncharacterised failures, which need a fresh live sweep to diagnose.

**Pre-conditions.** Phase 7's recorded breakdown.

**What landed.**
- *Product-side.* `build_cumulative_merge_sql` no longer hardcodes `MaintenanceDialect::DuckDb`:
  the dialect threads from `run_windowed_keyed_maintenance` through a new parameter on
  `WindowedKeyedRule::merge_sql`, resolved once via `smelt_backend::maintenance_dialect
  (backend.dialect())` — the same resolution the driver already uses for `emit_create_table_as`
  and `emit_recurrence_bound_probe`. `INSERT ROW` on BigQuery, byte-identical `INSERT *` on
  DuckDB.
- *Product-side.* BigQuery's `drop_table_if_exists`/`drop_view_if_exists` classify the
  wrong-object-type `400` as "already absent", restoring the `IF EXISTS` contract at the backend
  that deviates rather than changing the shared default `execute_model` (which would put the
  byte-stable DuckDB and Spark paths at risk). The classifier is an allow-list over the two
  measured GoogleSQL error shapes; every other error, including quota refusals, still propagates.
- *Harness-side.* `STracker`'s S-restricted oracle body round-trips through `smelt_dialect::print`
  under the target's own dialect, so the exact-`MEDIAN` GoogleSQL lowering reaches the oracle
  instead of only compiled models. DuckDB and Spark output is asserted byte-identical.
- *Harness-side.* Each independent physical staging in the `pinned` family carries its own case
  index through the existing `ConformanceBackend::target`/`schema` seam. Phase 5b's fix covered
  two writers sharing one target; `pinned`'s bug was the N-writers-one-target generalisation,
  closed the same way rather than by a new seam method.
- The `no_family_hardcodes_a_backend_dialect` scan reads a family file's production body only. It
  was red at `HEAD`: Phase 7's own byte-identity test quotes DuckDB's `(VALUES …)` output
  verbatim, which the whole-file scan could not distinguish from the hand-rolled constructor it
  exists to forbid. A body-level violation is still caught.

**Verification.** `bash .claude/scripts/verify-phase.sh` — all green. No live BigQuery access in
this phase; the live re-run belongs to the orchestrator.

**Not closed.** The two uncharacterised failures
(`dags_bigquery::diamond_propagation_suffices_on_bigquery`,
`gate_composed_bigquery::composed_keyed_pool_upholds_equivalence_on_bigquery`) still need a fresh
live sweep that captures full failure text, and no §Known Divergences entry retires until a live
sweep confirms the six fixed cases pass.

**Commit.** `fix: thread the target dialect into the keyed-fold MERGE` + `fix: honour DROP ... IF
EXISTS across an object-type mismatch on BigQuery` + `fix: dialect-aware S-restricted oracle and
per-case staging in the harness`

---

### Phase 9: The 2026-08-19 live sweep and the operator-lowering gap

**Goal.** Re-measure the leg against the live warehouse with the Phase 8 fixes in, characterise
whatever remains, and close what the measurement newly exposes.

**Measured result.** `bash scripts/bigquery-conformance.sh`, `--test-threads=1`: **14 passed / 7
failed / 0 ignored, 1265.89s**. Five of the seven failures are the credential window expiring
mid-sweep — the token preflight refused `gate_mixed`, both `pinned` cases and two
`harness_self_check` cases (474s remaining against a 600s estimate), so they never ran and the
Phase 8 `pinned` fix is still unconfirmed. Confirmed fixed live: the `INSERT ROW` dialect gap,
both `DROP`-type-mismatch cases, and `gate_composed_bigquery::composed_keyed_pool_upholds_
equivalence_on_bigquery` (one of Phase 7's two uncharacterised failures — it was collateral from
the gaps already closed).

**What the sweep newly exposed.** `dags_bigquery::diamond_propagation_suffices_on_bigquery`, the
other uncharacterised failure, is `400 Syntax error: Expected ")" but got "%"` — a model body's
`id % 2` reaching GoogleSQL unlowered. Chasing it found a second, worse instance of the same
class: smelt's grammar reads `^` as DuckDB does (power), while GoogleSQL defines infix `^` as
bitwise XOR, so an unlowered `^` returns a *different number* instead of failing. Both are closed
in the printer (`docs/specs/multi_backend.md` §"Operator lowering"); `//` is deliberately left
unlowered, since the printer cannot see operand types and DuckDB's `//` is truncating on integers
but plain division on floats.

**Still open.** `gate_bigquery::append_only_partition_pool_upholds_equivalence_on_bigquery` clears
the `MEDIAN` error and fails on a genuine S-restricted equivalence violation, at the first run
whose `ColumnScopedMerge` takes the `MATCHED` arm. The two candidate causes an offline diagnosis
could not separate — a stale row, a partial `SET` column list, or a duplicate row from a
mismatched `ON` — need a live re-run; the assertion now prints the differing rows, which is why
this failure survived two sessions uncharacterised.

**Commit.** `fix: lower infix modulo to MOD() on BigQuery` + `fix: lower the power operators to
POWER() on BigQuery` + `test: name the rows an equivalence violation diverges on`

---

### Phase 10: Every case green, measured live

**Goal.** Drive the remaining live failures to zero.

**Measured result (2026-08-19, targeted runs against the live warehouse).** All 21 cases pass.
`diamond_propagation` (231.65s), `gate_mixed` (129.80s), all three `harness_self_check` cases
(108.21s), `append_only_partition_pool` (168.75s), `pinned_recipes_reproduce_catalogue_coverage`
and `hazard_schedules_are_pinned` (200.01s) were each re-run and each passed. A single whole-sweep
measurement is still owed and is the one thing this plan cannot close on its own: a one-hour
credential does not cover a ~21-minute sweep once part of the window is already spent, and the
token preflight refuses a family it cannot budget for rather than dying mid-case.

**What the last two causes turned out to be.**

- *The equivalence violation was not the median lowering — it was the cast wrap.* The row-level
  diff (added this phase, see below) showed `-285.0` against the oracle's exact `-284.5`.
  `apply_type_casts` re-parses SQL the printer has already lowered, so it saw
  `(CAST(x AS FLOAT64) + CAST(y AS FLOAT64)) / 2`; `FLOAT64` is not a spelling
  `smelt_types::parse_type` knows, both operands resolved to `None`, and division's promotion rule
  adopted the literal `2`'s `SmallInt`. The wrap then emitted `CAST(med_val AS SMALLINT)`, rounding
  every interpolated median before it left the warehouse. Division with exactly one unresolved
  operand now yields no type at all. DuckDB never reached this path: its dialect leaves `MEDIAN`
  as a plain call, so the already-correct `SqlFunction::Median` arm answers `Double` directly.
- *The `pinned` hazard case failed a second time, further along.* Phase 8's per-case staging fix
  worked — the `409 Already Exists` collision is gone — and the case then ran far enough to hit a
  hand-spelled `DOUBLE` in the g-10 staging DDL, which GoogleSQL does not have. Column types now
  come from `ConformanceBackend::int_type`/`double_type` like the storage clause already did, and
  `no_family_hardcodes_a_backend_dialect` gained DDL-shaped needles so the next hand-spelled type
  fails the gate rather than a live sweep.

**Why two cases stayed uncharacterised for two sessions.** The equivalence assertion printed the
maintained and oracle *queries* but never the rows they disagreed on, so a live failure carried no
more information than "these two SQL strings differ". It now prints a bounded sample of the
differing rows in both directions — the median cause was obvious within one run of having it.

**Commit.** `fix: never guess a division's type from one known operand` + `fix: resolve staged
column types through the conformance backend`

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

- **The Spark `dags` family may be comparing a project against itself.** `SparkConformanceBackend`'s
  `target`/`schema` ignore the case index and return the one fixed `SPARK_CONFORMANCE_SCHEMA`, and
  the DAG node names in `dag.rs` are fixed literals rather than case-parametrized. So every case's
  incremental project and its full-refresh twin write the same physical tables in the same shared
  warehouse schema, and because the full-refresh build runs after the incremental steps, the
  equality assertion can read one already-overwritten table for both sides — which would pass even
  if the incremental engine were wrong. This is pre-existing and unchanged by the twin-target seam:
  the seam's default reproduces the old single-target behaviour exactly, and only BigQuery, whose
  per-case dataset made the collision fatal rather than silent, overrides it. Making the Spark leg's
  `dags` assertions non-vacuous means giving Spark per-case schemas (or case-parametrized node
  names), which is a change to the Spark leg rather than to the BigQuery one.

## Verification

- `bash scripts/bigquery-auth.sh` (human), then `bash scripts/bigquery-conformance.sh` — green against the live warehouse.
- **Live Spark re-run — the gate on Phase 4.** `bash scripts/spark-up.sh` **from this worktree** (the container binds to whichever worktree last ran it; a stale binding gives silent path mismatches rather than an error), `source scripts/spark-env.sh`, then `cargo test -p smelt-cli --features smelt-cli/spark --test maintenance_conformance_spark -- --test-threads=1`, then `bash scripts/spark-down.sh`.
- `cargo test -p smelt-cli --test maintenance_conformance` — the DuckDB leg, unchanged.
- `bash .claude/scripts/verify-phase.sh`.
- `/smelt:validate multi_backend` reports zero drift.

# Plan: W4 — Spark toward first-class (per-PR CI, connection secrets, verified ledger)

**Date**: 2026-07-19
**Spec**: [`docs/specs/multi_backend.md`](../specs/multi_backend.md)
**Spec diff**: Phase 1 of this plan writes it (connection security + Spark supported-surface statement); later phases implement against it
**Tracking PR / branch**: `worktree-production`
**Docs**: code+docs
**Master**: [`docs/plans/20260719-production-readiness.md`](20260719-production-readiness.md) — this is sub-plan **W4**. Research basis: [`docs/research/20260719-production-release-review.md`](../research/20260719-production-release-review.md) (decision D1, blocker #1's Spark leg, "Spark CI is nightly-gated" secondary).

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/multi_backend.md` — it is the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `worktree-production`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**CRITICAL PREREQUISITE — live Spark server.** Phases 2 (integration leg), 4, and 5 need a live Delta-enabled Spark Connect server:

```bash
bash scripts/spark-up.sh
source scripts/spark-env.sh   # exports SPARK_CONNECT_URL, SMELT_SPARK_WAREHOUSE, PYTHONPATH, PYSPARK_PYTHON
```

These must be exported into the loop's environment before the iteration starts (the loop's stateless iterations do not stand Spark up). If a phase below is marked **[needs Spark]** and `SPARK_CONNECT_URL` is unset, **emit `<<PHASE_BLOCKED>>` with the reason** — never let Spark-targeted assertions skip green and call the phase done. Skip-green here is a silent hole: the entire point of this plan is verified Spark behaviour.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:** red-green TDD; real-fixture tests; verification gate is `bash .claude/scripts/verify-phase.sh`; atomic per-phase commits with the phase's `Commit.` line verbatim; never `--no-verify`; don't widen scope; honor `CLAUDE.md` invariants; Timeless-oracle rule — phase vocabulary stays in this file, spec/docs-site edits read as if the feature always existed.

---

## Context

The v0.5 release review positions Spark as beta unless promoted (decision D1). Three things keep it beta: a Spark regression can merge green because all three Spark CI jobs are nightly/label-gated; `connect_url` is plaintext in `smelt.yml` with no secrets path (research blocker #1); and the 21 `spark_type` divergence-ledger entries were written during soaks that later proved some entries stale. This plan closes all three and produces the evidence brief for Andrew's final supported-vs-beta label call — the call itself is his, not this plan's.

## Scope

### In scope (spec coverage)
- `multi_backend.md` §Surface + new §"Connection security": env-interpolated `connect_url`, token/TLS via Spark Connect URL parameters.
- `multi_backend.md` §"Parity contract": the explicit supported-surface statement for Spark and the CI tiering that enforces it (per-PR paths-gated + nightly full).
- `multi_backend.md` §"Output-schema type conformance": re-verified Spark divergence ledger.

### Explicitly deferred
- The supported-vs-beta label decision (Andrew's; Phase 6 only assembles the evidence).
- Spark-native maintenance-conformance twin gate (Phase 5 enumerates the gap list; building the twin is post-v0.5 — recorded in Deferred).
- Any new auth mechanism beyond what the Spark Connect URL grammar already carries (`;token=…;use_ssl=true` passes through to pyspark today).

## Progress tracking

| Phase | Status  | Commit | Date |
|-------|---------|--------|------|
| 1     | done    | (this commit) | 2026-07-20 |
| 2     | done    | (this commit) | 2026-07-20 |
| 3     | done    | (this commit) | 2026-07-20 |
| 4     | done    | (this commit) | 2026-07-20 |
| 5     | done    | (this commit) | 2026-07-20 |
| 6     | done    | (this commit) | 2026-07-20 |
| 7     | done    | (this commit) | 2026-07-20 |

## Phase detail

### Phase 1: Spec diff — connection security + Spark supported surface

**Goal.** Land the normative text later phases implement: a §"Connection security" subsection under §Semantics (env-interpolated `connect_url`; token/TLS carried as Spark Connect URL parameters, never new YAML keys) and a supported-surface statement under §"Parity contract" (what dual-target parity covers, what is excluded, and the two-tier CI contract: per-PR paths-gated + nightly full).

**Pre-conditions.** None (docs-only). Coordinate wording with the W2 secrets spec phase if it has already landed `${ENV_VAR}` interpolation semantics in `smelt_yml.md` — cite, don't duplicate.

**TDD tests to write first.** None (spec phase). Gate: `/smelt:validate multi_backend` after the plan completes reports the new sections as implemented, not drifted.

**Implementation shape.** Edit `docs/specs/multi_backend.md`: add §"Connection security" (interpolation reference, `sc://host:port/;token=${DATABRICKS_TOKEN};use_ssl=true` example, plaintext-token-in-YAML is a lint-worthy smell); extend §"Parity contract" with the supported-surface table (full/view/table/ephemeral + incremental maintenance legs verified by which test suite) and CI-tier statement. Record the not-yet-true parts (per-PR gating, verified ledger) under §Known Divergences with a link to this plan — behavioural terms, no phase vocabulary.

**Critical files (allowed to touch in this phase).**
- `docs/specs/multi_backend.md` — the two new/extended sections + Known Divergences rows.

**Docs touched.** Spec only in this phase; docs-site lands in Phase 6 once behaviour exists.

**Review checklist** (material findings only):
- [ ] Connection-security section defers interpolation *mechanics* to the config spec (single owner), only binds Spark specifics
- [ ] Supported-surface statement is falsifiable (names the test suites that enforce each row)
- [ ] Known Divergences rows describe behaviour gaps, not plan phases
- [ ] No phase vocabulary in spec body

**Commit.** `spec(multi-backend): connection security + Spark supported-surface statement`

### Phase 2: `connect_url` secrets — interpolation + authenticated URL passthrough **[needs Spark for the integration leg]**

**Goal.** `connect_url: sc://host:443/;token=${DATABRICKS_TOKEN};use_ssl=true` works: the `${ENV_VAR}` is interpolated at config load, and the token/TLS parameters reach pyspark's `builder.remote(...)` untouched.

**Pre-conditions.** W2's env-interpolation phase (master registry order guarantees W2 runs before W4). If interpolation has not landed in `smelt-core/src/config.rs`, emit `<<PHASE_BLOCKED>>` naming the missing W2 phase — do not implement a second interpolation mechanism here.

**TDD tests to write first.**
- `crates/smelt-core/src/config.rs::spark_connect_url_interpolates_env_var` — a `smelt.yml` with `connect_url: sc://h:443/;token=${SMELT_TEST_TOKEN};use_ssl=true` + the env var set loads with the token substituted; unset env var is a load-time diagnostic (fail-loud), not a silent empty string.
- `crates/smelt-backend-spark/src/tests.rs::connect_url_params_pass_through_verbatim` — `SparkBackend::new` hands the full parameterised URL to the Python adapter unmodified (assert via the adapter-arg capture path used by existing tests around `crates/smelt-backend-spark/src/tests.rs:228`).
- Real-fixture leg **[needs Spark]**: `crates/smelt-cli/tests/cross_engine_parity.rs::spark_target_via_interpolated_url` — a staged project whose `connect_url` is `sc://localhost:${SMELT_SPARK_PORT}` via env interpolation builds green against the live server.

**Implementation shape.** No new grammar: `config.rs:332`'s `connect_url: Option<String>` picks up interpolation from W2's shared pass; verify `SparkBackend::new` (`crates/smelt-backend-spark/src/lib.rs:62`) and `python/smelt/spark_adapter.py` (`builder.remote(connect_url)`) need no change beyond tests. Add the unset-var diagnostic case if W2's pass doesn't already cover nested-in-target strings.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-core/src/config.rs` — interpolation coverage for target-level strings + tests
- `crates/smelt-backend-spark/src/tests.rs`, `crates/smelt-cli/tests/cross_engine_parity.rs` — passthrough + fixture tests
- `python/smelt/spark_adapter.py` — only if passthrough proves broken

**Docs touched.**
- `docs/specs/multi_backend.md` — move the connection-security Known Divergences row to implemented
- `docs-site` Spark page defers to Phase 6

**Review checklist:**
- [ ] Exactly one interpolation implementation (W2's) — none added here
- [ ] Unset env var fails loud with a config diagnostic naming the variable
- [ ] No token value ever logged (check `tracing` calls on the connect path)
- [ ] Spark integration leg ran against a live server (not skipped)

**Commit.** `feat(spark): env-interpolated connect_url with token/TLS passthrough`

### Phase 3: Per-PR Spark CI on Spark-relevant paths

**Goal.** A PR touching Spark-relevant code runs `spark-parity` and `type-property-spark` before merge; nightly full runs are unchanged. A Spark regression can no longer merge green.

**Pre-conditions.** None (independent of Phases 1–2).

**TDD tests to write first.** CI is not unit-testable; the red-green here is empirical:
- A scratch commit touching `crates/smelt-backend-spark/src/lib.rs` (whitespace) pushed to this PR branch triggers both jobs (verify via `gh pr checks` / `gh run list --workflow=compat.yml`).
- A scratch commit touching only `docs/` does **not** trigger them.
- `schedule` path still runs everything (assert by inspecting the rendered `if:` conditions — the gate expression must keep `github.event_name == 'schedule'`).

**Implementation shape.** In `.github/workflows/compat.yml`: add a first `changes` job using `dorny/paths-filter` (or an equivalent `git diff --name-only` step) exposing a `spark: true/false` output over the filter list: `crates/smelt-backend-spark/**`, `crates/smelt-cli/tests/*parity*`, `crates/smelt-types/src/signatures.rs`, `crates/smelt-db/src/type_inference*/**`, `crates/smelt-parser/src/**` (dialect surface), `python/**`, `scripts/spark-*.sh`, `.github/workflows/compat.yml`. Rewrite the `if:` on `spark-parity` (line ~267) and `type-property-spark` (line ~218) to `schedule || label 'run-docker-tests' || needs.changes.outputs.spark == 'true'` and add `needs: changes`. Leave `spark-integration` (parser-compat, line ~182) nightly/label-gated — it is corpus-driven, not code-path-driven. Mind the workflow's `on: pull_request: branches: [main]` — the paths-filter job must handle `push` events to `main`/`feature/*` too (diff against the merge base).

**Critical files (allowed to touch in this phase).**
- `.github/workflows/compat.yml` — the `changes` job + two `if:` rewrites

**Docs touched.**
- `docs/specs/multi_backend.md` — CI-tier Known Divergences row → implemented

**Review checklist:**
- [ ] Nightly `schedule` still runs all three Spark jobs unconditionally
- [ ] `run-docker-tests` label escape hatch preserved
- [ ] Paths list covers the type-inference and signature-registry surfaces (a signatures.rs change must trigger `type-property-spark`)
- [ ] Both empirical trigger checks recorded in the phase commit message or PR comment

**Commit.** `ci(spark): per-PR spark-parity + type-property-spark on Spark-relevant paths`

### Phase 4: Divergence-ledger re-verification + soak **[needs Spark]**

**Goal.** Every one of the 21 `spark_type` entries in `crates/smelt-db/tests/prop_helpers/divergences.rs` is re-verified against the live server: stale entries deleted, entries masking real smelt inference bugs fixed in `smelt-db` type inference, survivors annotated with the verifying expression. Then a 1000-case soak confirms the shrunk ledger holds.

**Pre-conditions.** Live Spark server (else `<<PHASE_BLOCKED>>`). Phase 3 helpful but not required.

**TDD tests to write first.**
- For each entry judged stale: delete it and let `cargo test -p smelt-db --test type_property_tests` (Spark oracle mode) prove deletion is safe — red if the divergence was real.
- For each entry masking a smelt bug: a pinned regression test in `crates/smelt-db/tests/type_property_tests.rs` reproducing the expression with the *correct* expected type (red), then fix inference (green), then delete the ledger entry.
- Soak (not committed as a test): `PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_type_inference` with the Spark oracle env set — zero new unregistered divergences.

**Implementation shape.** Work entry-by-entry in ledger order; prior soaks (see memory: `spark_type: None` entries were often wrong assumptions) mean the default posture is *suspicion of the entry, not of DuckDB parity*. Each surviving entry gets a `verified: 2026-07-…` comment with the exact SQL used. Inference fixes go through the usual `smelt-db/src/type_inference` path honoring registry-first typing.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/tests/prop_helpers/divergences.rs` — deletions/annotations
- `crates/smelt-db/src/type_inference*` — real-bug fixes only
- `crates/smelt-db/tests/type_property_tests.rs` — pinned regressions

**Docs touched.**
- `docs/specs/multi_backend.md` §"Output-schema type conformance" — ledger-size statement refreshed

**Review checklist:**
- [ ] No entry deleted without a green oracle run proving it stale
- [ ] Every inference fix has a pinned red-first regression test
- [ ] Soak output (case count, result) recorded in the commit message
- [ ] Registry-migration ratchet not regressed by inference fixes

**Commit.** `test(spark): re-verify spark_type divergence ledger against live server; fix real inference bugs`

### Phase 5: Dual-target parity sweep + conformance-gap enumeration **[needs Spark]**

**Goal.** `cargo test -p smelt-backend-spark` and `cargo test -p smelt-cli --features smelt-cli/spark` fully green against the live server; every failure becomes a fix or an explicitly registered gap. Separately, enumerate which DuckDB `maintenance_conformance` legs have **no Spark twin** and record that list.

**Pre-conditions.** Live Spark server (else `<<PHASE_BLOCKED>>`). Phases 2–4 done (sweep runs on the secured, re-verified surface).

**TDD tests to write first.** The sweep *is* the red run:
- `cargo test -p smelt-backend-spark --quiet 2>&1 | tail -40`
- `cargo test -p smelt-cli --features smelt-cli/spark --quiet 2>&1 | tail -40`
- Every failure: fix red-green, or register in the relevant ledger/gap list with a named entry — **no `#[ignore]`, no skip-green**.
- Gap enumeration is analytical: cross-reference `crates/smelt-cli/tests/maintenance_conformance*` legs against the Spark-feature parity tests; output is a table (leg → covered-on-Spark? → why not).

**Implementation shape.** Run the sweep, triage failures into (a) smelt bugs — fix; (b) genuine engine divergences — ledger; (c) environment issues — fix `scripts/spark-*.sh`. Write the conformance-gap table into this plan's **Deferred during implementation** section (it is the post-v0.5 backlog seed and Phase 6's evidence input).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-backend-spark/**`, `crates/smelt-cli/tests/*parity*` — fixes
- ledger files for registered divergences
- this plan file — the gap table

**Docs touched.**
- `docs/specs/multi_backend.md` §Known Divergences — gap list summarized behaviourally

**Review checklist:**
- [ ] Zero skipped Spark assertions in the final green run (grep the test output for `skip`)
- [ ] Gap table complete: every maintenance_conformance leg dispositioned
- [ ] No `#[ignore]` added anywhere

**Commit.** `test(spark): full dual-target parity sweep green; enumerate maintenance-conformance Spark gaps`

### Phase 6: Docs promotion + supported-vs-beta evidence brief

**Goal.** The docs-site Spark backend page reflects the secured, CI-enforced surface; a compact evidence brief lands in the master plan for Andrew's label call.

**Pre-conditions.** Phases 1–5 done.

**TDD tests to write first.**
- `cargo test -p smelt-cli --test example_diagnostics` — any example touched stays diagnostic-clean.
- Docs build: the docs-site build (uv-based, per CI) succeeds with the new/updated page.

**Implementation shape.** Update/create the docs-site Spark backend page: setup (`spark-up.sh`/Connect URL), secrets (`${ENV_VAR}` + Databricks token example), capabilities, and a limitations table generated from Phase 5's gap list. Append to `docs/plans/20260719-production-readiness.md` a ≤20-line evidence brief: per-PR CI status, final ledger size, gap-table summary, sweep results — ending with the open question "supported or beta for v0.5?" explicitly addressed to Andrew.

**Critical files (allowed to touch in this phase).**
- `docs-site/docs/**` — Spark backend page
- `docs/plans/20260719-production-readiness.md` — evidence brief

**Docs touched.**
- `docs-site/docs/**` (timeless feature description; limitations table in behavioural terms)
- `docs/specs/multi_backend.md` — final Known Divergences sweep for this plan's rows

**Review checklist:**
- [ ] Limitations table matches Phase 5's gap table (no rosier claims than tests prove)
- [ ] No phase vocabulary in docs-site page
- [ ] Evidence brief states facts + the open decision, not a recommendation disguised as fact

**Commit.** `docs(spark): backend page for secured surface + supported-vs-beta evidence brief`

### Phase 7: Post-completion remediation — CI-gate hardening, evidence corrections, empirical verification

**Goal.** Close the four findings from the 2026-07-20 post-completion review of this plan:

1. **Phase 3's empirical trigger checks were never run** — the phase commit substituted
   static inspection of the rendered `if:` expressions for the required scratch-commit
   red-green via `gh pr checks`, and no tracking PR from `worktree-production` existed to
   fire a `pull_request` event at all (master pre-flight step 5 unfulfilled).
2. **Nightly skip hole in the new CI wiring** — `spark-parity` and `type-property-spark`
   carry `needs: changes` with an `if:` containing no status-check function; per GitHub
   Actions semantics a failure of the `changes` job (plausible on non-PR events: the
   `base:` fallback resolves to the literal `HEAD~1` on `schedule`) silently skips both
   jobs even on nightly — the exact skip-green failure mode this plan exists to close.
3. **Evidence-brief count wrong** — the D1 brief in the master plan says "all 24
   `spark_type` entries"; the ledger has 22 (20 annotated `verified: 2026-07-20` + the 2
   by-design leniency entries; Phase 4's own commit message says 22; this plan's original
   "21" was a stale pre-work estimate, left as written per plans-are-historical).
4. **Phase 5's sweep evidence unrecorded** — the phase commit is docs-only with an empty
   body; the checklist's "sweep output recorded" and zero-skip grep evidence exist nowhere,
   so the brief's "zero skipped assertions" claim was unevidenced.

**Remediation shape.**
- `.github/workflows/compat.yml`: wrap both gated `if:` expressions as
  `${{ !cancelled() && (schedule || label || needs.changes.outputs.spark == 'true') }}`
  so a `changes`-job failure can never silently skip the nightly legs (finding 2).
- `docs/plans/20260719-production-readiness.md`: correct the brief's ledger count 24 → 22
  with the 20+2 breakdown (finding 3).
- Re-run the Phase 5 sweep against the live worktree-bound Spark server and record the
  tails + zero-skip grep below (finding 4).
- Open the tracking PR from `worktree-production` → `main` and record whether the
  `changes` job fires `spark-parity` + `type-property-spark` on it via `gh pr checks`
  (finding 1; the W4 commits touch Spark-relevant paths, so the gate must fire).

**Critical files.** `.github/workflows/compat.yml`, `docs/plans/20260719-production-readiness.md`, this file.

**Commit.** `fix(spark): harden per-PR CI gate against changes-job failure; correct + evidence the D1 brief (W4 review remediation)`

#### Phase 7 evidence

(recorded at execution time)

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- Spark twin of the generative `maintenance_conformance` gate — Phase 5 records the gap table here; building it is post-v0.5.

### Phase 5 conformance-gap table

The entire `crates/smelt-cli/tests/maintenance_conformance/` harness is
`#![cfg(feature = "duckdb")]` (see `main.rs`) — it drives the deterministic-
seeded `ModelRecipe` pool and the `s_tracker` multiset-equivalence oracle
exclusively against the DuckDB backend. There is no Spark-feature-gated
variant of this harness at all, so every leg below is uncovered on Spark by
construction, not by an individual gap. What Spark *does* have is a set of
hand-authored fixed-recipe dual-target parity tests exercising one technique
each (`merge_parity.rs`, `incremental_parity.rs`, `lowering_parity.rs`,
`schema_evolution_parity.rs`, `materialization_parity.rs`, `seed_parity.rs`,
`cross_engine_parity.rs`, `cross_engine_types_parity.rs`) — those give
per-technique smoke coverage but never run the generative recipe pool or the
S-restricted equivalence oracle Spark-side.

| Leg (`maintenance_conformance/*.rs`) | Covered on Spark? | Why not |
|---|---|---|
| `append_only_partition_pool_upholds_equivalence` | No | Generative recipe pool + `s_tracker` oracle only wired to the DuckDB backend |
| `admission_rate_stays_above_floor` | No | Same — admission-rate statistics computed over the DuckDB-only pool |
| `mutable_pool_settles_to_full_refresh` | Partial (smoke only) | `incremental_parity.rs` exercises full-refresh-vs-incremental convergence for fixed recipes; the generative mutable-pool sweep is DuckDB-only |
| `keyed_pool_upholds_end_state_equivalence` | Partial (smoke only) | `merge_parity.rs` covers `KeyedFold`/keyed merge for fixed recipes; no generative keyed-pool sweep on Spark |
| `retained_departed_keys_adjusts_the_oracle` | No | Oracle-adjustment logic is a DuckDB-harness-only concept (drives the comparison oracle, not the backend under test) |
| `redelivery_of_processed_window_is_idempotent` | Partial (smoke only) | `incremental_parity.rs` has fixed re-run idempotency cases; no generative redelivery sweep |
| `full_refresh_interleave_resets_state_correctly` | Partial (smoke only) | `materialization_parity.rs` covers fixed full-refresh/incremental interleave; not generative |
| `boundary_rows_within_reach_are_reflected` | No | Boundary/lookback-window generative sweep is DuckDB-only |
| `change_feed_source_admits_recompute_only` | No | No Spark change-feed source fixture exists |
| `feed_declared_source_upholds_equivalence_via_recompute` | No | Same — feed-declared-source path untested on Spark |
| `column_add_between_runs_recovers_equivalence` | Partial (smoke only) | `schema_evolution_parity.rs` covers fixed column-add cases; not generative |
| `skeleton_position_add_is_refused_or_recomputed_never_corrupted` | No | Skeleton-position-add refusal path has no Spark fixture |
| `composed_keyed_pool_upholds_equivalence` | No | Composed multi-cell keyed pool generative sweep is DuckDB-only |
| `composed_keyed_admission_rate_stays_above_floor` | No | Same |
| `delta_restriction_admission_rate_stays_above_floor` | No | Delta-restriction admission generative sweep is DuckDB-only |
| `chain_since_upstream_dirty_set_suffices` / `diamond_propagation_suffices` / `include_upstreams_resolved_slices_suffice` / `upstream_payload_in_downstream_skeleton_position` / `keyed_grain_node_excluded_from_generated_graph` (`dags.rs`) | No | DAG-propagation generative harness is DuckDB-only |
| `window_order_permutations_converge` / `probe_skips_are_counted_never_silent` (`probes.rs`) | No | Probe harness is DuckDB-only |
| `pinned_recipes_reproduce_catalogue_coverage` / `hazard_schedules_are_pinned` (`pinned.rs`) | No | Pinned-recipe catalogue is DuckDB-only |
| `oracle_flags_a_seeded_divergence` (`harness_self_check.rs`) | N/A | Harness self-test, not a backend-conformance leg |

**Disposition.** Building a Spark-native twin of the generative harness
(recipe pool + `s_tracker` oracle retargeted at the Spark backend, or a
dual-execution mode of the existing harness) is the single largest Spark
conformance gap remaining post-v0.5. It is out of scope for this plan (see
Explicitly deferred) and is the natural next backlog item once the
supported-vs-beta label decision (Phase 6) lands.

## Verification

How to confirm the spec is satisfied at the end:
- Live-server sweep: `cargo test -p smelt-backend-spark --quiet` and `cargo test -p smelt-cli --features smelt-cli/spark --quiet` green with `SPARK_CONNECT_URL` set (no skips).
- A PR commit touching `crates/smelt-backend-spark/` triggers `spark-parity` + `type-property-spark` (`gh pr checks`).
- Interpolated authenticated URL fixture (`cross_engine_parity.rs::spark_target_via_interpolated_url`) green.
- `bash .claude/scripts/verify-phase.sh`
- `/smelt:validate multi_backend` reports zero drift on the sections this plan owns.

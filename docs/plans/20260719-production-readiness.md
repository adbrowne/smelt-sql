# Master plan: Production readiness — the v0.5 release programme

**Date**: 2026-07-19
**Design basis**: [`docs/research/20260719-production-release-review.md`](../research/20260719-production-release-review.md)
**Specs**: per sub-plan (each sub-plan's first phase is its spec diff)
**Tracking PR / branch**: `worktree-production`
**Docs**: code+docs

This is a **registry-driven master** (no probe rows). The autonomy loop's unified-priority
dispatch works the "Spawned sub-plans" registry below top-to-bottom: the first row whose
Status is not `done` and whose plan file has a `pending` phase runs next, one phase per
iteration, via the `/smelt:implement` per-phase routine. When no row is ready the loop emits
`<<MASTER_EXHAUSTED>>` and a human scaffolds the next item from the scaffolding queue.

## Context

The production-release review concluded that what blocks a first production release is
**operability, not correctness**: secrets, parallel execution, retry/resume, state locking,
onboarding, declarative tests, release plumbing, and deployment docs. The correctness
infrastructure (equivalence-invariant conformance gate, differential type oracle, parser
conformance, fail-loud ratchets) is already the strongest part of the project and is treated
as a fixed regression net every sub-plan must keep green. Positioning: **v0.5, DuckDB-first**,
with Spark promoted toward first-class by W4 (final supported-vs-beta label call is Andrew's,
made on W4's evidence brief). 1.0 is reserved for environments + fingerprint reuse.

## Decisions (from the review's "Decisions needed", proposed 2026-07-19)

| # | Question | Decision |
|---|----------|----------|
| D1 | Spark first-class or beta? | **Work it now, decide on evidence.** W4 promotes Spark CI to per-PR (paths-gated), re-verifies the divergence ledger against a live server, and sweeps the dual-target parity suite. The supported-vs-beta label is decided by Andrew on W4's closing evidence brief. |
| D2 | Does composed-axes (PR #163) land before the release cut? | **Yes.** Only G2 (docs sweep + drift report) is pending; its loop finishes it. Merge #163 (and #164, fully done) into `main`, then rebase `worktree-production` before launching this loop — W2 rewrites `smelt-runtime/src/execute.rs`, which #163 touched heavily. Pre-flight, human-gated. |
| D3 | Declarative tests: dbt-compatible or smelt-native? | **Smelt-native semantics, dbt-familiar vocabulary.** `not_null`/`unique`/`accepted_values`/`relationships` names, but each test first consults the derived properties: already-proven tests compile to a compile-time verdict ("proven — no scan emitted"); only unproven tests lower to SQL scans via the existing `smelt check` machinery. This is the "derive, don't declare" differentiator. Spec: `docs/specs/data_tests.md` (new, W3 phase 1). |
| D4 | Environments in v0.5? | **Minimal per-target state partitioning only** (W2): state keyed by target so switching targets cannot cross-contaminate, layout covered by the versioned state schema. Promotion/diffing/virtual environments stay post-0.5 (1.0 scope). |
| D5 | quality-grind-t2 Phase 9 | **Resolved by events** — decided and executed on `worktree-roadmap_todo` (Phase 9 done 2026-07-19, 9(b) dropped per human decision). Merge PR #164. |

### D1 evidence brief (W4 closing, for Andrew's supported-vs-beta call)

- **CI.** `spark-parity` + `type-property-spark` now run per-PR, gated on Spark-relevant changed
  paths (`.github/workflows/compat.yml`'s `changes` job); full Spark job set (incl.
  `spark-integration`) still runs nightly and on the `run-docker-tests` label.
- **Secrets.** `connect_url` supports `${ENV_VAR}` interpolation (fail-loud on unset), closing
  the plaintext-token gap.
- **Divergence ledger.** All 24 `spark_type` entries in `divergences.rs` re-verified against a
  live Spark Connect server; stale entries corrected (e.g. `SIGN` is always `Double` on Spark,
  not argument-typed); confirmed by a 1000-case property soak with zero new unregistered
  divergences.
- **Dual-target parity sweep.** Full-refresh/view/ephemeral models and the
  `batched`/`keyed`/`versioned` maintenance legs green on both backends, zero skipped
  assertions.
- **Gap.** The generative `maintenance_conformance` harness (recipe pool + `s_tracker` oracle)
  has no Spark twin — Spark's per-technique coverage is fixed-recipe smoke tests only, not the
  generative sweep. Full gap table in `docs/plans/20260719-prod-w4-spark.md` Phase 5. Building
  the twin is the largest remaining Spark gap, seeded as post-v0.5 backlog.
- **Open question for Andrew: supported or beta for v0.5?**

## Pre-flight (human-gated — NOT loop work)

1. Let the PR #163 loop finish G2, then merge PR #163 (`spec-incremental-models-consolidation`).
2. Merge PR #164 (`worktree-roadmap_todo`) — all t1/t2/t3 phases done.
3. Rebase/merge `worktree-production` on post-merge `main` (W2 conflicts with #163's
   `execute.rs` changes otherwise).
4. Export into the loop env: `DUCKDB_LIB_DIR` + `LD_LIBRARY_PATH` (always), and for W4's
   Spark phases a live Delta-enabled Spark Connect server
   (`bash scripts/spark-up.sh` + `source scripts/spark-env.sh`). W4 phases must emit
   `<<PHASE_BLOCKED>>` when `SPARK_CONNECT_URL` is unset — never skip green.
5. Open a tracking PR from `worktree-production` once the first phase lands.

## Execution model

- One phase per loop iteration, `/smelt:implement` conventions: implementer subagent →
  reviewer subagent → `bash .claude/scripts/verify-phase.sh` → atomic commit + push.
- Registry order is dependency order: W1 (small, hardening) → W2 (operability substrate;
  env interpolation is a W4 dependency) → W4 (Spark) → W3 (adoption surface) → W5 (release
  engineering) → W6 (docs, documents W2/W3 surfaces — blocked-not-speculative if a surface
  hasn't landed) → W7 (`smelt bakeoff`; builds on W2's `ExecuteRequest`/per-target state
  rewrite, so it must not start before W2 is done — its execution prompt enforces this) →
  W8 (composed-axes follow-up debt; needs only the D2 pre-flight merge, registered last as
  lowest-priority).
- Standing gates every phase keeps green: `execute_parity`, `statement_parity`,
  `maintenance_conformance`, `walk_coverage`, the hardening/census/registry ratchets, and the
  parser/type conformance suites. Never lowered without a reviewer sign-off note.
- Blocked phases are recorded in the sub-plan's "Blocked phases" section and skipped —
  never stop-the-line.

## Spawned sub-plans

| Sub-plan | What it delivers | Status |
|----------|------------------|--------|
| [`docs/plans/20260719-prod-w1-fail-loud.md`](20260719-prod-w1-fail-loud.md) | **W1 Fail-loud closure + exit codes** — materialized-view fallback audit/hard-error pin, the 2 `error`-classified Unknown census sites emit real diagnostics, exit-code contract (0/1/2) standardized + documented. | done (2026-07-19) |
| [`docs/plans/20260719-prod-w2-operability.md`](20260719-prod-w2-operability.md) | **W2 Operability** — `${ENV_VAR}` interpolation in `smelt.yml` (fail-loud), state locking + versioned state schema + atomic writes, per-target state partitioning (D4), DAG-parallel execution (`--jobs`), bounded retry for transient errors, `--resume`, run-report artifact + structured logs. | pending |
| [`docs/plans/20260719-prod-w4-spark.md`](20260719-prod-w4-spark.md) | **W4 Spark first-class push** (D1) — connect_url secrets/TLS via W2 interpolation, per-PR paths-gated Spark CI, divergence-ledger re-verification against live Spark, dual-target parity sweep with catalogued gaps, docs + supported-vs-beta evidence brief. **Needs live Spark Connect server in loop env.** | done (2026-07-20) |
| [`docs/plans/20260719-prod-w3-adoption.md`](20260719-prod-w3-adoption.md) | **W3 Adoption surface** — `smelt init`, declarative column tests per D3 (proven-property short-circuit + SQL-scan lowering), `smelt list`/`smelt clean`, failure-summary UX. New spec `docs/specs/data_tests.md`. | pending |
| [`docs/plans/20260719-prod-w5-release-eng.md`](20260719-prod-w5-release-eng.md) | **W5 Release engineering** — UI hardening (localhost-default bind, CORS), CHANGELOG + RELEASING checklist, SECURITY.md, macOS-Intel wheel claim fix, Docker image, Homebrew formula, crates-publishing decision brief + `publish = false` markers. | pending |
| [`docs/plans/20260719-prod-w6-docs.md`](20260719-prod-w6-docs.md) | **W6 Production docs** — deployment guide, orchestration (cron/Airflow) guide, state & recovery reference, per-command CLI reference, getting-started refresh around `smelt init`. Documents W2/W3 surfaces; phases block rather than speculate. | pending |
| [`docs/plans/20260719-prod-w7-bakeoff.md`](20260719-prod-w7-bakeoff.md) | **W7 `smelt bakeoff`** — un-defers ROADMAP §10: wire the choice ladder into the runtime (frontmatter `technique:` pins honoured at execution), `ExecuteRequest.technique_overrides` + scratch-as-synthetic-target seam, the measurement CLI over replayed real-data windows, emit-only `--pin`. Decisions B1–B4 recorded in the sub-plan. **Runs after W2** (builds on per-target state + the rewritten `execute.rs`); registered last so the loop reaches it post-W6. | pending |
| [`docs/plans/20260719-prod-w8-composed-axes-followups.md`](20260719-prod-w8-composed-axes-followups.md) | **W8 Composed-axes follow-ups** — the deferred-item sweep from `20260715-composed-axes-conditional-maintenance.md`: `batched:` sub-block retirement (top-level `safety_overrides:`, fix-it refusal, pre-cut spelling rename), the generative suppressed-MERGE conformance leg (the C4 deferred item), and the recursive composed-driving-source case in `build_forward_graph` (its decision 9). Remaining source-plan deferrals are recorded there with tracked homes. **Requires PR #163 merged (D2 pre-flight)**; independent of W2–W7, registered last as lower-priority debt. | pending |

## Scaffolding queue (human-gated — NOT registered until scaffolded)

- **Release cut + blog series**: version bump to 0.5.0, release notes, and the "proofs, not
  vibes" blog series (posts 1–3 first) with runnable `examples/` companions. Andrew-led;
  the loop does not write launch prose.
- **Maintenance demo rewrite** (`docs/handoffs/2026-07-10-emit-unification-and-demo.md`) —
  Andrew wants involvement; stays out of the loop.
- **Post-0.5 (1.0 scope)**: environments/promotion + output-fingerprint reuse, planner
  L2/L3, SCD-2 (`versioned_models.md` shape-profile composition), native IVM delegation,
  Python-model parity, dbt migration guide.

## Deferred / explicitly out of scope for v0.5

Same list as the review's "Deliberate scope cuts": virtual environments, planner L2/L3 +
user-authored rules, versioned models / SCD-2, native IVM materialized views, Python-model
PyO3 parity, metrics DSL, dbt migration guide.

## Verification (programme level)

- All seven sub-plans `done`; `bash .claude/scripts/verify-phase.sh` green on the tip.
- `cargo test -p smelt-cli --test maintenance_conformance` and the nightly 200-case soak
  stay green throughout.
- W4's evidence brief exists and D1's label call is recorded here.
- `/smelt:validate` drift reports clean for every spec a sub-plan touched
  (`smelt_yml`, state/run-state, `multi_backend`, `data_tests`, CLI, `diagnostics`,
  `maintenance_plan`).
- A tagged `v0.5.0-rc` build passes the release workflow (wheels + VSIX + Docker) — the tag
  itself is human-gated.

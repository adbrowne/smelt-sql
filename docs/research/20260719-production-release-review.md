# Production-release review: where smelt is, what blocks v1, and the story to tell

Date: 2026-07-19
Inputs: docs/ROADMAP.md, all 39 specs (Known Divergences sections), docs/plans/ registry state,
CI workflows + ratchet baselines, divergence/known-unknown ledgers, crate survey
(CLI/runtime/state/UI), open PR #163, active worktrees, last-quarter git history.

## Crux

smelt's core thesis — **derived incremental maintenance with a machine-checked equivalence
invariant** (`incremental_state(S) == full_refresh(inputs ∈ S)`) — is implemented, generatively
tested on every PR, and real on DuckDB. The correctness infrastructure (ratchets, differential
oracles, conformance gates) is stronger than most shipped data tools. **What blocks a production
release is not correctness — it is operability**: secrets, parallel execution, retry/resume, state
locking, onboarding (`smelt init`), and declarative data tests. These are boring, bounded, and
mostly independent — ideal autonomy-loop fodder. The release should cut scope to what the
conformance gates actually prove (DuckDB-first, incremental partition+key grains), label Spark
beta, and explicitly defer the specced-but-unbuilt headliners (virtual environments, planner
L2/L3, SCD-2, native IVM).

## State of the union

- **Scale/velocity**: ~322K lines of Rust, 704 files, ~4,800 tests across 350 test files.
  ~1,480 commits in the last quarter (~15/day), ~95% agent-executed under spec-gated plans.
  Workspace version 0.3.2.
- **Programme state**: the refresh-as-maintenance-plan master (20260704-model-updates) completed
  2026-07-13, capped by the standing generative gate
  (`cargo test -p smelt-cli --test maintenance_conformance` + nightly 200-case soak).
- **In-flight (important)**:
  - **PR #163** (`spec-incremental-models-consolidation`, +28K/−2.9K, 246 files) started as a
    four-spec consolidation into `incremental_models.md` but is now the **live autonomy-loop
    branch** running the 34-phase composed-axes + conditional-maintenance plan
    (`docs/plans/20260715-composed-axes-conditional-maintenance.md`). It has already landed: the
    three keyed-temporal-locality routes (closing the "key_recurrence parses but is unconsumed"
    gap), `grain: key_per_partition` fail-loud, change-suppressed MERGE / conditional
    DELETE+INSERT, observed-output-delta recording + propagation, delta-restricted enrichment
    recompute, the Relation Contract surface, and the web-analytics composed-dedupe tutorial
    stage. Roughly half of last month's "known gaps" are being closed here.
  - **worktree-roadmap_todo** runs a second loop on the quality-grind plans
    (20260718-quality-grind-t1/t2/t3): parser dialect fixes, proptest generator expansion
    (FILTER, ordered-set aggs, arrays, structs), planner→smelt-logical dedup, cold-Salsa
    2000-model perf profiling. t2 Phase 9 is blocked awaiting a human decision.
  - `docs/handoffs/2026-07-10` queue: emit-unification plan, maintenance demo rewrite (Andrew
    wants involvement).

## What is genuinely strong (the "no BS in, no BS out" story)

1. **The equivalence invariant + generative conformance gate** — incremental models are derived
   from an algebraic ladder, never declared via a `strategy:` knob; proptest-generated recipes run
   through the real `execute_project` pipeline against a full-refresh oracle (bidirectional
   `EXCEPT ALL`), per-PR, with a harness self-check that corrupts a row to prove the oracle can
   fail.
2. **Differential type oracle** — inferred types compared exactly (decimal precision/scale,
   integer width) against a real DuckDB; 18 registered divergences, only 1 KnownBug; every
   `Unknown` site enumerated (95, 2 flagged `error`); value-based nullability soundness.
3. **Parser conformance at zero** — dual-direction DuckDB differential (accept + print-back
   fidelity), `duckdb_seed_gaps 0`, shrink-only ledgers (330-entry external-corpus backlog is
   catalogued, not hidden).
4. **Property composition walk** — all composition-relevant verdicts from one shared bottom-up
   fold; fail-closed proofs, world-facts only widen.
5. **Fail-loud discipline as CI ratchets** — unwrap/expect budget (~62/~112 and shrink-only),
   println gate, Unknown census, registry-migration ratchet (30 legacy-typed functions left).
6. **Real release plumbing already exists** — tag-triggered PyPI wheels (linux x86_64/aarch64,
   macOS aarch64, windows), VS Marketplace + Open VSX, nightly TestPyPI pre-releases, version-check
   job.

## Production blockers (ranked, from the operator lens)

| # | Blocker | Where | Size |
|---|---------|-------|------|
| 1 | Secrets: no env-var interpolation in `smelt.yml`; Spark `connect_url` plaintext, no auth/TLS | smelt-core config | S–M |
| 2 | Sequential execution: models run in a `for` loop, no DAG parallelism | `smelt-runtime/src/execute.rs` | M |
| 3 | No retry / no `--resume` from partial failure (state tracks intervals but isn't used to resume) | smelt-runtime | M |
| 4 | State store: plain JSON, no locking (single-process assumption), no backup/migration versioning | `smelt-state/src/file_store.rs` | M |
| 5 | `smelt init` referenced in error hints but not implemented — no onboarding path | `smelt-cli/src/errors.rs:9` | S |
| 6 | No declarative column tests (`not_null`/`unique`/`accepted_values`/`relationships`) — custom-SQL `smelt check` only | smelt-cli/check | M |
| 7 | Materialized-view **silent fallback to plain table** (warning only) — violates fail-loud; must hard-error until a backend advertises IVM | backend emit path | S |
| 8 | Environment isolation thin: single `.smelt/` regardless of target; no promotion/diffing | smelt-state | M–L |
| 9 | UI: permissive CORS, no auth, `--host 0.0.0.0` possible | `smelt-ui/src/server.rs` | S |
| 10 | Ops observability: terse single-line failure, no run-report artifact, no structured logs; docs-site has no production/deployment/orchestration guide | runtime + docs-site | M |

Secondary: exit codes non-standardized; no `smelt clean`/`list`/`state` commands; macOS Intel
wheels claimed in docs but not built; only 3/21 crates published; no CHANGELOG/SECURITY.md;
Spark CI is nightly-gated (a Spark regression can merge green); smelt-ui ~16 tests.

## Deliberate scope cuts for the first release (specced, not built — defer honestly)

- Virtual environments / output-fingerprint reuse (prototype unwired; cross-model column lineage
  doesn't exist).
- Planner L2/L3 + user-authored rules (L1 only today).
- Versioned models / SCD-2 (`versioning:` doesn't parse yet) and snapshot-reconcile keyed executor.
- Native IVM materialized views (no backend advertises it — hence blocker #7's hard error).
- Python-model PyO3/subprocess parity, metrics DSL, dbt migration guide.

## Proposed release shape

**Positioning**: "first production release" = **v0.5, DuckDB-first**. Supported surface: full /
incremental (partition + key grain, including composed once #163 merges) / view / table /
ephemeral, seeds, unit tests + checks, docs generation, LSP + VSCode. Spark ships as **beta**
(verified parity, nightly-gated) unless we promote Spark CI to per-PR. 1.0 is reserved for
environments + fingerprint reuse.

**Workstreams** (each is an autonomy-loop sub-plan; W2–W5 are parallelizable):
- **W0 Land in-flight**: finish composed-axes plan, merge PR #163, close the quality-grind t2
  Phase 9 decision.
- **W1 Fail-loud closure**: materialized-view hard error, the 2 `error`-Unknown sites, exit-code
  standardization.
- **W2 Operability**: secrets/env interpolation, DAG-parallel execution, retry + `--resume`,
  state locking + versioned state schema, run-report artifact.
- **W3 Adoption surface**: `smelt init`, declarative column tests, `smelt list`/`clean`,
  failure-summary UX.
- **W4 Release engineering**: CHANGELOG + SECURITY.md, fix macOS-Intel docs claim, Homebrew tap +
  Docker image, UI auth-or-localhost-only, crates publishing decision.
- **W5 Docs**: production/deployment guide, orchestration (Airflow/cron) guide, state & recovery
  reference, per-command reference pages.
- **W6 CI**: decide Spark per-PR promotion vs beta label; keep conformance soak.

## Blog series proposal — "In BS, out BS" (working theme: proofs, not vibes)

Order optimized for launch impact; 1–3 are the flagship trio, each with a runnable hook.

1. **"Your incremental model is a proof obligation"** — the equivalence invariant, the algebraic
   ladder, and the generative conformance gate (with the corrupt-a-row harness self-check as the
   kicker). The anti-`strategy:`-knob argument vs dbt/SQLMesh.
2. **"We test our type system against a real database, exactly"** — the differential oracle,
   decimal refuse-don't-approximate, value-based nullability soundness, the Unknown census.
3. **"Derive, don't declare"** — the property composition walk; why YAML-declared safety windows
   drift and proven ones can't.
4. **"Jinja was the wrong language"** — the typed meta-language: spread, HOFs, generators, and
   diagnostics that point into the originating expression.
5. **"A SQL parser with zero known gaps (and a ledger for everything else)"** — dual-direction
   differential conformance, ratchets, the external-corpus ledger as honest debt accounting.
6. **"The planner works for you"** — logical/physical separation, `--show-plan`, cube_split as a
   worked example.
7. **"Ratchets as oracles: shipping 1,500 commits a quarter with an autonomy loop"** — the
   methodology post; likely the widest-reaching (AI-engineering audience).
8. **"Two engines, one type system, zero silent coercions"** — Spark↔DuckDB parity, Parquet
   exchange with no copy step, refuse-don't-approximate collation.

Cadence: post 1 lands with the v0.5 announcement; then fortnightly. Each post gets a runnable
`examples/` companion and links the relevant spec — the specs are unusually publishable evidence.

## Decisions needed (Andrew)

1. Scope call: is v0.5-DuckDB-first + Spark-beta the right positioning, or must Spark be
   first-class (→ per-PR Spark CI becomes a blocker)?
2. Does the composed-axes plan (PR #163) finish before the release cut, or does the release
   branch from post-consolidation state?
3. Declarative tests: dbt-compatible YAML shape or smelt-native (derived-property-aware) shape?
4. Environments: ship v0.5 with documented single-target state isolation, or pull minimal
   per-target state partitioning into W2?
5. quality-grind-t2 Phase 9 blocked decision on worktree-roadmap_todo.

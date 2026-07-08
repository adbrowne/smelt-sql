# 08 — Code placement: where the maintenance plan lives

- **Date**: 2026-07-06
- **Status**: research (part 8 of [the refresh-as-maintenance-plan series](README.md))
- **Related docs**: [01-framework.md](01-framework.md) (the plan model this doc places),
  [02-loop-findings.md](02-loop-findings.md) (empirical inventory of today's behaviour),
  [03-design-forks.md](03-design-forks.md) (decisions several placements depend on),
  [06-proof-obligations.md](06-proof-obligations.md) (what the graduated harness must keep proving)
- **Related specs**: `docs/specs/architecture.md` (layered single-ownership, run pipeline parity,
  Salsa purity, fail-loud), `docs/specs/model_maintenance.md`, `docs/specs/model_transforms.md`

The research loop deliberately put its harness in `smelt-cli`'s test tree, because `smelt-cli`
already depended on both `smelt-runtime` (for `execute_project` + a real DuckDB backend) and
`smelt-db` (to stage the Salsa workspace `execute_project` expects), so the harness
(`link_c_harness.rs`) required zero new dependency-graph plumbing there. That reasoning does not
hold for the plan's own DuckDB "equivalence oracle" tests (`tracer_evolution.rs`,
`tracer_maintenance.rs`, `tracer_propagation.rs`): they call only `smelt_logical::maintenance::*`
against a raw DuckDB connection, never `execute_project` or `link_c_harness`, and `smelt-runtime`
already carries both `smelt-logical` and `duckdb` as dependencies — so they live in
`crates/smelt-runtime/tests/` (moved 2026-07-08; a genuine `smelt-runtime → smelt-db` dev-dependency
cycle was never the constraint for *them*). `link_c_harness.rs` itself, and the cells built on it
(`g_12` and friends), stay in `smelt-cli`'s test tree — moving them is a separate call, since a
future `smelt-maintenance-testkit` graduation crate (§3, M3) shared as a dev-dependency of *both*
`smelt-db`'s and `smelt-runtime`'s test trees is where a real cycle risk would first appear. This
doc surveys where the relevant machinery actually lives today, then proposes the production home
for each piece of the framework in [01-framework.md](01-framework.md), constrained by the
architectural invariants in `CLAUDE.md`.

---

## 1. Survey: where things live today

### 1.1 Analysis facts (all in `smelt-logical`, mostly already pure)

The pure classifiers the maintenance plan needs as inputs are already concentrated in
`crates/smelt-logical/src/analysis/`:

| module | fact it derives | status |
|---|---|---|
| `source_bounds.rs` (`derive_model_bounds`, :265) | per-source read reach `(col, before, after)` — the scan bound | live (consumed by `smelt-runtime/src/compile.rs::build_source_bound_map` and `smelt-planner`); FIX-1 made it column-aware |
| `input_delta.rs` (`input_delta_discovery`, :88) | per-source delta kind (`ChangeFeed`/`WindowForward`/…) | **dormant** — zero production call sites, guarded by `crates/smelt-logical/tests/input_delta_discovery_dead_code_tripwire.rs` (ledger FIX-2) |
| `join_shape.rs` (`JoinContext` :37, `fan_out` :63) | join cardinality proof (one-to-one vs fan-out) | **dormant** — no production caller; single-column unique keys only (ledger G-10) |
| `discriminants.rs` (`combiner_discriminants`) | combiner algebra class (additive / idempotent / holistic) | live but narrow — consumed only by `rules/cumulative.rs:92` and `join_shape`; `rules/incremental.rs` never consults it (ledger G-07) |
| `window_independence.rs` | self-edge ordering verdict (`Ordered` → sequential batches) | live |
| `temporal.rs`, `monotonicity.rs`, `horizon_ceiling.rs` | lookback/lookahead, event-time monotonicity, horizon warning ceiling | live |
| `functional_dependency.rs`, `bounded_domain.rs`, `decomposed_state.rs` | declared FD widening, space budget, hidden decomposed state | live (declaration widenings from `model_properties.md`) |

Rule-shaped analysis sits beside them in `crates/smelt-logical/src/rules/` — `incremental.rs`
(batched admission) and `cumulative.rs` (`classify_cumulative` :233, `CumulativeClassification`
:201 — keyed admission and combiner selection).

What does **not** exist anywhere yet, and is required by [01-framework.md](01-framework.md) §5–§6:
column-provenance → **mutation-sensitivity partitioning** (the column-group factoring), a
**skeleton-role classifier** (which columns hold membership/grouping/dedup/ordering positions), and
cross-model **payload taint propagation** (today's `nondeterministic_columns` taint in the batched
rule is intra-model only).

### 1.2 Execution (in `smelt-runtime` + `smelt-backend*`)

- `crates/smelt-runtime/src/execute.rs` (1,298 lines) is the single compile+execute pipeline behind
  `execute_project` (run-pipeline-parity rule). Its incremental branch computes the write window
  (the DELETE range must equal exactly what the INSERT writes — :970–:1042) and calls the backend's
  `execute_model_incremental` (:1051).
- `crates/smelt-backend/src/lib.rs::resolve_strategy` (:242) **always returns
  `IncrementalStrategy::DeleteInsert`**; `BatchedConfig.unique_key` is deliberately unused on this
  path (`let _ = unique_key;` :200). This is the mechanism behind the loop's headline finding:
  every `refresh: batched` cell today is recompute-region ([02-loop-findings.md](02-loop-findings.md)).
- The one targeted-write technique that *is* live — keyed's `merge_into`
  (`crates/smelt-backend/src/lib.rs` :294) — is driven by
  `crates/smelt-runtime/src/maintenance_driver.rs::run_windowed_keyed_maintenance` (:96) via the
  `WindowedKeyedRule` trait, implemented by `CumulativeClassification`
  (`crates/smelt-runtime/src/cumulative.rs` :35). The driver is already deliberately mode-agnostic
  ("the reusable classify → step → per-partition pushdown → create-or-merge loop") — it is the
  embryo of the per-cell technique executor.
- `crates/smelt-runtime/src/dimension_horizon_merge.rs` (:38) — the column-scoped re-derivation
  primitive — is **dormant** (no production caller; ledger G-10).
- `crates/smelt-runtime/src/transformer.rs::inject_time_filter` (:299) injects the outer clamp;
  its unqualified-column bug is ledger G-11 / [03-design-forks.md](03-design-forks.md).

### 1.3 State (`smelt-state`)

The "keyed window-ledger" is concretely `crates/smelt-state/src/intervals.rs`: an `IntervalStore`
of per-model `covered_intervals` + `model_hash`, persisted by `file_store.rs` as
`.smelt/intervals.json` (plus per-run manifests in `.smelt/runs/`). Two properties matter for the
generalized ledger:

- It is **frontier-only** (covered time intervals). There is no per-delta identity anywhere — the
  additive grading of the reconciliation ledger ([01-framework.md](01-framework.md) OQ4 design) has
  no substrate yet.
- It is keyed **per model**, not per `(output-region × column-group)`, and lives in a project-local
  JSON file, not in the target warehouse.

`snapshot_store.rs` and `history.rs` already handle warehouse-adjacent state; `ddl_duckdb.rs` /
`ddl_spark.rs` show the crate already knows how to emit backend-specific state DDL.

### 1.4 The two-parallel-paths wart is now dead code

The previously-noted duplicate incremental path in the CLI has *resolved itself into dead code*:
`crates/smelt-cli/src/executor.rs::execute_plan_incremental` (:182) and
`::execute_model_incremental` (:50) have **no callers anywhere in the workspace** (checked by
`rg`; the only `executor::` call left is `validate_sources` from `commands/run_setup.rs:230`).
`commands/run.rs` goes through `smelt_runtime::execute_project` (:215). The wart is no longer "two
live paths that can drift"; it is ~350 lines of unreachable incremental logic (including its own
`inject_time_filter` usage and DELETE-range computation) that still *compiles*, still shows up in
searches, and still looks authoritative. It should be deleted, not migrated (§2.5).

### 1.5 The research harness

All in test targets, tagged `EXPERIMENTAL(property-discovery)` and gated by
`.claude/scripts/property-experimental-gate.sh`:

- `crates/smelt-cli/tests/property_discovery/` — `link_c_harness.rs` (`LinkCProject`,
  `SqlCapturingReporter`, real-`execute_project` staging), `run_schedule.rs` (`ScheduleStep`,
  `MutationProfile` self-check, proptest strategies), `oracle.rs` (`EXCEPT ALL` multiset equality),
  `model_shapes.rs` (the single model-SQL catalogue), plus one file per ledger cell.
- `crates/smelt-db/tests/proptests/maintenance_link_a.rs` (abstract combiner/schedule model),
  `maintenance_link_b.rs` + `maintenance_link_b_composite_key_fan_out.rs` (analyzer-fact vs
  independent-DuckDB-probe diagnostics).
- Permanent, untagged guards: `input_delta_discovery_dead_code_tripwire.rs` (smelt-logical),
  `test_form_b_does_not_leak_bound_to_unrelated_source` (in `source_bounds.rs`'s test module).

---

## 2. Proposed placement

The one-line summary: **plan model and every classifier feeding it → `smelt-logical`; thin Salsa
wrappers → `smelt-db`; technique lowering and the driver loop → `smelt-runtime`; technique
primitives → `smelt-backend*`; the reconciliation ledger → `smelt-state`; nothing new in
`smelt-cli`.** This is not a new architecture — it is the existing layering with the maintenance
plan slotted into the same seams the logical `Plan` already uses.

```
                 smelt-parser ── smelt-types ── smelt-core
                        │             │             │
                        └──────┬──────┴──────┬──────┘
                               │             │
                        smelt-logical   smelt-dialect
                     (analysis/* + NEW │        │
                      maintenance/*:   │   smelt-state
                      MaintenancePlan, │  (NEW reconciliation
                      column groups,   │   ledger beside
                      skeleton roles)  │   intervals.rs)
                          │       │    │        │
                     smelt-db  smelt-planner    │
                    (thin Salsa (rule           │
                     queries +   application)   │
                     LSP diags)      │          │
                          │          │          │
                          └────┬─────┴────┬─────┘
                               │          │
                         smelt-runtime ← smelt-backend ← smelt-backend-{duckdb,spark}
                        (plan→technique   (technique primitives:
                         lowering behind   delete+insert, merge_into,
                         execute_project,  column-scoped merge,
                         ledger ops)       ledger DDL/DML)
                               │
                    smelt-cli / smelt-ui (consumers only)
```

### 2.1 The `MaintenancePlan` model → `smelt-logical` (new `src/maintenance/` module)

A pure data type, sibling of the logical `Plan`:

- `CellKey { column_group, input }`; per cell: the **admitted technique set** (which 2×2 corners
  are legal), the **chosen technique** (defaults per [04-knobs.md](04-knobs.md), overridable), the
  **obligations** (ledger grade, ordering discipline, cascade requirement), and the **traded
  guarantees** (the CONDITIONAL labels from [02-loop-findings.md](02-loop-findings.md), e.g.
  "stale-until-backfill", "cascade-required").
- Derived by a pure function `derive_maintenance_plan(analysis facts × source declarations ×
  declared shape/grain) → MaintenancePlan | refusals`. Its inputs are exactly the §1.1 modules —
  `source_bounds`, `discriminants`, `input_delta`, `join_shape`, `window_independence`,
  `temporal`/`horizon_ceiling` — plus the three classifiers that must be **built new in the same
  directory**: mutation-sensitivity column grouping, skeleton-role extraction, payload taint.
- The dormant classifiers stop being dormant *here*: `input_delta_discovery` and `fan_out` become
  inputs to admission (which techniques a cell may use), not direct drivers of execution. The FIX-2
  tripwire is then retired in favour of a positive test asserting the plan consumes them (§2.7).

**Why `smelt-logical` and not `smelt-planner`:** the layered single-ownership invariant exists
precisely so `smelt-db` can reach shared pure analysis without depending on the planner —
`smelt-db → smelt-planner` has no production path (`cargo tree -p smelt-db -i smelt-planner`), and
surfacing plan refusals as *editor diagnostics* is a first-class goal. Putting the plan in the
planner would either duplicate it or break the invariant.

**Rejected: a new `smelt-maintenance` crate** between `smelt-logical` and its consumers. Nothing
today forces the split: `smelt-logical` already hosts the logical `Plan`, `RuleContext`, and every
classifier this plan consumes, and a new crate adds a boundary exactly where the derivation needs
the tightest coupling (plan derivation calls a dozen sibling analysis modules). Revisit only if
`smelt-logical` compile times or the module's size make the seam worth paying for; the module
boundary (`src/maintenance/`) is designed so extraction stays mechanical.

### 2.2 Derivation entry points → `smelt-db` (Salsa) and `smelt-planner` (rules)

Per the Salsa-purity rule, `smelt-db` gains a thin tracked query — `maintenance_plan(file)` — that
assembles inputs (parsed model, resolved sources, declared properties) and calls the pure
derivation. Consumers:

- `file_diagnostics()` folds in plan refusals and mismatches (declared shape vs derived plan,
  payload-in-skeleton-position — fail-loud, with new diagnostic codes catalogued in
  `docs/specs/diagnostics.md`).
- The LSP gets plan hovers/explain for free through the same query.
- `smelt-planner` calls the *same pure function* during rule application (it already depends on
  `smelt-logical`); the planner's job narrows to *choosing among admitted techniques* and lowering
  to physical steps — "validator, not chooser" stays intact because choice is only over
  proven-interchangeable techniques.

### 2.3 Technique execution → `smelt-runtime` lowering + `smelt-backend*` primitives

- **Primitives** stay/land on the `Backend` trait: `delete_and_insert_transactional` (exists),
  `merge_into` (exists, currently keyed-only), a **column-scoped merge/update** (new — the
  `dimension_horizon_merge` write shape, promoted from the dormant module into a trait-level
  primitive with per-backend implementations), and ledger DDL/DML (§2.4). Capability gating uses
  the existing idiom — trait methods with refusing defaults — rather than a capability-enum matrix;
  a backend that cannot express a primitive causes the *plan* to drop that technique from the
  admitted set at compile time (fail-loud, not runtime surprise).
- **Lowering** lives in `smelt-runtime`, behind `execute_project` (run-pipeline-parity rule keeps
  CLI and UI on one path; `cargo test -p smelt-runtime --test execute_parity` is the standing
  gate). `resolve_strategy`'s constant `DeleteInsert` (:242) is replaced by reading the chosen
  technique off the plan cell; `maintenance_driver.rs`'s classify→step→pushdown→merge loop
  generalizes from "keyed's driver" to "the per-cell technique executor" — it is already written
  mode-agnostically and refuses unsound combiners before any backend call, which is exactly the
  shape the plan executor needs.
- `dimension_horizon_merge` gets wired *only* behind a plan cell that admits column-scoped
  re-derivation — which requires the G-10 composite-key extension and the FIX-2 wiring decision
  ([03-design-forks.md](03-design-forks.md)) to land first. The G-11 clamp-qualification fix is a
  precondition for self-referential models regardless of placement.

### 2.4 The generalized reconciliation ledger → `smelt-state`

`intervals.rs` is the degenerate frontier-only case of the OQ4 ledger design; the generalization
lives beside it as a new `reconciliation.rs`:

- Entries keyed `(output-region × column-group)`, each carrying a per-input processed vector —
  **frontier grade** (a watermark per input; sufficient for idempotent groups and for
  recompute-only groups, and structurally an extension of today's `covered_intervals`) and
  **per-delta grade** (delta identities, required for additive fold cells).
- **Storage split by grade.** Frontier-grade state can stay in the project-local `.smelt/` file
  store (it is small, per-model-ish, and already there). Per-delta-grade state must live **in the
  target warehouse** as smelt-managed state tables — delta-identity volume scales with data, must
  survive independently of the checkout, and must be transactional with the fold that consumes it
  (a JSON file cannot be atomically committed with a DuckDB/Spark MERGE). `smelt-state` already has
  the backend-DDL seam (`ddl_duckdb.rs`/`ddl_spark.rs`) for exactly this.
- The two ledger *operations* — fold-with-never-fold-twice-check, and reset-on-recompute — are
  runtime concerns: they live in `smelt-runtime` next to the driver, calling `smelt-state` for
  state access and the backend for transactional writes. The ledger *schema* for a model is derived
  from the plan (which cells need which grade), keeping declared surface minimal.

**Rejected: ledger in `smelt-runtime` directly.** The state model/serialization is independent of
execution and already has a crate; `smelt-state` keeps it testable without a backend and shared
with status/history commands.

### 2.5 Delete the dead CLI incremental path

`crates/smelt-cli/src/executor.rs`'s `execute_model_incremental`/`execute_plan_incremental` (and
their helpers) have zero callers (§1.4). Delete them outright — the maintenance-plan work is the
forcing function, because every line of that file that survives is a line a future reader may
mistake for the write-path (it computes its own DELETE ranges, which is precisely the
write-window/filter-range hazard the parity rule exists to prevent). `validate_sources` is the only
live export; keep it (or fold it into `smelt-runtime`'s gate module later — cosmetic). Gate:
`execute_parity` stays green and `cargo test -p smelt-cli` compiles without the module.

### 2.6 Cross-model payload propagation → `smelt-db` workspace queries

Skeleton-role extraction and single-model payload classification are pure (`smelt-logical`,
§2.1). The DAG propagation — "a payload column of `M` consumed in a skeleton position of `N` fails
loud at `N`" — is a workspace-scoped consumer-side check: it belongs with the existing resolution
machinery in `smelt-db` (project-scoped per the project-isolation rule, since payload-ness must not
leak across project boundaries any more than refs do), surfacing as diagnostics on the *consumer*
file with a secondary span pointing at the producer column.

### 2.7 What graduates from the research harness

The Link-C harness is not throwaway after all — [06-proof-obligations.md](06-proof-obligations.md)
§5 makes "emitted maintenance ≡ full refresh over adversarial schedules" a *standing* obligation,
and this harness is the only thing that proves it end-to-end. Graduation plan:

- **Graduate** (`run_schedule.rs`, `oracle.rs`, `link_c_harness.rs`): into a dev-only workspace
  crate `smelt-maintenance-testkit` (publish = false, dev-dependency of `smelt-cli` and — once the
  dep-cycle question is checked — `smelt-runtime`). Rationale for a crate over `tests/common/`:
  three crates' test targets want it (`smelt-cli` Link-C, `smelt-db` Link-A/B, future
  `smelt-runtime` conformance tests), and a named crate makes the parity-style CI gate addressable.
  The `EXPERIMENTAL(property-discovery)` tags come off at graduation; the gate script narrows to
  whatever stays disposable.
- **Keep disposable** (per-cell `g_*.rs`/`sc_*.rs` probe tests): they are evidence for ledger
  cells, not regression tests. Each one that guards a *fixed* production behaviour (FIX-1's SC-1
  re-run) gets distilled into a small permanent test beside the code it guards; the rest stay
  tagged and deletable.
- **`model_shapes.rs`**: its SQL catalogue seeds the plan-conformance fixture set (each shape maps
  to an expected `MaintenancePlan`); the [07-example-catalogue.md](07-example-catalogue.md)
  machine headers are written to be translatable into new shapes here.
- **Tripwires**: `input_delta_discovery_dead_code_tripwire` is *inverted* when §2.1 lands — replaced
  by a test asserting the plan derivation consumes the classifier (the tripwire's failure message
  already points at SC-2; the replacement must carry the same pointer).
- The Link-A abstract model and Link-B clamp-probe stay in `smelt-db`'s proptests — they are
  analyzer-fact tests, correctly placed already.

### 2.8 Migration sketch (coarse, CI-green at every step)

Not a plan (that comes via `/smelt:plan` after the spec); an ordering with dependencies:

1. **M0 — delete dead code.** Remove the caller-less CLI incremental path (§2.5). No design-fork
   dependency. Also the natural moment to fix the two flagged non-fork bugs
   ([03-design-forks.md](03-design-forks.md)): the `BigInt` aggregate-truncation bug and the
   unqualified multi-timeseries filter (the latter may fold into G-11's resolution).
2. **M1 — descriptive plan.** Land `smelt-logical/src/maintenance/` deriving a `MaintenancePlan`
   that *describes* today's behaviour (every batched cell: recompute-region; keyed: fold via
   `WindowedKeyedRule`; traded guarantees from the ledger's CONDITIONAL findings). No execution
   change; conformance test asserts the described technique matches what `execute_project` emits.
   Needs the new column-group/skeleton classifiers in their v0 form.
3. **M2 — surface it.** Salsa query + diagnostics + `smelt explain`-style output (§2.2). Declared
   shape/grain validated against the plan (the [01-framework.md](01-framework.md) §10 check).
4. **M3 — graduate the harness** into `smelt-maintenance-testkit`; wire the Link-C schedule suite
   as a standing CI gate over the shape catalogue (the [06](06-proof-obligations.md) §5 gate).
5. **M4 — generalized ledger, frontier grade** in `smelt-state` (§2.4), subsuming
   `intervals.rs`'s role for plan-managed models.
6. **M5 — first targeted-write cell.** Wire `dimension_horizon_merge` behind plan admission.
   Depends on design forks: G-10 (composite keys), FIX-2 (classifier wiring policy), G-11 (clamp
   qualification — precondition for the self-referential shapes in the same admission family).
7. **M6 — fold-delta cells + per-delta ledger grade + cost-model defaults/overrides**
   ([04-knobs.md](04-knobs.md)), including the offline bake-off harness reusing the testkit's
   run-schedule driver.

---

## 3. Invariant compliance checklist

| invariant | how this placement satisfies it |
|---|---|
| Layered single-ownership | plan model + classifiers in `smelt-logical`; `smelt-db` never touches `smelt-planner`; structural assertion unchanged |
| Salsa purity | `maintenance_plan` query is a thin wrapper over the pure derivation |
| Run pipeline parity | lowering only in `smelt-runtime` behind `execute_project`; M0 deletes the last (dead) rival path; `execute_parity` gates |
| Project isolation | payload-taint DAG check is project-scoped in `smelt-db` |
| Fail-loud | refusals/mismatches are diagnostics with catalogued codes; backend capability gaps drop techniques at plan time, never silently at run time |
| Hardening gates | new production code lands under the existing `unwrap`/`println!`/Unknown budgets; testkit is dev-only |

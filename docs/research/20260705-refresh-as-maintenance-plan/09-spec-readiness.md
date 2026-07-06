# 09 — Spec readiness: what stands between this research and a full spec

- **Date**: 2026-07-06
- **Status**: research (part 9 of this directory; see [`README.md`](README.md))
- **Author**: Andrew (with Claude)

This is the honest gap list. The framework ([`01-framework.md`](01-framework.md)) is
internally settled; the loop ([`02-loop-findings.md`](02-loop-findings.md)) has pinned what
today's smelt actually does; the surface ([`04-knobs.md`](04-knobs.md),
[`05-source-properties.md`](05-source-properties.md)), obligations
([`06-proof-obligations.md`](06-proof-obligations.md)), examples
([`07-example-catalogue.md`](07-example-catalogue.md)) and placement
([`08-code-placement.md`](08-code-placement.md)) are proposed. What remains falls into five
buckets: decisions only Andrew can ratify, machinery that does not exist yet, empirical gaps
worth closing before the spec commits, the concrete spec-diff map, and the sequencing.

---

## 1. Decisions to ratify (blocking; each is argued elsewhere, this is the queue)

**Ratified 2026-07-06 (Andrew):** all decisions 1–11 below are resolved as annotated. The
resolutions are the spec's marching orders.

Design forks from the loop ([`03-design-forks.md`](03-design-forks.md), recommendations
included there) — **all four ratified as recommended:**

1. **F1 (G-11)** — outer output clamp: adopt the always-wrap-in-subquery repair (also closes
   F5, the same-named multi-timeseries ambiguity). *Blocks the spec*: the spec's own documented
   self-referential pattern must execute. **✓ ratified.**
2. **F2 (G-10)** — composite unique keys: spec the declaration composite-valued from day one;
   the `JoinContext` generalization lands with the first `fan_out` consumer. **✓ ratified.**
3. **F3 (FIX-2)** — dormant `input_delta_discovery`: wire only as an input to the
   plan-derivation layer, evolved to per-trigger delta channels; no standalone landing. **✓ ratified.**
4. **F4 (BigInt truncation)** — fix now, ahead of everything; framework-independent live
   silent corruption. **✓ ratified.**

Surface decisions flagged contentious by their own proposals:

5. **Mode-name sugar** ([`04-knobs.md`](04-knobs.md) K1): **✓ resolved — `batched`/`keyed`/`versioned`
   are removed outright, no sugar.** smelt has no users, so there is no compatibility cost to
   preserving the names, and keeping them would imply the old strategy semantics the paper removes.
   `models.md`'s rewrite and `smelt migrate` do a hard cut.
6. **Retention trust default** ([`05-source-properties.md`](05-source-properties.md) P5): **✓ resolved —
   accept as proposed.** Undeclared retention stays *trusted* replayable; the ledger-anomaly probe
   is the blast-radius bound.
7. **Technique-preference granularity** (K2): **✓ resolved — support BOTH.** `maintenance.defaults.prefer`
   sets a per-model default and `maintenance.cells[].prefer` overrides per cell (distinct from the
   hard per-cell `technique:` pin). K2 updated accordingly.
8. **`columns.<c>.contract` grammar ownership** (K3): **✓ resolved — deferred deliberately.** The
   collision with future column `tests:` is real but not blocking; ownership in `models.md` is
   worked out when the shared `columns:` grammar is specced, provided the per-column-contract design
   holds up (it does).
9. **`key_recurrence` subsumption** (P1): **✓ resolved — subsume** into the `mutation_profile` block
   (it is delivery-contract metadata of the same species); the standalone key is dropped.
10. **Backend-derived source facts** (P6/open): **✓ resolved — leave as a Known Divergence.** Whether
    a backend capability (Delta CDF, Iceberg snapshots) *derives* `change_feed` + `delta_identity`
    is a `multi_backend.md` question tracked separately, not gating this spec.
11. **Scan-locality guardrail default** ([`04-knobs.md`](04-knobs.md) K8): **✓ resolved — ship
    `require: partition_local` + `on_violation: error`** (silent full scans impossible by default; no
    users to migrate, so no friction cost to the strict default). **The ceiling is model-side,
    per-consumer** (`maintenance.scan_bounds.per_source.<s>.max_lookback`); the source-side mirror
    (`max_consumer_scan:`, [`05-source-properties.md`](05-source-properties.md) P7) is **deferred** as
    a design option, not initial surface — added only if owner-side governance proves needed.

## 2. Machinery the spec will name that does not exist anywhere yet

From [`06-proof-obligations.md`](06-proof-obligations.md) (its three hardest) plus
[`08-code-placement.md`](08-code-placement.md) §2:

- **Mutation-sensitivity column grouping** — per-column provenance × per-source mutation
  profile, with fail-closed group merging and a `MaintenancePlanDegenerate`-style visibility
  diagnostic. Nothing computes any of it today; every loop cell worked with hand-known groups.
  This is the single largest new derivation and the heart of the plan.
- **Skeleton-role extraction** — which columns sit in membership/grouping/dedup/ordering
  positions, per model. Prerequisite for both the determinism bar (OQ1 rule) and the theorem's
  skeleton-equivalence statement.
- **Cross-model payload propagation** — DAG-level column provenance with a consumer-side
  fail-loud; needs the same provenance machinery plus a workspace fixpoint (project-scoped, per
  the project-isolation rule).
- **The `MaintenancePlan` datatype itself** and its derivation (admission per cell, chosen
  technique, obligations, traded guarantees) — `smelt-logical/src/maintenance/`.
- **The generalized ledger's straddle attribution without key temporal locality** — the one
  part of the §8 ledger design the paper itself flags as proposal-not-property; the spec should
  scope v1 to locality-or-explicit-footprint and name the rest a Known Divergence.
- **Partition-locality derivation + the emitted partition predicate** (`01-framework.md` §5) —
  projecting each cell's `(partition_col, before, after)` reach/footprint triple onto the source and
  output partition columns to decide partition-locality, *and* emitting that partition predicate into
  the maintenance SQL (scan and merge/overwrite target) so the engine prunes. The derivation reuses
  the reach machinery, but the "project onto the partition column and refuse when unbounded" step,
  the K8/P7 guardrail check, and the `MaintenanceScanUnbounded` diagnostic are new. Without it the
  targeted-write cells are correct but silently full-table.
- **Definition-change (schema-evolution) trigger + column-scoped backfill emitter** — detecting
  that a model gained one or more output fields, instantiating the ledger entries `(region,
  new-group)` at `S = ∅` (§8 of [`01-framework.md`](01-framework.md)), and emitting the
  field-backfill in the 2×2's left column: an in-place `UPDATE` when the field is a pure
  function of stored columns (top-left), or a keyed column-scoped `MERGE` when it re-derives
  from upstream (bottom-left — reuses the `dimension_horizon_merge` emitter). The classifier
  must also fail-loud when the added field lands in a **skeleton** position (a grain change,
  not a field-add — [`07-example-catalogue.md`](07-example-catalogue.md) EX-39), reusing the
  skeleton-role extraction above. A field co-sensitive with an existing group still starts at
  `S = ∅` and forms its own catch-up group until its ledger converges with its sibling's
  (group convergence, EX-40). Worked cells: Family G (EX-36–40).

- **Cross-model dependency propagation** (added 2026-07-07; scoped fully in
  [`10-dependency-propagation.md`](10-dependency-propagation.md)) — the graph layer over
  the per-model plan: **forward** (what landed upstream → which partitions of which
  downstream models run, keyed per inbound edge so the right trigger cell runs per
  region) and **backward** (build a model for a specified period *including upstreams* —
  the test/validation-build resolution, where the date arithmetic runs backwards through
  the same edge clamps). Day-granular v0 tracer exists
  (`smelt-logical/src/maintenance/propagate.rs`); grain mapping, self-referential
  unrolling, and column-group-scoped dirt are the named next steps (`10` §§6–8). Relates
  to EX-34's cross-model settledness (this is its scheduling dual).

The spec can (and should) be written with these as normative behavior + admission rules; but
each needs at least a sketch-level derivation story in the spec so it isn't specifying magic.

**Tracer bullet (2026-07-06).** A v0 of the plan datatype and its derivation/emission now
exists in code and discharges the "sketches reviewed once" condition executably:
`crates/smelt-logical/src/maintenance/` (`MaintenancePlan`, the 2×2 corners, trigger
taxonomy incl. definition-change, K8 refusals; pure functions, no wiring into
diagnostics/planning/execution), exercised by
`crates/smelt-logical/tests/maintenance_tracer.rs` (EX-02/07/13/24/36/39/40 corner +
refusal assertions) and `crates/smelt-cli/tests/property_discovery/tracer_maintenance.rs`
(DuckDB equivalence: emitted maintenance ≡ full refresh per trigger, via the loop's
EXCEPT-ALL oracle). Hand-supplied in v0, exactly the deferred machinery above: column
groups, skeleton roles, the fold spec. Derived and consumed: scan bounds, combiner
algebra, the additive-only column-add proof.

## 3. Empirical gaps worth closing before the spec commits (the loop's next backlog)

From [`02-loop-findings.md`](02-loop-findings.md) §8, prioritized in
[`06-proof-obligations.md`](06-proof-obligations.md) §5; lift-ready cells at the end of
[`07-example-catalogue.md`](07-example-catalogue.md):

1. **The `cumulative_aggregate`/`merge_into` MERGE path** — the only live path where a ledger
   obligation can actually be violated today, and entirely unprobed. Highest information per
   probe; should run before the spec asserts anything about targeted-write behavior.
2. **Beyond-horizon lateness through the real path** (only demonstrated abstractly, P0-5).
3. **Change-feed/CDF sources** (the `ChangeFeed` classifier arm has never been exercised).
4. The remainder (multi-arm mutable unions, holistic-over-mutable, ≥3-column composite keys,
   proptest depth on the single-schedule cells) can trail the spec as its verification suite.

None of these block *writing* the spec; 1–2 block *trusting* its targeted-write and horizon
sections.

## 4. The spec-diff map (what changes where, per the spec-first rule)

- **`models.md`** — the refresh axis rewrite: `full | incremental | materialized_view` +
  declared-and-checked `grain:`; the "peers" argument revised per `01` §13's normative conflict
  1; declaration law preserved for shape/grain. Decision 5 (sugar) shapes this diff.
- **`model_maintenance.md`** — gains the plan matrix (cells, corners), the S-indexed theorem
  and faithful-fold conditions, the generalized ledger, and the obligations table from `06`.
  Likely the spec that grows most; **resolved (P11, 2026-07-07)**: split a new
  **`maintenance_plan.md`** carrying the plan matrix *and* the graph layer
  ([`10-dependency-propagation.md`](10-dependency-propagation.md) — forward/backward
  propagation, ratified P1–P11), leaving `model_maintenance.md` as the invariant/ladder spec.
  The graph layer also touches the sources spec (P10 delta interface),
  `models.md` (granularity's role as the propagation grain, P3), `diagnostics.md`
  (keyed/self-referential/unclocked refusal codes), and the CLI docs (P9 surface).
- **`batched_models.md` / `keyed_models.md` / `versioned` material** — demoted from strategy
  specs to *shape profiles* (grain + default plan); their admission matrices re-derived as
  instances of the theorem's failure cases; `nondeterministic_columns` superseded by per-column
  contracts (K3).
- **Sources spec (`smelt_yml.md` / `sources.md`)** — the structured `mutation_profile` block,
  lateness/watermark, composite `unique_key`, retention, delta identity, the source-side scan
  ceiling (`max_consumer_scan:`, P7), and the trust rule (widening trusted / narrowing verified)
  from `05`. The model-side scan-locality guardrail (`maintenance.scan_bounds`, K8) lands with the
  refresh-surface rewrite in `models.md` / `maintenance_plan.md`, and the partition-locality
  property + its emitted predicate land in `model_maintenance.md` / `maintenance_plan.md`.
- **`model_transforms.md`** — the clamp change (F1's subquery wrap) and the new technique
  primitives (column-scoped merge, ledger fold/reset) as transform contracts, plus the
  **schema-evolution field-backfill** primitive (in-place `UPDATE` / keyed column-scoped
  `MERGE` over existing regions, §5 definition-change trigger).
- **`models.md` / `maintenance_plan.md`** — the **definition-change trigger** as a first-class
  plan trigger beside creation/mutation, and a proposed **`on_column_add: backfill |
  leave_null | recompute`** policy knob (species of EX-21's `on_backfill` cascade knob) noted
  for [`04-knobs.md`](04-knobs.md): whether adding a field auto-backfills it column-scoped,
  leaves it `NULL` on already-processed regions, or triggers a whole-region recompute. Held to
  a *mention* here — surfaced fully when the refresh-surface rewrite lands.
- **`architecture.md`** — one new invariant: the maintenance plan is pure data in
  `smelt-logical`, derived by pure functions; consumers (smelt-db diagnostics, smelt-planner
  application, smelt-runtime lowering) never re-derive (per Andrew's rule that plans landing
  architectural invariants must land them in specs/CLAUDE.md).
- **`diagnostics.md`** — the `Maintenance*` diagnostic family from `06` §7.
- **User docs (`docs-site/`)** — refresh-surface pages rewritten; the example catalogue is the
  seed corpus.

## 5. Sequencing (proposed)

1. Andrew ratifies §1 (a single review pass over `03` + the two contentious flags).
2. **F4 then F1 land as ordinary fixes** (each has a red test named in `03`) — independent of
   the spec.
3. Point the property loop at §3's items 1–3 (the catalog rows are written).
4. `/smelt:spec` the §4 diff — `models.md` + new `maintenance_plan.md` + sources spec first;
   shape-profile demotions second.
5. `/smelt:plan` + implement along `08`'s M0–M6 (M0, deleting the dead CLI incremental path,
   can land any time; M1–M2 give the descriptive plan + diagnostics with **no behavior
   change**, which de-risks everything after).

**Definition of ready-to-spec**: §1 items 1–10 decided (✓ 2026-07-06); item 11 (K8 default) decided;
§3 item 1 probed; the §2 sketches reviewed once. Everything else can trail as Known Divergences with
plan links.

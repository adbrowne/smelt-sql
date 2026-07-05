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

Design forks from the loop ([`03-design-forks.md`](03-design-forks.md), recommendations
included there):

1. **F1 (G-11)** — outer output clamp: adopt the always-wrap-in-subquery repair (also closes
   F5, the same-named multi-timeseries ambiguity). *Blocks the spec*: the spec's own documented
   self-referential pattern must execute.
2. **F2 (G-10)** — composite unique keys: spec the declaration composite-valued from day one;
   the `JoinContext` generalization lands with the first `fan_out` consumer.
3. **F3 (FIX-2)** — dormant `input_delta_discovery`: wire only as an input to the
   plan-derivation layer, evolved to per-trigger delta channels; no standalone landing.
4. **F4 (BigInt truncation)** — fix now, ahead of everything; framework-independent live
   silent corruption.

Surface decisions flagged contentious by their own proposals:

5. **Mode-name sugar** ([`04-knobs.md`](04-knobs.md) K1): do `batched`/`keyed` survive as sugar
   for grain declarations, or are they removed outright (as proposed)? This decides the
   migration story for every existing model and the shape of `models.md`'s rewrite.
6. **Retention trust default** ([`05-source-properties.md`](05-source-properties.md) P5):
   undeclared retention is *trusted* replayable — the one deviation from
   conservative-by-default. Accept, or flip to requiring a declaration before any backfill?
7. **Technique-preference granularity** (K2): per-model `maintenance.defaults.prefer` vs
   per-cell only — proposal defers to bake-off experience, but the spec must pick an initial
   grammar.
8. **`columns.<c>.contract` grammar ownership** (K3): the per-column contract key shares the
   `columns:` map with future column `tests:`; one grammar owner needed in `models.md`.
9. **`key_recurrence` subsumption** (P1): fold it into the `mutation_profile` block or keep it
   standalone (leaning subsume; deferred until the locality gate has a consumer).
10. **Backend-derived source facts** (P6/open): may a backend capability (Delta CDF, Iceberg
    snapshots) *derive* `change_feed` + `delta_identity` instead of a declaration? Touches
    `multi_backend.md`; can be deferred to a Known Divergence.

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

The spec can (and should) be written with these as normative behavior + admission rules; but
each needs at least a sketch-level derivation story in the spec so it isn't specifying magic.

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
  Likely the spec that grows most; consider splitting a new **`maintenance_plan.md`** and
  leaving `model_maintenance.md` as the invariant/ladder spec it already is.
- **`batched_models.md` / `keyed_models.md` / `versioned` material** — demoted from strategy
  specs to *shape profiles* (grain + default plan); their admission matrices re-derived as
  instances of the theorem's failure cases; `nondeterministic_columns` superseded by per-column
  contracts (K3).
- **Sources spec (`smelt_yml.md` / `sources.md`)** — the structured `mutation_profile` block,
  lateness/watermark, composite `unique_key`, retention, delta identity, and the trust rule
  (widening trusted / narrowing verified) from `05`.
- **`model_transforms.md`** — the clamp change (F1's subquery wrap) and the new technique
  primitives (column-scoped merge, ledger fold/reset) as transform contracts.
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

**Definition of ready-to-spec**: §1 items 1–8 decided; §3 item 1 probed; the §2 sketches
reviewed once. Everything else can trail as Known Divergences with plan links.

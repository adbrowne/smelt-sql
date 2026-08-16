# Phase 2 claim inventory — `incremental_models.md` lines 448–833 (pre-edit)

Every normative claim (must/refuse/diagnostic/default/definition/ownership/carve-out) in the
redraft range, numbered, with source line ranges. Used by the adversarial verify step (task 7):
each claim is graded preserved / weakened / lost / strengthened against the post-edit text.

## §"The equivalence invariant" (455–516)

1. (455–463) The invariant: every maintained (non-`full`) model upholds `incremental_state(S) ==
   full_refresh(source | input ∈ S)`.
2. (465–470) `S` = source rows/partitions scanned; partition-set form is the clocked
   specialisation; unclocked source's specialisation is stated per shape profile.
3. (472–476) Order/set-determinacy is a corollary for every shape; trivial-but-present for
   partition grain (disjoint combiner).
4. (478–484) Strengthenings, checkable slice-by-slice: per-partition equivalence (partition
   grain) and per-slice equivalence (key grain, needs key temporal locality).
5. (486–489) Strengthenings are not peer contracts; the real per-shape difference is write
   addressing (§"Per-cell write addressing"), not a second invariant; key-addressed shapes
   discharge the same invariant via key-addressed writes.
6. (491–500) Replayability split: full (executable-oracle) equivalence holds only for replayable
   inputs; admission matrix enforces this per column; non-admitted combinations could one day get
   a weaker observer/prefix-consistency contract (Future Extensions), never smuggled in under the
   executable-oracle invariant.
7. (502–511) Two named carve-outs on every admitted keyed model's oracle: retained departed keys
   (snapshot-reconcile) and ordering-key ties (order-monotone overwrite).
8. (512–516) Every `model_properties.md` property is proven in service of this invariant; every
   `model_transforms.md` transform is licensed by preserving it; smelt-driven shapes discharge it
   via the generative equivalence oracle; `refresh: materialized_view` discharges it via the
   engine's native IVM and smelt runs no combiner.

## §"The contract lattice" (520–591)

9. (520–525) The invariant above is the lattice's default point; a relaxation trades bounded,
   checked equivalence for a capability; never ambient — declared, validated, probe-checked,
   always printed by `smelt explain`.
10. (527–535) A lattice point is admissible only as a complete triple single-owned in
    `smelt-logical`: declaration schema, pure oracle transform, probe emitter; the conformance
    gate consumes the oracle transform directly; users pick/parameterise, never define; v1 ships
    exactly two relaxations.
11. (537–555) Frozen horizon (`H`), partition grain only: oracle over `S_H`; `frozen_horizon: H`
    clamps writes by contract, narrowing never widening the derived horizon clamp; a partition
    older than `H` is never revisited; the probe is baseline-comparative (per-partition row-count
    baseline over the frozen band); a frozen-band partition whose count increased, or that is new
    since the baseline, raises `ContractLateArrivalOutsideHorizon` naming the partition, added row
    count, and `H`; the first run only establishes the baseline.
12. (557–569) Deferral (`D`): oracle licenses lag via `∃ S' ⊆ S`; `deferral: D` licenses run
    skipping and work subsumption; the probe is ledger-derived (maintained frontier vs. input
    frontier) and raises `ContractDeferralExceeded`, naming the cell and measured lag, when lag
    exceeds `D`.
13. (571–578) Scheduling: `lag = input_frontier − maintained_frontier`; `0 < lag ≤ D` licenses a
    skip recorded `skipped_deferral`; `lag ≤ 0` or an unresolved frontier always falls through to
    the normal path; skipping is never the fallback and never available past `D`.
14. (578–586) A deferral skip propagates to every selected dependent, recorded
    `skipped_deferral_upstream`; work subsumption is proven from two ledger facts (a prior
    `skipped_deferral` manifest entry, and the current run's write range covering that cell's
    pending window), never inferred from range coverage alone; the covering run's manifest
    records the subsumed window alongside its normal `success` outcome.
15. (588–591) Both points compose with existing shape facts without a new mode; a relaxed cell
    still resolves technique via the same per-cell admission rule, checked against its point's
    restated oracle.

## §"The algebraic maintenance ladder" (593–626)

16. (595–601) What's maintainable is fixed by combiner algebra, not backend feature; ordering
    criterion is invertibility → maintainability; discriminants owned by `model_properties.md`,
    ladder ordering owned here; the invariant holds unconditionally on every rung — only state
    representation/size changes, never value fidelity.
17. (603–605) Rung 1, direct monoid: `SUM`/`COUNT` (`+`, 0), `MIN`/`MAX` (±∞), `BOOL_*`, `BIT_*`.
18. (606–609) Rung 2, decomposed monoid: user value is `π(state)`; `AVG` = (sum, count); variance
    = Welford triple; approx-distinct = HLL register vector; kept in state table, exposed via
    presentation view.
19. (610–613) Rung 3, group: combiner must be invertible (commutative group) when inputs can
    change; non-group monoids cannot be reprocessed without a full refresh.
20. (614–621) Rung 4, opt-in bounded-domain multiset: holistic aggregates (exact
    `MEDIAN`/`PERCENTILE`/`MODE`/quantiles, exact `COUNT(DISTINCT)`, `DISTINCT`-modified) are
    maintained via a per-key value→count multiset (Z-set), signed form making retraction free
    even for `MIN`/`MAX`; opt-in and fail-loud — unbounded-state aggregate refused by default
    unless a bounded-domain budget is supplied; runtime caps the multiset with a full-refresh
    fallback.
21. (623–626) Ladder boundary: rungs 1–4 are smelt-maintained (`merge_into` loop, optional
    presentation view); beyond it is delegated to the engine's native IVM via
    `refresh: materialized_view`.

## §"Decomposed state (rung 2) in keyed models" (628–703)

22. (630–633) Section fixes physical location of rung-2 state for the key grain, licensed column
    families, and invisibility to consumers.
23. (635–641) Physical layout: state columns live in the same stored table, named
    `<output>__<part>`; presented column materialised at merge time via `π`. Rejected
    alternative: a separate `<model>__state` table plus presentation view (dual `ref()`
    resolution, a second relation in every backend's DDL/atomic-swap path, no benefit since `π`
    is a per-row pure function of the same row's state).
24. (643–647) Presentation projection: state columns excluded from public schema (`ref()`
    expansion, `SELECT *`, declared-schema checks, downstream type inference); collision with a
    declared/projected column is fail-loud `KeyedStateColumnCollision`, never a silent rename.
25. (649–655) Wildcard compile-time rewrite to presented columns for a state-bearing model
    (sibling relations keep their own `.*`); explicit refs untouched; a hand-written `__part`
    name is an ordinary unresolved-column diagnostic; an unresolvable wildcard while a
    state-bearing model is in scope fails loud, naming the model and the wildcard.
26. (657–668) State-shape catalogue table: `AVG` (`sum`,`count`); `STDDEV_*`/`VAR_*`
    (`n`,`sx`,`sxx`); `MAX_BY`/`MIN_BY` (`v`,`o`, incumbent wins on tie per §"Ordering ties");
    once-write (`value`,`written`, `COALESCE`-style combiner).
27. (669–674) `AVG`/`STDDEV_*`/`VAR_*` state combiners are commutative monoids, graded
    **additive** in the transactional frontier write, same as `SUM`/`COUNT`.
28. (674–677) `MAX_BY`/`MIN_BY` state combiner keeps the same ordering-key-tie carve-out as its
    rung-1 form.
29. (677–682) Once-write's state combiner is fully order-independent given its provenance proof;
    `MAX_BY`/`MIN_BY` and once-write keep the idempotent grade their rung-1 form already carries.
30. (684–688) Once-write's `π` widens admitted spellings: the raw reduction is never
    fallback-tainted, so fallback/preference can be applied fresh on every read.
31. (690–700) Three once-write sub-cases: fallback-bearing single reduction (`(value,written)`
    over the bare reduction, `π = value` if written else fallback); multi-candidate reduction
    (one pair per candidate, `π` applies declared preference order over written candidates); the
    bare key-derived spelling needs no decomposed state (plain `COALESCE(target, delta)` already
    computes the presented value).
32. (702–703) `smelt explain` renders state columns as internal state, distinct from the public
    schema (§Surface "CLI").

## §"Validator, not chooser" (705–713)

33. (707–713) The machinery validates the declared shape (`refresh:` value + shape-defining
    facts, any check-only `grain:`/`write:` assertion) against derived properties, rejecting
    fail-loud when the SQL cannot uphold it; it never chooses or silently switches the shape or
    addressing; a full refresh is the honest fallback surfaced as a diagnostic, never an
    automatic downgrade; per-cell technique choice among proven-interchangeable techniques
    operates strictly inside this rule — may change freshness, never observable bits at a fixed
    processed-input set.

## §"The plan matrix" (715–793)

34. (717–718) Every maintained model has a maintenance plan: pure data, derived once, consumed
    everywhere; cells keyed by `(output-column-group × trigger × changed-input)`.
35. (720–724) Column groups: factored by shared mutation-sensitivity (`model_properties.md` owns
    the proof and degenerate-collapse rule); creation is shared by every column, mutation is what
    partitions.
36. (726–738) Sensitivity kind carries into the cell: a value-sensitive group's mutation cell may
    be repaired column-scoped (`MERGE`); a membership-sensitive group must be repaired by the
    recompute family (delete+insert, change-suppressed where comparable), never a column-scoped
    merge; a mutable join partner never read in any select item still produces membership
    sensitivity — absence from value-sensitivity sets is not admissibility for cheaper repair;
    the one admissible pruning is a proof: an enrichment join whose skeleton-source closure is
    proven `Closed` over a provably outer join contributes no membership sensitivity.
37. (740–745) Four trigger classes: creation, mutation, definition change, backfill (each
    defined).
38. (747–755) Each trigger pairs with the changed input driving the cell (the model's scope
    maps, surfaced by `smelt explain`); the same column group under the same trigger class can
    derive different write addressing for different changed inputs (§"Per-cell write
    addressing").
39. (757–768) Each cell carries: its 2×2 corner; its technique (open write-pattern registry
    including the repair family); its write mechanism (available-addressings rule or a validated
    `write:` pin); derived scan clamps per source; the partition-locality verdict per source; its
    obligations and traded guarantees (per-column, equivalence contract × settle bound).
40. (770–777) The 2×2: read scope (delta+state vs. full-input) × write scope (targeted vs.
    region-overwrite) → fold-a-delta / read-modify-write region / column-scoped re-derivation /
    recompute-a-region.
41. (779–788) Recompute-a-region is contract-agnostic and unconditionally valid over replayable
    input; the fold corner is contract-specific (needs a combiner algebra); the repair family is
    recompute-a-region's targeted-write refinement — column-scoped re-derivation corner, scoped
    to a finite key slice, inherits recompute-a-region's correctness argument rather than needing
    its own; where interchangeability holds, a region recompute supersedes and resets what folds
    had written; "unconditionally valid" is a correctness claim, not an admission/cost claim —
    holds even for a whole-table region, gated separately by the partition-locality guardrail.
42. (790–793) The plan is derived, never declared; declared facts are validated against it, error
    on mismatch, never a silent flip; `smelt explain` prints every cell, addressing, clamps,
    locality verdicts, the per-column guarantee ledger, and inbound edges.

## §"Per-cell admission" (795–833)

43. (797–798) A technique enters a cell's plan space only when all obligations discharge;
    fail-closed — an unrecognised construct refuses, never defaults.
44. (800–801) Obligation 1, replayable input (recompute family): source re-readable at its
    current processed set; declared posture (`sources.md`).
45. (802–805) Obligation 2, faithful fold (fold family): two independent conditions (source
    posture × combiner algebra) hold; either failure alone refuses the fold family.
46. (806–807) Obligation 3, combiner algebra class: derived, fail-closed; holistic/unrecognised
    combiner leaves only the recompute family.
47. (808–811) Obligation 4, bounded reach: scan bound derived or declared-and-checked; absent
    both, full-input techniques only (`MaintenanceReachNotDerivable` when the trigger requires a
    bound).
48. (812–814) Obligation 5, bounded footprint (targeted writes): write-scope reflection of the
    scan bound is bounded; a trajectory column's unbounded forward footprint fails
    (`MaintenanceUnboundedFootprint`).
49. (815–816) Obligation 6, well-defined groups: mutation-sensitivity partition is computable;
    degenerate collapse is surfaced, never silent.
50. (817–820) Obligation 7, affected-key discovery (repair family only): a changed input's delta
    resolves to a finite output key set, a sound over-approximation admitted; an unresolvable
    delta shape refuses the repair family by name (`MaintenanceRepairKeysNotDiscoverable`).
51. (822–832) Interchangeability: two techniques serve one cell interchangeably iff, at a fixed
    `S`, they produce identical state on row-existence-deciding columns (the `S`-indexed
    refinement, `S` a per-input vector once the plan factors); faithful idempotent columns get
    bit-preserving choice; additive columns get state-preserving-modulo-the-ledger choice (never
    fold a delta already reflected in state — fold-then-recompute is safe, recompute-then-refold
    double-counts); technique choice among interchangeable techniques belongs to the cost model
    or operator (`prefer`/`technique`) and may change only which `S` is reflected (freshness),
    never observable bits at a fixed `S` — staying inside §"Validator, not chooser".

## Diagnostic codes in range

`KeyedStateColumnCollision` (claim 24), `ContractLateArrivalOutsideHorizon` (claim 11),
`ContractDeferralExceeded` (claim 12), `MaintenanceReachNotDerivable` (claim 47),
`MaintenanceUnboundedFootprint` (claim 48), `MaintenanceRepairKeysNotDiscoverable` (claim 50).

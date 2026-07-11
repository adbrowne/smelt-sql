# Proof obligations — what the framework must establish, when, and what failure looks like

- **Date**: 2026-07-06
- **Status**: research (part 6 of [`README.md`](README.md); spec-ready "Obligations" section draft)
- **Depends on**: [`01-framework.md`](01-framework.md) (the theorem, the skeleton/payload split, the
  generalized ledger), [`02-loop-findings.md`](02-loop-findings.md) (what is already empirically
  pinned), [`04-knobs.md`](04-knobs.md) (declared surface), [`05-source-properties.md`](05-source-properties.md)
  (source declarations these obligations verify)

Every obligation below names four things: **what** must hold, **who** establishes it (analyzer
derivation / declared-and-checked assertion / runtime tripwire / offline property test), **when**
(plan time, run time, CI/loop), and **on failure** (the diagnostic family — per the fail-loud
discipline, no obligation ever degrades to a silent fallback or an `Unknown`). Existing diagnostics
are cited by their current names (`diagnostics.md`); proposed ones carry a `Maintenance*` prefix and
are marked *(new)*.

The framing rule, from the framework paper: **a maintenance plan is admitted per
`(column-group × input × technique)` cell**, and each cell's admission is a conjunction of the
obligations in §1. Obligations in §2–§4 are cross-cutting (they hold for the whole plan); §5–§6 are
the meta-obligations that keep the implementation honest against the theory.

---

## 1. Per-cell admission obligations (plan time)

For a cell `(column-group g, input i, technique T)` the planner must discharge all of the following
before `T` enters the cell's plan space. "Enters the plan space" is the load-bearing phrase: per the
framework's identity rule ("what stays singular"), *admissibility* — not the cost model's eventual
pick — drives derived requirements like row identity.

### 1.1 Replayable input (recompute techniques)

- **What**: `i` is re-readable at the current `Sᵢ` — a recompute of any region derives ground truth
  from `i`'s *current* contents. Per the theorem's `S`-vector refinement, this is
  replayable-at-current-`Sᵢ`, never replayable-at-a-past-`S` (counterfactual for a real source).
- **Who/when**: declared-and-checked at plan time — read from the source's declared posture
  (`05-source-properties.md`: a table source is replayable by construction; a *feed-only* source —
  a change feed whose base table is not retained — is not).
- **On failure**: recompute-family techniques leave the cell's plan space. If **no** technique
  remains for some cell, refuse the model: `MaintenanceNoAdmissibleTechnique` *(new)*, naming the
  cell. (Compare today's `BatchedNotSafe` / `KeyedSnapshotPostureUnsupported`, which are the
  per-mode projections of this refusal.)

### 1.2 Faithful fold (fold-delta techniques)

- **What**: two independent sub-conditions (the theorem keeps them separate — a replayable feed
  with retractions satisfies 1.1 and fails this):
  1. the delta stream of `i` **partitions** the input multiset — no overlaps, no retractions, no
     in-place mutation (every row appears in exactly one delta);
  2. the combiner's fold over any sub-multiset equals the batch aggregate over it.
- **Who/when**: (1) is read from the declared source property (`append_only`, or a change feed
  declared retraction-free) at plan time and re-verified by the §4 tripwires at run time; (2) is
  analyzer-derived from the combiner class (§1.3) — `BOOL_OR`/`MIN`/`MAX`/`SUM`/`COUNT` over a
  partitioning stream are faithful; anything holistic is not.
- **On failure**: fold-delta leaves the cell's plan space. The observer-semantics case (idempotent
  non-invertible combiner over a mutable snapshot — ledger cell G-04's Link-0 prediction, today's
  `KeyedSnapshotSourceUnsupportedColumn`) is the canonical refusal: `fold` would compute
  *min-ever-observed*, `recompute` *min-in-current-snapshot*, unequal at almost every `S`.

### 1.3 Combiner algebra class

- **What**: each aggregate expression in `g` is classified into exactly one of: **additive monoid**
  (non-idempotent: `SUM`, `COUNT`), **idempotent monoid** (`MIN`, `MAX`, `BOOL_OR`),
  **invertible group** (admits retraction by inverse — `SUM` over a feed carrying signed
  retractions), **non-invertible**, **holistic / no bounded state** (`MEDIAN`, exact
  `COUNT(DISTINCT)`, `MODE`, percentiles). The class decides fold admissibility (§1.2), ledger
  grading (§3.1), and retraction handling.
- **Who/when**: analyzer-derived at plan time — `combiner_discriminants`
  (`crates/smelt-logical/src/analysis/discriminants.rs`) exists and fails **closed** (unmatched
  functions and all exact-`DISTINCT` route to `holistic_or_unknown()`; ledger cell G-07 verified
  both the classification and the fail-closed default).
- **On failure**: fail-closed is the failure mode — an unrecognized combiner is holistic, so only
  recompute-family techniques remain. Today's `KeyedUnknownCombiner` is the surfaced form when that
  leaves keyed with nothing. **Gap to discharge** (G-07 finding): the batched rule
  (`rules/incremental.rs`) never consults `discriminants` at all — sound today only because batched
  is unconditionally recompute-region; the moment a fold path exists this wiring becomes mandatory,
  not optional.

### 1.4 Bounded reach (the read clamp)

- **What**: for each `(g, i)` the scan bound `(clock_column, before, after)` — how much of `i` a
  region's recompute must read — is either derived or declared, and if declared, **checked** against
  the SQL (declared-bound-admitted-only-checked; a declaration is a validator, never a chooser).
- **Who/when**: analyzer-derived at plan time — Form-A/B extraction in
  `source_bounds::derive_model_bounds`, now column-aware after FIX-1
  (`lhs_column_is_partition_col`). Known residual gaps, empirically pinned: attribution is
  column-*name*-scoped, not alias/source-scoped, so same-named partition columns across sources can
  spuriously **widen** a bound — proven safe-not-unsound by `BoundResult::merge`'s max semantics
  (ledger cell SC-1b) — and correlated-`EXISTS` shapes match by textual accident (SC-1). The spec
  must state the derivation's domain precisely so "derived" never silently means "guessed".
- **On failure**: `NotDerivable` ⇒ the cell has no bounded read; full-input techniques only. If the
  model's mode requires a bound (window-forward batched), refuse: today's `BatchedNotSafe` family;
  generalized `MaintenanceReachNotDerivable` *(new)*, naming the source and the unmatched predicate
  shape.

### 1.5 Bounded footprint (the write target — property A)

- **What**: for a *targeted-write* technique, an input delta of `i` maps to a **bounded** set of
  output addresses in `g` (the footprint map, the reflection of the scan bound). A trajectory
  column's unbounded forward footprint (running total under late data) fails this — the ladder
  rescues the read (B), never the write (A).
- **Who/when**: analyzer-derived at plan time from the same bound triple, reflected
  (scan `(before=0, after=7d)` ⇒ footprint `(before=7d, after=0)`), plus the join-shape cardinality
  proof for broadcast footprints (`join_shape::fan_out` — currently dormant and single-column-only,
  ledger cell G-10; see [`03-design-forks.md`](03-design-forks.md)).
- **On failure**: targeted-write techniques leave the cell's plan space; region-overwrite remains.
  For a genuine trajectory grain the honest outcomes are the ordered-cascade condition (G-08) or
  the deferred as-of-run contract — both must be *named*, never silently tolerated:
  `MaintenanceUnboundedFootprint` *(new)* when a targeted write was requested for such a cell.

### 1.6 Mutation-sensitivity partition (column groups are well-defined)

- **What**: the partition of output columns by shared mutation-sensitivity is computable: for every
  output column, which inputs' *post-creation* deltas can change its value — distinguishing
  creation reads (a reference to the row's own immutable skeleton at materialization time) from
  mutation sensitivity. Requires per-column provenance from the SQL **and** each input's mutation
  profile (an immutable-at-creation reference drops out only because the source is append-only).
- **Who/when**: analyzer-derived at plan time. This is the largest *new* derivation the framework
  needs — nothing in the current analyzer computes it (the ledger's cells all worked with
  hand-known groups).
- **On failure**: fail-closed by merging — any column whose provenance cannot be fully resolved
  joins a single conservative group mutation-sensitive to *all* inputs (the plan degenerates to
  today's per-model story: correct, never wrong, just expensive). This is the one place fail-closed
  is a *degradation* rather than a refusal, so it must be **visible**: a
  `MaintenancePlanDegenerate` *(new)* info-level diagnostic naming the unresolvable column, or the
  plan-explain surface (`04-knobs.md`), so the modeller can restructure.

### 1.7 Grain / identity adequacy

- **What**: if any cell's plan space contains a targeted-write technique (admissible, not
  necessarily chosen), the model needs a declared row identity, and the declared
  `unique_key`/grain must **address** every such cell's writes (a targeted MERGE and a region
  recompute must agree on which physical rows they touch). Recompute-only models need no identity.
- **Who/when**: need-for-identity is analyzer-derived (over admissibility); the identity itself is
  declared-and-checked at plan time.
- **On failure**: identity needed but absent ⇒ `MaintenanceIdentityRequired` *(new)*; identity
  declared but inadequate for some cell ⇒ `MaintenanceIdentityInadequate` *(new)*, naming the cell
  and the unaddressable write.

### 1.8 Output shape consistency

- **What**: the declared output shape (partitioned / keyed / key×time / versioned) is consistent
  with the derived plan — the declaration-law obligation: shape is asserted and validated, never
  silently derived, so a projection refactor cannot flip downstream consumption semantics without
  a diagnostic.
- **Who/when**: declared-and-checked at plan time.
- **On failure**: `MaintenanceShapeMismatch` *(new)* — error on mismatch, never a silent flip.

---

## 2. Skeleton / payload obligations

2.1 **Skeleton derivation.** The skeleton column set — row existence, identity, partition
placement, every membership/grouping/dedup/ordering role — is analyzer-derived per model, at plan
time, from the same role positions the batched taint check already enumerates (`WHERE`, `HAVING`,
`JOIN … ON`, `DISTINCT`, `GROUP BY`, window `PARTITION BY`/`ORDER BY`/frame, `unique_key`,
`event_time_column`/`partition_column`). Failure to resolve a column's role fails closed to
skeleton (strict equivalence demanded). No diagnostic on the conservative default; the plan-explain
surface shows the classification.

2.2 **Intra-model non-determinism confinement** (exists today — `batched_models.md` §"Non-determinism
and the payload rule"). A non-deterministic value flows only into declared
`nondeterministic_columns`; hard exclusions (clock/partition, `unique_key`, membership/grouping
positions) reject regardless of opt-in; run-nondeterministic (`NOW()`) direct projections are
admitted via compile-time pinning. Analyzer taint, plan time; configuration error on violation.
The obligation for the framework: **restate this per column-group** so the exemption is a property
of `g`'s equivalence contract (`exact` vs `plausible-payload`), not a whole-model list.

2.3 **Cross-model payload propagation** (the settled OQ1 rule — *new*). Payload-ness
(plausible-only, non-deterministic) is a column-level property that propagates down the DAG; a
payload column of `M` consumed in a **skeleton position** of `N` fails loud **at the consumer** —
`MaintenancePayloadInSkeletonPosition` *(new)*, offering the two repairs (retro-tighten `M`'s
contract, or derive a stable value). Who/when: analyzer, plan time, requires cross-model column
provenance (already needed for §1.6). This extends today's strictly intra-model taint; it is a new
whole-DAG check.

2.4 **The surrogate-key rule** (settled). A `unique_key` derived from a non-deterministic surrogate
is rejected — `MaintenanceNondeterministicIdentity` *(new)* — unless it is a stable
hash-of-skeleton-columns, which the analyzer verifies by taint (the hash's inputs are all skeleton,
all deterministic). Plan time.

2.5 **Settledness is labeled, not implied.** Every non-immediately-settled column carries its
settle bound in the per-column ledger of guarantees — watermark-relative unless the source declares
an absolute lateness bound (`05-source-properties.md`). This is a *documentation-generation*
obligation (the contract must be materialized where consumers can read it — plan-explain /
catalog), not a refusal; the refusal form is claiming an absolute settle time without a declared
lateness bound to derive it from.

---

## 3. Ledger obligations (run time)

The generalized reconciliation ledger (framework paper, "the generalized ledger" design) carries
run-time obligations that plan-time analysis cannot discharge alone:

3.1 **Grading is licensed by algebra.** Additive (non-idempotent) groups record **per-delta
identity** (partition key / change-feed offset); idempotent groups may record only the frontier
`S_{i,g}` (watermark per input). The license — "re-folding an idempotent delta is harmless" — is a
theorem about the combiner class, dischargeable **once, in CI** by property test (the Link-A
abstract scaffold P0-5 already proves both arms in the abstract: idempotent fold survives
re-delivery/reorder; additive `SUM` needs the ledger, and its control test shows the ledger — not
idempotency — is what keeps it correct). The run-time obligation is then merely mechanical: the
right grading is selected from §1.3's class.

3.2 **Never fold a delta twice.** Fold into `(r, g)` refuses a delta already in the entry's
processed set. Runtime check against recorded identities (additive) / no-op (idempotent). On
violation — which can only arise from ledger corruption or an identity collision —
`MaintenanceLedgerDoubleFold` *(new)*, halting the run (this is a wrong-answer hazard, not a
degradation).

3.3 **Recompute resets the ledger.** A recompute of write-region `W` for group `g` resets every
ledger entry whose ledger-region intersects `W` to exactly the input `S` the recompute read. This
ordering discipline (fold-then-recompute safe; recompute-then-refold double-counts) is the
asymmetric hazard the theorem names; obligation: the reset and the write commit **atomically**
(same transaction as the DELETE+INSERT/MERGE — the execution parity family, §6).

3.4 **Straddle attribution is a partition.** Every delta is attributed to exactly **one** ledger
region (ledger regions are unions of whole footprints; the write region may be finer). Obligation:
the attribution function is total and single-valued — checkable cheaply at run time (assert the
region lookup returns exactly one entry), and property-tested in CI over generated footprints
(including the no-locality case where regions key on output address sets, not intervals).

3.5 **The `S`-vector invariant.** At every quiescent point, stored state of `(r, g)` =
`full_refresh` over `⋃_i S_{i,g}` per the entry. Not directly checkable at run time (that *is* a
full refresh); it is the property the **loop's Link-C oracle** checks offline (§5) and the
tripwires (§4) protect the premises of. The run-time shadow of it: the per-input freshness the
ledger reports (`converted` current on bronze, stale on conversions) must be derived from the
ledger, never estimated.

---

## 4. Source-declaration verification obligations (run-time tripwires)

Declared source properties are *load-bearing premises* of §1's admissions — so each declaration
gets a verification story. Split: **cheap always-on probes** (O(1)–O(delta) per run, on by
default) vs **opt-in audits** (O(table), scheduled). A violated premise invalidates the plan's
admission, so tripwire failure is loud and halts the affected model, never logged-and-continued:
`MaintenanceSourceContractViolated` *(new)*, naming the declaration and the witness.

| declared property | always-on probe (cheap) | opt-in audit (expensive) |
|---|---|---|
| `append_only` | per-run: row count never decreases; max(clock) never regresses; (with a declared key) no delta row's key already exists in the processed set's frontier partition | full-table content hash vs prior snapshot; per-partition count/checksum drift |
| `unique_key` on a source | delta-scoped duplicate check (`GROUP BY key HAVING COUNT>1` over the delta) | full-table uniqueness scan |
| lateness bound `L` | per-run: min(clock of delta) ≥ watermark − `L`; a later arrival is a witness | distribution audit: quantiles of observed lateness vs declared `L` |
| change-feed semantics (exactly-once / at-least-once, retraction-free) | offset monotonicity + gap detection; retraction-flag scan over the delta for a feed declared retraction-free | replay a window of the feed against the base table |
| clocked (`timeseries:`) | clock column non-NULL over the delta; monotone watermark advance | clock-vs-arrival-time skew audit |

Two design obligations for the spec: (1) the *at-least-once* delivery declaration interacts with
§3.2 — an at-least-once feed makes re-delivery a **normal** event the additive ledger must absorb
(dedup by delta identity), not a tripwire violation; the tripwire fires only on identity-*less*
duplication. (2) Lateness-bound violation has two declared policies (refuse vs re-stamp/truncate at
the horizon — `04-knobs.md`); the tripwire enforces whichever is declared, and "silently excluded
by window-forward discovery" (the ledger's G-08/G-06 forward-advance finding) stops being a silent
behavior: beyond-horizon arrivals are **counted and surfaced** even when the policy is to drop
them.

---

## 5. Cross-technique equivalence obligations (CI / the property loop)

The offline obligation is the theorem itself, instantiated per cell: `recompute(R,g,S)` ≡
`fold(R,g,S)` at fixed `S` (idempotent: value-equal; additive: state-equal modulo ledger). The
property-discovery loop is the discharge mechanism; its Link-C oracle (execute smelt's own emitted
maintenance over adversarial schedules, `EXCEPT ALL`-diff against full refresh at step-`k`) is
exactly an `S`-fixed equivalence check. What is already pinned and what remains, as a prioritized
probe backlog in the loop's cell vocabulary (liftable into
`docs/research/property-discovery/catalog.md`):

**Already discharged** (see [`02-loop-findings.md`](02-loop-findings.md)): recompute-region over
append-only for additive/idempotent/holistic aggregates, re-delivery, left-join late-right,
UNION ALL both-arms-late, correlated-EXISTS late-append (G-01..03, 06, 07, 09, SC-1/1b);
backfill-recovers/forward-advance-never-revisits as the uniform traded condition (SC-2, G-04, G-05);
the self-ref trajectory cascade condition (G-08); abstract fold-safety arms (P0-5).

**Priority backlog** (expected verdicts in parentheses):

1. `keyed / cumulative_aggregate path × append-only × fold-into-key-state` — the loop has **never
   exercised the real MERGE/fold path** (`resolve_strategy` returns DeleteInsert for every batched
   cell; `merge_into` backs `cumulative_aggregate`); re-delivery into it is exactly where a ledger
   obligation can actually be violated (G-02's own generality note). (expected: HOLDS or a real
   double-count REFUTED — highest information either way.)
2. `any fold cell × append-only × fold-vs-recompute interchangeability at fixed S` — run both
   techniques to the same `S`, diff (expected HOLDS on skeleton bits; the theorem's direct test —
   becomes runnable only once a fold technique is selectable, `04-knobs.md`).
3. `additive fold × at-least-once re-delivery × ledger dedup` — the §3.2 obligation, adversarial
   duplicate offsets (expected: HOLDS with ledger, REFUTED without — a designed-in control pair).
4. `any windowed cell × append-only with arrivals beyond the derived horizon × window-forward` —
   quantify the silent exclusion, verify the §4 surfacing obligation (expected CONDITIONAL,
   named trade).
5. `additive/invertible agg × change feed with retractions × fold-with-inverse` — retraction
   handling per §1.3's invertible class (expected: HOLDS for `SUM` with signed deltas; REFUTED for
   `MIN`/`MAX` — the non-invertible refusal, G-04's Link-0 arm through the real engine).
6. `join fan-out × composite unique key × dimension-horizon MERGE` — after the G-10 fork resolves
   (expected: HOLDS once `JoinContext` accepts composite keys).
7. `UNION ALL × one arm mutable-snapshot × recompute-region` — mixing G-09's shape with G-04/05's
   hazard (expected HOLDS via recompute; establishes the mutable-arm condition is per-arm).
8. `holistic agg × mutable-snapshot × recompute-region` — G-07's named residual (expected HOLDS;
   confirms holistic soundness is technique-borne, not source-borne).
9. `skeleton/payload cross-model propagation × nondeterministic payload consumed downstream in a
   JOIN key` — once §2.3 exists, a red→green pair (expected: refused at plan time; the loop proves
   the *absence* of the refusal is a wrong-answer generator).
10. `late bronze beyond horizon × declared lateness policy (drop vs re-stamp)` — policy conformance
    (expected CONDITIONAL by declared policy).

**Standing CI obligations** (cheaper than loop cells, run per commit): the Link-A abstract algebra
proptests (P0-5) and Link-B classification diagnostics (P0-6, clamp-probe sufficiency+tightness)
graduate from disposable harness to permanent suites — they pin §1.3/§1.4's derivations against an
independent oracle; and the dormant-classifier tripwire pattern (FIX-2's
`input_delta_discovery_dead_code_tripwire`) extends to `fan_out`/`dimension_horizon_merge` so no
dormant verdict is silently wired to execution without meeting its backlog cell first.

---

## 6. Execution parity obligations

6.1 **DELETE range ≡ INSERT write-window** (exists; keep as a stated invariant + test). The
DELETE+INSERT transaction deletes exactly what the INSERT writes — the write window, not the
widened scan window (`crates/smelt-runtime/src/execute.rs` ~:969–1042 states and enforces this;
G-02 empirically confirmed re-delivery is a full partition replace because of it). Under the
framework this generalizes: **every region-overwrite technique's write region equals the region the
ledger resets** (§3.3), and the scan widen (§1.4) must never leak into the write.

6.2 **One maintenance driver.** Two parallel incremental DELETE+INSERT paths exist (CLI `run.rs`
vs runtime `execute.rs`; the DELETE-covers-the-write-window bug class has already occurred once in
the divergence). Obligation: the per-cell technique executor lives in exactly one place
(`smelt-runtime`, per the run-pipeline-parity invariant), CLI and UI consume it, and the redundant
path dies — see [`08-code-placement.md`](08-code-placement.md). Discharge: an `execute_parity`-style
standing test extended to cover incremental paths, plus deletion of the duplicate.

6.3 **Backend parity per technique primitive.** Each write primitive the plan can emit —
delete+insert (transactional), key-scoped MERGE/upsert, column-scoped UPDATE-join / partial MERGE,
ledger reset — has per-backend conformance tests (DuckDB now; Spark via the gated parity job), so a
cell admitted on one backend is refused, not miscompiled, on a backend lacking the primitive.
Failure at plan time: `MaintenanceBackendPrimitiveUnsupported` *(new)*, naming the primitive —
never a silent downgrade to a different technique (that would be a chooser, not a validator, unless
the theorem's interchangeability licenses the swap at fixed `S`, in which case the swap is legal
and *logged*).

6.4 **Compiled-SQL scoping obligations.** The injected clamp and derived per-source filters must be
alias-correct in every FROM scope — the G-11 binder-ambiguity family (self-ref direct join) and
G-06's same-named multi-timeseries variant. Discharge: the [`03-design-forks.md`](03-design-forks.md)
resolution plus a compile-time "every injected predicate binds unambiguously" check (statically
checkable: qualified emission or outer-wrap by construction).

---

## 7. Summary table

| # | obligation | mechanism | time | on failure |
|---|---|---|---|---|
| 1.1 | replayable input (recompute) | declare + check | plan | technique out of plan space → `MaintenanceNoAdmissibleTechnique` if cell empties |
| 1.2 | faithful fold (partition-of-multiset × combiner faithfulness) | declare (stream) + derive (algebra) | plan (+ §4 premises) | fold out of plan space (observer-semantics refusal) |
| 1.3 | combiner algebra class | derive (`combiner_discriminants`, fail-closed) | plan | unknown ⇒ holistic ⇒ recompute-only (`KeyedUnknownCombiner` today) |
| 1.4 | bounded reach | derive (Form-A/B) else declare + check | plan | `NotDerivable` ⇒ full read; refuse if mode requires bound (`BatchedNotSafe`/`MaintenanceReachNotDerivable`) |
| 1.5 | bounded footprint (A) | derive (reflection + join cardinality) | plan | targeted write out of plan space → `MaintenanceUnboundedFootprint` |
| 1.6 | mutation-sensitivity partition | derive (provenance × mutation profile) | plan | fail-closed merge to one group + `MaintenancePlanDegenerate` (visible) |
| 1.7 | identity adequacy | derive need; declare + check key | plan | `MaintenanceIdentityRequired` / `MaintenanceIdentityInadequate` |
| 1.8 | shape consistency | declare + check vs plan | plan | `MaintenanceShapeMismatch` (never silent flip) |
| 2.1 | skeleton derivation | derive (role positions) | plan | fail-closed to skeleton (strict) |
| 2.2 | intra-model non-det confinement | declare + taint check | plan | configuration error (exists) |
| 2.3 | cross-model payload propagation | derive (DAG taint) | plan | `MaintenancePayloadInSkeletonPosition` at consumer |
| 2.4 | surrogate-key rule | derive (taint on key derivation) | plan | `MaintenanceNondeterministicIdentity` |
| 2.5 | settledness labeling | derive + surface | plan/docs | refusal only for unlabelable absolute claims |
| 3.1 | ledger grading by algebra | proptest (CI) + mechanical selection | CI + run | wrong grading unreachable if 1.3 sound |
| 3.2 | never-fold-twice | runtime check vs recorded identities | run | `MaintenanceLedgerDoubleFold` (halt) |
| 3.3 | recompute-resets-ledger, atomically | runtime, same txn as write | run | txn failure = run failure (no partial state) |
| 3.4 | straddle attribution unique | runtime assert + CI proptest | run + CI | attribution ambiguity halts |
| 3.5 | `S`-vector invariant | offline oracle (loop Link-C) | CI/loop | REFUTED ledger cell ⇒ bug triage |
| 4.* | declared source premises | runtime tripwires (always-on) + audits (opt-in) | run | `MaintenanceSourceContractViolated` (halt affected model) |
| 5.* | fold ≡ recompute at fixed `S`, per cell | property loop (Link A/B/C) | CI/loop | REFUTED ⇒ admission matrix narrows |
| 6.1 | DELETE ≡ write-window; scan widen never leaks into write | code invariant + tests | CI | test failure |
| 6.2 | one maintenance driver | structural (parity test + deletion) | CI | parity test failure |
| 6.3 | backend primitive parity | conformance tests per backend | CI | `MaintenanceBackendPrimitiveUnsupported` at plan |
| 6.4 | injected predicates bind unambiguously | compile-time check | plan | diagnostic, not a DuckDB binder error |

---

## References

- [`01-framework.md`](01-framework.md) — the interchangeability theorem and its `S`-vector; the
  skeleton/payload split; the generalized ledger; what stays singular (identity/shape).
- [`02-loop-findings.md`](02-loop-findings.md) — empirical status of each obligation today.
- [`docs/research/property-discovery/ledger.md`](../property-discovery/ledger.md) — cells cited as
  G-nn / SC-n / P0-n / FIX-n.
- `docs/specs/batched_models.md` §"Non-determinism and the payload rule" (2.2's current form);
  `docs/specs/keyed_models.md` §"Admission matrix"; `docs/specs/diagnostics.md` (existing codes).

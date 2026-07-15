# Composed axes (key + time) and conditional maintenance

**Status:** SKELETON — structure and sequencing are settled; per-phase test lists, file
inventories, and review checklists are to be fleshed out before implementation (Andrew).

- **Spec:** `docs/specs/incremental_models.md`
- **Spec diff:** the 2026-07-15 composed-axes diff on the `spec-incremental-models-consolidation`
  branch — §Surface "The two axes are orthogonal — 'partitioned or keyed' is a category error";
  §"Windowed maintenance and the horizon" "Three pruning categories, one principle" (category 2:
  no-op write elimination); §"What the composed shape uniquely enables"; the graph layer's
  keyed-node refusal refined to "without an admitted time axis"; §Design "The axes compose;
  exclusivity is the recurring error"; three new Known Divergences; Future Extensions
  "Conditional maintenance without a change feed".
- **Research:** `docs/research/20260715-conditional-maintenance-without-cdf.md` (M1/M2/M3,
  P1–P4, T1–T5, prior art, cost model); `docs/research/20260705-keyed-time-superset.md`
  (key temporal locality, the composed shape).
- **Related plan:** `docs/plans/20260705-keyed-time-partitioned-output.md` predates this plan
  and covers part of Group A. **Decision needed before starting Group A:** absorb it into this
  plan (mark it superseded, pointing here) or keep it as the Group A vehicle and have this plan
  depend on it. Do not run both.
- **Docs:** each group that changes user-visible behaviour carries a docs phase
  (docs-site + spec Surface); groups A0/B are internal-behaviour + spec only.

## Why one plan

The two workstreams gate each other and share their best demo:

- The composed shape (locality-admitted `grain: key` + `timeseries:`) is what makes conditional
  maintenance *affordable* (slice-bounded compares) and *propagatable* (exact key→partition
  dirt projection, so observed output deltas ride the interval graph with no keyed dirt-sets).
- Conditional maintenance is what makes the composed shape *pay off* downstream (settled or
  unchanged slices → empty-delta no-op cascades).
- The flagship fixtures coincide: event-grain dedupe under `key_recurrence` (locality's
  motivating shape) is exactly where change-suppressed writes turn redelivery storms into
  zero-write no-ops; the web-analytics events→sessions→identity chain is the model-edge M2/M3
  demo.

Sequencing follows research §11: suppression first (cheap, standalone), observed deltas second,
delta-restricted compute over model edges third, external-source sidecars last. Locality (Group
A) runs in parallel with C — they meet at C6 and D3.

## Progress

| Phase | Description | Status |
|---|---|---|
| A0 | `key_per_partition` fail-loud refusal (stop the silent collapse) | pending |
| A1 | Locality route 1 (key-embedded): admission + narrowed `KeyedForbidsTimeseries` message | pending |
| A2 | Slice-pruned merge target scan | pending |
| A3 | Locality route 2 (key-determined, once-write provenance) | pending |
| A4 | Locality route 3 (recurrence-bounded): `key_recurrence` source declaration + transactional `KeyedRecurrenceBoundViolated` check | pending |
| A5 | Settle-bound derivation + `smelt explain` surface | pending |
| A6 | Docs: time-partitioned keyed output (docs-site guide + reference) | pending |
| B1 | Graph admissibility for locality-admitted keyed nodes (edge construction, granularity) | pending |
| B2 | Key→partition dirt projection (exact routes 1–2; widen-by-`r` route 3) in forward propagation + backward resolution | pending |
| B3 | `--since-upstream` accepts a composed node as `--source`; adjointness tests extended | pending |
| C1 | Spec diff: `model_transforms.md` T1/T2 (change-suppressed MERGE; staged-candidate conditional DELETE+INSERT); `multi_backend.md` capability flags | pending |
| C2 | P3 change-comparability per column (walk lattice fold; `plausible`/pinned-`NOW()` ⇒ Incomparable) | pending |
| C3 | P2 region row identity (declared `unique_key` → proven grain key → `WholeRow` multiset) | pending |
| C4 | T1 on the column-scoped MERGE emitter (+ statement-parity leg; conformance suppressed-vs-unconditional bit-equality at fixed `S`) | pending |
| C5 | T1 on the keyed fold; T2 staged-candidate conditional DELETE+INSERT (statement groups gain a staged temp relation; first keyed path for merge-less backends) | pending |
| C6 | Slice-bounded compare on composed models (compose C4/C5 with A2) | pending |
| C7 | Docs: conditional writes (explain output, cost notes, `prefer`/`technique` steering) | pending |
| D1 | Spec diff: `sources.md` landed-delta refinement (whole-table → changed-row set / partition projection); storage home + transactionality of recorded deltas | pending |
| D2 | T5 observed output delta recording (comparable columns only; byproduct of C4/C5 writes) | pending |
| D3 | Partition projection of observed deltas via locality → exact `--landed` for model edges; propagation consumes it | pending |
| D4 | `smelt explain` observed-delta/settle surface; docs | pending |
| E1 | Spec diff: `model_properties.md` P1 skeleton-source closure; `sources.md` referential-integrity world-fact + count-preservation tripwire | pending |
| E2 | P1 proof (skeleton-role × provenance × OneToOne × row preservation × no membership predicates on enrichment columns; fail-closed to `Open`) | pending |
| E3 | T3 delta-restricted compute over **model edges** (consume D2/D3 deltas; web-analytics events→sessions chain demo) | pending |
| E4 | Conformance legs: delta-restricted vs widened-scan equivalence at fixed `S`; empty-delta no-op cascade end-to-end | pending |
| F1 | Spec diff: fingerprint sidecar (naming, storage home, transactionality, invalidation; digest stance vs `output_fingerprint.md`); P4 projection derivation | pending |
| F2 | P4 fingerprint-projection derivation (fail-closed: unprojectable ⇒ full-row digest) | pending |
| F3 | T4 sidecar DDL/DML via emitters, upserted in the consuming write's transaction; external `mutable_snapshot` delta derivation | pending |
| F4 | Sidecar invalidation (definition change / schema evolution ⇒ "everything changed", widen-never-narrow) | pending |
| F5 | T3 over external sources (fixture: `daily_events_enriched` — needs `LEFT JOIN` or the RI declaration, deliberately: the closure proof must discriminate) | pending |
| G1 | Cost-model/bakeoff integration: conditional vs unconditional as proven-interchangeable per-cell choice; first-run/backfill admit-but-not-prefer | pending |
| G2 | Docs sweep + `/smelt:validate incremental_models` drift report | pending |

## Phase notes (skeleton level — flesh out per phase before running)

### Group A — the composed shape exists at all (key temporal locality)

- **A0** is a hygiene fix shippable immediately: `derive_model_maintenance_plan` maps
  `Grain::KeyPerPartition` to `PlanGrain::Key { unique_key: vec![] }` — a silent grain collapse
  that also drops the declared key. Replace with an explicit not-yet-supported refusal naming
  this plan. TDD: a `key_per_partition` fixture asserting the refusal diagnostic (not a keyed
  plan). One-commit phase.
- **A1–A5** implement the three routes in spec order (structural preconditions first). The
  admission seam is `crates/smelt-core/src/metadata.rs`'s unconditional `KeyedForbidsTimeseries`
  — it becomes route evaluation, and the *message* becomes the three-routes-and-nearest-missing-
  fact wording the spec already specifies. A4 carries the only new declared surface
  (`key_recurrence` on sources) and the transactional violation check.
- Oracle: extend `maintenance_conformance` recipes with a composed-shape recipe (keyed +
  timeseries, all three routes) asserted against the full-refresh oracle per run step, plus
  per-slice equivalence.

### Group B — graph layer

- B1/B2 change `build_forward_graph`/`propagate`/`required_inputs`: a locality-admitted keyed
  node contributes edges at its declared granularity instead of refusing. Route 3's projection
  widens backward by `r` + margins — the widening lives in the projection, not the edge clamp.
- The `maintenance_propagation_adjoint` law (`forward(backward(P)) ⊇ P`) must be extended over
  composed nodes — that is the phase's TDD spine.
- B depends on A1 (at least one admitted route) and nothing in C–F.

### Group C — change-suppressed writes (M1)

- C1 is the spec-first gate for the group: T1/T2 as *variants* in `model_transforms.md`'s
  catalogue (a property licenses, never chooses), `WHEN NOT MATCHED BY SOURCE` vs scoped-DELETE
  dialect split, and the `multi_backend.md` capability flags (including the pre-existing drift
  that `supports_column_scoped_merge` is absent from the capability matrix).
- C2/C3 are pure proofs in `smelt-logical` (walk-composed; fail-closed). C2's subtlety: pinned
  `NOW()` is comparable *within* a run but incomparable *across* runs — it must be Incomparable.
- C4 is the single cheapest high-value change in the whole plan (one predicate in one emitter)
  and directly mitigates the recorded "dispatch fires every run unconditionally" divergence.
  Conformance leg: suppressed and unconditional variants produce bit-identical state at fixed
  `S`; statement-parity leg for the new emitter text.
- C5's genuinely new machinery is the staged-candidate **statement group** (one temp relation +
  dependent statements, one transaction) — it also gives Spark-over-Parquet its first keyed
  lowering. Keep the emitter the single author; extend the structural no-authoring gate.
- C6 composes with A2 and is the composed-shape payoff phase; measure compare cost ∝ slice.

### Group D — observed output deltas (M3-output)

- D2 records the changed-row set the conditional write already computed — comparable columns
  only (a `plausible` column's flutter must never dirty downstream). Storage home decided in D1
  (warehouse-resident beside the merge ledger, same-transaction, is the default posture).
- D3 is the composed-axes keystone: key-level deltas project to exact partition dirt (routes
  1–2), making model-edge `--landed` precise with **no keyed dirt-sets** — resolves research
  open question 3 for the composed case. Bare keyed nodes still propagate nothing (refused,
  Group B).
- v1 posture: record key-level, propagate partition-level (widen-never-narrow).

### Group E — delta-restricted compute over model edges (M2, free-delta case first)

- E1's referential-integrity world-fact follows `sources.md`'s trust rule: narrowing declaration
  ⇒ paired runtime tripwire (count preservation over the region).
- E2 restricts `SkeletonClosure` to non-aggregating enrichment scopes in v1 (join-below-
  aggregation ⇒ `Open`); the shipped `daily_events_enriched` uses a bare `JOIN` and must *fail*
  the closure until F5 — keep a pinned test asserting that refusal (the proof must discriminate).
- E3 consumes Group D's deltas on maintained-model edges only (no sidecar): the web-analytics
  chain is the demo — redelivered event → sessions writes conditionally → downstream joins only
  `session_id ∈ Δ`.

### Group F — input sidecars (M3-input; last, most new state)

- F1 must reconcile with `output_fingerprint.md`'s "ephemeral, never persisted" principle: the
  row-content fingerprint is a different artifact class (cross-run by definition) and needs its
  own naming/GC/full-refresh/multi-consumer story. Digest stance per research §6: SHA-256-class
  for sidecars with the soundness invariant stated + oracle-gated; exact `IS DISTINCT FROM` for
  write suppression.
- F5 closes the loop on the original fixture: one renamed user out of 10M ⇒ point-lookup
  enrichment recompute.

### Group G — choice and docs

- G1: conditional variants enter per-cell technique choice as proven-interchangeable (strongest
  sense); cost model / `smelt bakeoff` / `prefer`/`technique` steer; first-build and
  definition-change backfill stay unconditional by preference. Open question: does the cost
  model want region-level change-ratio statistics from prior observed deltas?
- G2: `/smelt:validate incremental_models` + user docs (docs-site guide section on conditional
  maintenance and the composed shape; the "partitioned or keyed is a category error" framing
  belongs in the docs too — the heresy creeps in from docs as much as from specs).

## Decisions needed before/while fleshing out

1. **Absorb vs depend** on `docs/plans/20260705-keyed-time-partitioned-output.md` (Group A
   overlap). Pick one; mark the other superseded.
2. **`plausible` semantics under suppression** (research §10.1): is "stale but
   previously-correct" plausible? Decides refuse-vs-compare-comparable policy (spec currently
   pins fail-closed refusal as the default).
3. **Digest acceptance surface** (research §10.2): global SHA-256 soundness assumption vs
   per-source named acceptance.
4. **Sidecar lifecycle** (research §10.5): namespace, GC, `--full-refresh` behaviour,
   multi-consumer sharing.
5. **Observed-delta trust boundary** (research §10.6): does T5 need an out-of-band-edit
   tripwire, or is the smelt-owned-state assumption enough?
6. **`key_per_partition`'s real profile** — A0 only makes it refuse honestly. Building the
   trajectory grain (backfill-cascade discipline, lateness truncation — `models.md` names both)
   is deliberately *not* in this plan; decide whether it gets its own plan or waits for demand.

## Verification spine (applies to every implementing phase)

- Red-green TDD per phase; new proofs land walk-composed (`walk_coverage` gate).
- Statement emission stays single-owner: every new/changed emitter gets a `statement_parity.rs`
  leg; the structural no-authoring gate extends to the new statement shapes.
- The `maintenance_conformance` generative gate is the equivalence net for every technique
  variant: suppressed, staged-candidate, delta-restricted, and sidecar-fed runs must all equal
  the full-refresh oracle after every step under the adversarial schedules; a suppressed-write
  bug is exactly the class it exists to catch.
- `bash .claude/scripts/verify-phase.sh` per phase; commit + push per phase to the tracking PR
  branch.

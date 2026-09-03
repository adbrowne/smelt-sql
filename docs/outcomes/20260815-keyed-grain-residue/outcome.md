# Outcome: Close the key grain's implementation-only residues

**Created:** 2026-08-15
**Status:** active
**Source:** `docs/specs/incremental_shapes.md` §"The key grain" §Known Divergences;
`docs/outcomes/20260815-definition-delta-migrate/outcome.md` §"Out of scope"
**Spec anchors:** `docs/specs/incremental_shapes.md`, `docs/specs/incremental_models.md`

## The outcome

Five key-grain divergences close where the spec has already decided the target behaviour and only
the implementation is missing — no new product decision required. `KeyedRetractableContribution`
gets a real classifier, diagnostic, and test: §"Enrichment joins" and the Diagnostics table already
state exactly when it fires and what it steers toward (`refresh: materialized_view` or DAG
composition), nothing about its semantics is undecided. §"The transactional frontier write (merge
ledger)" already states "every window-forward keyed model maintains a per-model frontier" — not
"every additive-graded one" — so a re-run-tolerant (non-additive) model writing no ledger record
today is a plain conformance gap against that sentence, not an open question; the same section's
"backend-resident and transactional with the write it describes" already implies every backend
must fold the ledger transactionally, so the DuckDB-only override is the gap, not the target.
§"Derived execution postures" already defines order-independence formally ("holds iff every
combiner is order-independent") — the implementation just never computes or prints the verdict it
already specifies. And the generative conformance pool's non-nullable payload type is a test-harness
gap against a family (once-write) whose NULL-preservation obligation is already fully specified in
§"The column-family catalogue".

## Success criteria (checkable)

1. `KeyedRetractableContribution` has a real classifier, a fixture that produces it, and a test —
   matching exactly the semantics already stated in §"Enrichment joins" and the Diagnostics table
   (no new admission rule invented).
2. A re-run-tolerant (non-additive) window-forward model writes a frontier record, matching
   §"The transactional frontier write (merge ledger)"'s unqualified "every window-forward keyed
   model" statement; `--auto` staleness can consult it.
3. The reconciliation ledger's fold is transactional on every shipped backend (matching the
   already-stated "backend-resident and transactional with the write it describes" guarantee), not
   DuckDB-only.
4. Order-independence (and the other derived execution postures already defined in §"Derived
   execution postures") is computed as a real verdict, not assumed sequential by default, and
   `smelt explain` prints it alongside the run shape.
5. The generative conformance pool's row type carries a nullable payload; the once-write family's
   already-specified NULL-preservation obligation is proven by the generated pool, not one
   hand-written test case.
6. `/smelt:validate incremental_shapes` reports no drift for every bullet this outcome closes; all
   standing gates green.

## Out of scope

The following key-grain `(Open Question)` bullets are **not** in this outcome because the spec
itself has not decided the target behaviour — building any of them means choosing new admission
width or a new surface, which is a product call, not an implementation gap. They stay named in
`docs/outcomes/20260815-definition-delta-migrate` §"Out of scope" pending explicit sign-off:
snapshot-reconcile multi-unclocked-source admission, once-write nullability for a key-derived
*expression* (widens the catalogue's four fixed spellings), pattern functions as built-ins vs. a
shipped template, driver granularity below `day`/`week`, `--auto` staleness fidelity beyond
conservative v1, self-referential keyed models, and run-pinning alignment for `NOW()`/`CURRENT_*`
in keyed models (today a deliberate hard refusal, `KeyedForbidsNondeterministic` — relaxing it
changes stated behaviour, not just fills a gap).

Also out of scope, discovered by phase 1: `repair::admit_per_group_recompute` passes an empty
`JoinContext` and never projects a join's own `ON` columns, so per-group repair can never admit
for a source reached only through a JOIN. It is a real limitation, but it belongs to the repair
family's admission width (`docs/outcomes/20260809-repair-family`), not to any of this outcome's
six success criteria — criterion 1 is already met end-to-end without it.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | `KeyedRetractableContribution`: classifier, diagnostic, fixture, test | done |
| 2 | Ledger presence for re-run-tolerant models, matching the spec's unqualified "every window-forward model" statement | planned |
| 3 | Transactional ledger fold on every shipped backend | pending |
| 4 | Derive and print execution postures (order-independence) in `smelt explain` | pending |
| 5 | Generative conformance pool: nullable payload, once-write NULL direction covered | pending |
| 6 | Validate + close out: `/smelt:validate incremental_shapes` clean, standing gates green | pending |

## Decision log

- 2026-09-03 — Outcome activated. Phase 1 planned with no reshape: no prior phase summary exists
  in this outcome, and the six phase rows still match the success criteria one-for-one. Phase 1's
  derivation seam was fixed to the key-grain `NewData` handler's repair-refusal arm in
  `smelt-logical/src/maintenance/derive.rs` — the only site where both halves of
  `KeyedRetractableContribution`'s stated firing condition (a retractable enrichment-join
  contribution; a repair family that cannot admit a per-group recompute) are already computed, so
  no new admission rule is invented.
- 2026-09-03 — Phase 1 implemented and closed out (all green:
  `.claude/scripts/verify-phase.sh`, `repair_wiring`, `maintenance_diagnostics`,
  `statement_parity`, `technique_lowering`, `maintenance_conformance`, `join_shape` unit tests).
  Discovered (not fixed, out of scope for this phase): `repair::admit_per_group_recompute` always
  passes an empty `JoinContext` to affected-key discovery and never projects a join's own `ON`
  columns, so per-group repair can never admit for a source reached only through a JOIN — flagged
  for the next planner as a candidate follow-up phase.

- 2026-09-03 — Phase 2 planned. No reshape of the remaining rows: phase 1's summary surfaced one
  new limitation (empty `JoinContext` in per-group repair admission), which serves none of the
  six success criteria and is recorded under "## Out of scope" pointing at the repair-family
  outcome. Phase 2's design was fixed to generalising the existing
  `execute_conditional_write_and_record_observed_delta` backend seam into
  `execute_write_with_bookkeeping` (one transactional implementation) rather than adding a
  parallel ledger-write method, and to writing the bookkeeping record with **no** `state.mode`
  gate — `state.md` §"`state.mode` and what each posture provides" already places correctness
  structures in every posture.

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->

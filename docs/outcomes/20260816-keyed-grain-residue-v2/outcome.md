# Outcome: Close the key grain's residues (v2, decision-grown)

**Created:** 2026-08-16
**Status:** queued
**Source:** `docs/handoffs/2026-08-16-delta-signature-closure-programme.md` (programme outcome 4);
carries forward `docs/outcomes/20260815-keyed-grain-residue/` (superseded) and adds the scope the
decision track graduated (`docs/research/20260816-open-questions-triage.md` items 1, 7, 10, 18).
**Spec anchors:** `docs/specs/incremental_shapes.md`, `docs/specs/incremental_models.md`

## The outcome

The key grain's Known Divergences close against the spec as it now stands after the decision
track. The headline is posture-derived deletion: a snapshot-reconcile run deletes keys absent
from the incoming scan (anti-join in the merge transaction), and `retain_departed` ships as the
third contract-lattice point — declaration, quotient oracle, anti-join probe,
`ContractRetainDepartedInvalid` — so keeping departed keys is a declared relaxation, never a
silent default. Alongside it, the carried implementation-only residues land:
`KeyedRetractableContribution` gets a real classifier and fixture, re-run-tolerant models write
the frontier whenever the project's state mode supports it, derived execution postures
(order-independence) are computed and printed, the generative pool stages NULL payloads, the
keyed `NOW()`/`CURRENT_*` refusal narrows to identity/membership positions per the determinism
scope, and the pattern helpers ship as a `smelt.define` template file.

## Success criteria (checkable)

1. **Posture-derived deletion is live.** A snapshot-reconcile run deletes a key absent from the
   incoming scan via an anti-join executed in the same transaction as the merge; the generative
   conformance suite stages key departure and the full-refresh oracle comparison holds without
   the departed-keys exemption. Closes `incremental_shapes.md` "Departed keys are still retained
   under snapshot-reconcile."
2. **`retain_departed` ships as a complete lattice-point triple** single-owned in `smelt-logical`
   (declaration schema incl. the tombstone form, quotient oracle transform consumed directly by
   the conformance gate, anti-join probe recording the retained-departed count) with
   `ContractRetainDepartedInvalid` firing on wrong-shape/wrong-posture/missing-tombstone-column
   declarations.
3. `KeyedRetractableContribution` has a real classifier, a fixture that produces it, and a test —
   matching the semantics already stated in the spec (no new admission rule invented).
4. A re-run-tolerant (non-additive) window-forward model writes a frontier record whenever the
   project's state mode supports it, and `--auto` staleness consults it. Closes "Re-run-tolerant
   keyed models do not yet write the frontier."
5. Derived execution postures (order-independence included) are computed as real verdicts and
   printed by `smelt explain` alongside the derived run shape, matching §"Derived execution
   postures".
6. The generative conformance pool's row type carries a nullable payload; the once-write family's
   NULL-preservation obligation is proven by the generated pool, not one hand-written case.
7. `KeyedForbidsNondeterministic` narrows per the determinism scope: `NOW()`/`CURRENT_*` admitted
   in payload positions (running as-is, no equivalence promise on the fed columns), still refused
   in `unique_key`/`GROUP BY`/membership positions; `RANDOM()`/`UUID()` unchanged. Closes
   "`NOW()`/`CURRENT_*` are still rejected in keyed models."
8. The pattern-function template file (`smelt.latest`, `smelt.once`, `smelt.current`) ships and
   is importable; each expansion is admitted on the same terms as the hand-written spelling.
   Closes "The pattern-function template file does not exist."
9. `/smelt:validate incremental_shapes` reports no drift for every bullet this outcome closes;
   all standing gates green (`maintenance_conformance`, `statement_parity`, `walk_coverage`
   included).

## Out of scope

- Ledger fold transactionality on non-DuckDB backends and the ledger's residency/downgrade
  machinery — owned by `20260816-state-residency` and the recorded Spark-deferral decision
  (triage items 11–12).
- The shared determinism-scope machinery (partition pinning removal, conformance-oracle
  exemption, explain per-column exemption surface) — `20260816-partition-grain-residue-v2`;
  this outcome only narrows the keyed refusal.
- Everything the specs moved to Future Extensions: multi-source snapshot-reconcile,
  self-referential keyed models, deletion-adjacent locality relaxations, wider driver
  granularities, exact `--auto` staleness, built-in promotion of the pattern helpers.
- Change-feed fold machinery (rides the deletion rule but is its own Future Extensions entry).
- Once-write admission for key-derived *expressions* (widens the catalogue — undecided).

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Snapshot-reconcile anti-join delete leg (transactional with the merge) + conformance key-departure staging, red-green against the oracle | pending |
| 2 | `retain_departed` lattice-point triple in `smelt-logical` (declaration incl. tombstone, quotient oracle, probe) + `ContractRetainDepartedInvalid`; conformance gate consumes the oracle transform | pending |
| 3 | `KeyedRetractableContribution`: classifier, diagnostic, fixture, test | pending |
| 4 | Frontier record for re-run-tolerant models gated on the project's state mode; `--auto` consults it | pending |
| 5 | Derive + print execution postures (order-independence) in `smelt explain` | pending |
| 6 | Generative pool: nullable payload, once-write NULL direction covered by generation | pending |
| 7 | Narrow `KeyedForbidsNondeterministic` to identity/membership positions per the determinism scope | pending |
| 8 | Pattern-helper `smelt.define` template file + admission-parity tests | pending |
| 9 | docs-site updates for deletion/retention and the narrowed refusal; validate + close out (`/smelt:validate incremental_shapes`, full gate sweep) | pending |

## Decision log

- **Inherited (2026-08-16, decision track).** All product calls this outcome implements are
  recorded in `docs/research/20260816-open-questions-triage.md` and already landed as spec text
  (PR #167); this outcome makes no product decisions of its own.

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->

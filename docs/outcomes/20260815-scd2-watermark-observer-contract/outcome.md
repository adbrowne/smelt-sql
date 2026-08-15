# Outcome: SCD2 succession recognition, automatic watermark-diffed `--since-upstream`, and the observer/prefix-consistency contract

**Created:** 2026-08-15
**Status:** queued
**Source:** `docs/specs/incremental_models.md` §Future Extensions (SCD2 via succession-pattern
recognition; automatic watermark-diffed `--since-upstream`; the observer/prefix-consistency
contract for non-replayable combinations); `docs/specs/incremental_shapes.md`'s
snapshot-observer-semantics refused matrix cells (the contract's eventual admission target);
`docs/outcomes/20260815-definition-delta-migrate/outcome.md` §"Out of scope"
**Spec anchors:** `docs/specs/incremental_models.md`, `docs/specs/incremental_shapes.md`

## The outcome

The three Future Extensions items `incremental_models.md` names together as "not decided ... may
not be relied on or implemented against until it graduates ... via its own spec diff" all graduate.
The observer/prefix-consistency contract is specified and implemented first, since the other two
build on it: it defines when a non-replayable (observation-based) combination is safe to admit,
targeting the currently-refused snapshot-observer-semantics matrix cells in `incremental_shapes.md`
(`MIN`/`MAX` over snapshots, `MAX_BY` regression, `COALESCE`-once-write under snapshot-reconcile).
Automatic watermark-diffed `--since-upstream` replaces the manual `--landed`-flag workaround the
scheduler-currency divergence names as its residue. Smelt-maintained SCD2 via succession-pattern
recognition ships as a classifier + emitter recognizing the succession pattern and maintaining a
type-2 slowly-changing dimension without hand-written merge SQL.

## Success criteria (checkable)

1. The observer/prefix-consistency contract is specified in `incremental_models.md` (a new spec
   diff, not an ad hoc admission) — what makes a non-replayable combination safe, and what proof
   or declaration is required.
2. At least one currently-refused snapshot-observer-semantics matrix cell in
   `incremental_shapes.md` §"Admission matrix (column family × source shape)" is admitted under
   the new contract, with the refusal narrowed to genuinely unsafe cases and the cell's ✗ becomes
   a conditional ✓ in the table.
3. Automatic watermark-diffed `--since-upstream` ships, replacing the manual `--landed` flag; the
   scheduler-currency divergence's cross-model-watermark residue (named in
   `docs/outcomes/20260815-definition-delta-migrate` phase 10's decision) is closed.
4. Smelt-maintained SCD2 ships: a classifier recognizes the succession pattern, an emitter
   maintains the type-2 dimension, with a generative-conformance fixture proving it against a
   full-refresh oracle.
5. All three Future Extensions entries are removed from `incremental_models.md` (promoted into
   the normative body); `/smelt:validate incremental_models` and `/smelt:validate
   incremental_shapes` report no drift for the bullets this outcome closes. All standing gates
   green.

## Out of scope

- None named; if the contract turns out not to admit any currently-refused matrix cell safely,
  record that finding in the decision log and narrow criterion 2 rather than silently dropping it.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec: observer/prefix-consistency contract in `incremental_models.md` | pending |
| 2 | Implement the contract's admission check + proof/declaration surface | pending |
| 3 | Admit the currently-refused snapshot-observer matrix cells the contract now covers | pending |
| 4 | Automatic watermark-diffed `--since-upstream` | pending |
| 5 | SCD2 succession-pattern classifier + emitter | pending |
| 6 | Generative conformance fixtures for SCD2 and the newly-admitted observer cells; docs-site | pending |
| 7 | Validate + close out: `/smelt:validate` clean for both anchor specs, standing gates green | pending |

## Decision log

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->

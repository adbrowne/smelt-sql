# Outcome: Decided-gap residue — close the spec bullets whose target behaviour is already written

**Created:** 2026-09-04
**Status:** active
**Source:** `docs/outcomes/20260815-incremental-spec-closure-confirm/closure-report.md` rows IM-25, IS-13 ("implementation gap in an already-decided design"); `docs/TODO.md` bullets "Frozen-horizon append-only gate", "Deferral oracle restatement", "Sidecar per-consuming-edge audit"
**Spec anchors:** `docs/specs/incremental_models.md` §"The contract lattice", §Known Divergences "Conditional-maintenance gaps"; `docs/specs/incremental_shapes.md` §"The column-family catalogue" (once-write), §Known Divergences; `docs/specs/sources.md` §Known Divergences (sidecar per-consumer comparandum)

## The outcome

Five gaps where the spec states the behaviour and only the code is missing are closed, each
against the sentence that already decides it. `frozen_horizon` declared on a non-append-only
driving source refuses with `ContractFrozenHorizonInvalid`. The conformance gate's deferral
oracle transform checks the restated landed-vs-processed form, and a probe that would pass under
a vacuous comparator fails under the real one. The once-write classifier has a nullability route
around its fallback case. The fingerprint sidecar upholds the per-consuming-edge comparandum
under a shared projection-identity partition, proven by a test. A target without
`supports_fingerprint_sidecar` takes the spec's stated conditional-maintenance behaviour rather
than the current residue.

## Success criteria (checkable)

1. `ContractFrozenHorizonInvalid` fires for `frozen_horizon` on a non-append-only driving
   source, is listed in `diagnostics.md`, reaches LSP diagnostics, and has a fixture + test; the
   `docs/TODO.md` bullet is removed.
2. The deferral oracle transform in `smelt-logical` implements the restated landed-vs-processed
   definition; a metamorphic test shows a deliberately wrong incremental state that the old
   vacuous comparator accepted is now rejected; the TODO bullet is removed.
3. The once-write classifier admits (or refuses with a named diagnostic) the fallback case's
   nullability route per §"The column-family catalogue"; the `incremental_shapes.md` bullet is
   deleted and the generative pool's once-write NULL schedule covers the route.
4. A test stages two consumers of one source under a shared projection-identity partition and
   asserts each consuming edge gets its own comparandum; the `sources.md` divergence bullet is
   deleted or rewritten to the residual gap; the TODO bullet is removed.
5. The `supports_fingerprint_sidecar` residue named in `incremental_models.md` §Known Divergences
   "Conditional-maintenance gaps" is closed per that bullet's stated target, with a test on a
   backend-capabilities stub that lacks the sidecar.
6. `/smelt:validate incremental_models`, `incremental_shapes`, `sources` report no drift for the
   closed bullets; `maintenance_conformance`, `statement_parity`, `verify-phase.sh` green.

## Out of scope

- Per-column `data_latency` (IS-04): decided but a feature, not a residue; queue separately if
  wanted.
- Every `(Open Question)` bullet the closure report classifies as a product decision (IS-05,
  IS-07, IS-19, IS-20, IS-21, IS-31, IS-32, MP-07, MP-10, MP-14, MP-16, IM-15, IM-16).
- The derived model-wide horizon (IM-19) and emission remainders (IM-11): larger in-progress
  tracks that deserve their own outcome.
- Widening once-write nullability to key-derived expressions (named out of scope by the
  keyed-grain residue outcome).

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | `ContractFrozenHorizonInvalid`: validation leg, diagnostic, LSP, fixture, test | done |
| 2 | Deferral oracle transform restated; metamorphic test proving the comparator is no longer vacuous | done |
| 3 | Once-write fallback-case nullability route; generative pool coverage | pending |
| 4 | Sidecar per-consuming-edge audit test; fix if it fails | pending |
| 5 | `supports_fingerprint_sidecar` residue closed against its stated target | pending |
| 6 | Delete/rewrite the closed bullets, TODO cleanup, validate, gates green | pending |

## Decision log

<!-- Dated one-liners appended by plan/implement steps. -->

- 2026-09-05: Phase 1 planned with no reshape (no prior summaries). Settled the ambiguous case
  in the spec sentence "any other **declared** mutation profile": an *undeclared* driving-source
  profile is admitted, since nothing declared contradicts the probe and the undeclared case is
  already policed at run time by `SourceMutationProfileViolated`.
- 2026-09-05: Phase 1 implemented and closed — `ContractFrozenHorizonInvalid` now refuses a
  `frozen_horizon` declaration on a model whose driving source declares a non-`append_only`
  mutation profile, surfaced through `check_file_diagnostics`, LSP-published, and covered by a
  new `examples/broken` fixture. See `phases/01-summary.md`.
- 2026-09-05: Phase 2 planned with no reshape. Named the vacuity precisely: the gate's
  `Bracketed` obligation holds vacuously on its lower leg whenever the settled cutoff precedes
  all recorded event time, so the upper leg alone admits any subset of `full_refresh(S)`. The
  restatement is the spec's own two obligations — strict equality over the processed set plus a
  lag bound on `L \ S` — which additionally requires `STracker` to split *landed* from
  *processed* (the current fixture records a run for a window the deferred model never folded).

- 2026-09-05: Phase 2 implemented and closed — `OracleObligation::Bracketed`
  replaced by `ExactOverProcessedSWithLagBound` (strict equality over `S` plus
  `deferral::settled_lag_bound` over landed-but-unprocessed event times);
  `STracker` gained `record_landing`/`landed_at`/`landed_not_processed`; the
  conformance fixture's run B is now recorded as a landing, not a run
  (deferred_model never folded that window); a new metamorphic test proves
  the restated comparator rejects a maintained state the superseded bracket
  admitted. See `phases/02-summary.md`.

## Blocked

<!-- Dated entries; each names the phase, what blocked it, and what a human must decide. -->

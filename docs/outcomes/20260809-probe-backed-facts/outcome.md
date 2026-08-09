# Outcome: Probe-backed world-facts

**Created:** 2026-08-09
**Status:** active
**Source:** `docs/research/20260809-incremental-rethink.md` §2 P-C, §6 step 3
**Spec anchors:** `docs/specs/model_properties.md` (model-scoped declarations), `docs/specs/incremental_models.md` (declared contract facts)

## The outcome

Every declared world-fact derives a cheap runtime probe that can falsify it —
the way the recurrence-bound and count-preservation probes already work.
"Declared" comes to mean "checked at run time", not "trusted forever": a
declaration is admissible only if a probe exists for it, a firing probe is a
named diagnostic with a remedy path, and the declared-facts surface becomes
safe to grow (the contract lattice will grow it).

## Success criteria (checkable)

1. The `referential_integrity` tripwire exists: the closure narrowing it
   licenses is verified by a probe in the runs that rely on it (closing the
   admitted-ahead-of-verification divergence in `model_properties.md`).
2. Declared functional dependencies, `bounded_domain`, source posture
   (append-only), and `assert_monotonic` each have a probe emitter in the
   single-owner maintenance layer; probe statements pass `statement_parity`.
3. The spec states the admissibility rule: no probe, no declaration; each
   declaration's section names its probe and firing semantics.
4. A firing probe produces a named diagnostic carrying the violated fact and
   the remedy (repair/refresh the affected cells), never a silent continue.
5. Probe cadence is controllable (per-run default, off/periodic override) and
   probe cost is visible in `smelt explain`.
6. Conformance gate includes fact-violation recipes: a violated declaration is
   caught by its probe, not by wrong output. All standing gates green.

## Out of scope

- Declared source lateness wiring into live scans (belongs to the contract
  lattice's frozen-horizon work).
- New declaration kinds — this outcome hardens the existing ones.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec: the probe obligation rule — per-declaration probe, firing semantics, cadence, admissibility | planned |
| 2 | Probe emitters for FD, bounded_domain, append-only posture, assert_monotonic | pending |
| 3 | `referential_integrity` tripwire wired into the runs that consume the closure narrowing | pending |
| 4 | Runtime wiring: cadence control, firing → named diagnostic + cell remedy marking | pending |
| 5 | Conformance recipes: violated-fact scenarios caught by probes | pending |
| 6 | Surface: explain rendering of probes + cost, docs-site update | pending |

## Decision log

<!-- Dated one-liners appended by plan/implement steps. -->

- 2026-08-10: Outcome activated. Phase table kept as scaffolded (no prior summary to reshape from). Phase 1 plans the probe registry as a spec-parsed standing gate (`crates/smelt-logical/tests/probe_obligation.rs`) so later phases flip Status cells from `not-yet` to `built` under test, rather than the table drifting from the emitters.

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->

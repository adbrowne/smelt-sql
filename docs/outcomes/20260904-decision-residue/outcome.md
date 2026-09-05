# Outcome: Decision residue — implement the 2026-09-04 decision-track calls

**Created:** 2026-09-04
**Status:** active
**Source:** `docs/research/20260904-decision-track.md` (all eight decisions); `docs/research/20260816-open-questions-triage.md` items C (`PartitionGrainForbidsMetrics`, sub-`g_part` suggestion); `docs/outcomes/20260815-incremental-spec-closure-confirm/closure-report.md` rows IS-08, IS-10, IS-20, IS-21, MP-04
**Spec anchors:** `docs/specs/incremental_shapes.md` §"Functions inside partition-grain bodies", §"Run window vs partition granularity", §"Key temporal locality (the time-partitioned output)" route 2, key-grain rule 16; `docs/specs/model_properties.md` §Constraints "Declared lateness is orchestration-only"; `docs/specs/sources.md` §Semantics trust rule; `docs/specs/diagnostics.md` (`PartitionGrainForbidsMetrics`, `KeyedRecurrenceDeclarationMismatch`)

## The outcome

Every product call the 2026-09-04 decision track made has its code. A partition-grain body that
calls `smelt.metric()` refuses with `PartitionGrainForbidsMetrics` from `file_diagnostics()`
(CLI and LSP alike) instead of executing undefined behaviour. A sub-`g_part` run window is
refused with a diagnostic that names the coarsened window the operator could ask for. Route 2 of
key temporal locality derives the key-to-partition dependency from the SQL where decidable and
falls back to the declared FD only where not, with a runnable end-to-end fixture. A declared
`key_recurrence` that disagrees with the derived bound refuses with
`KeyedRecurrenceDeclarationMismatch`, and key-set comparison is order-independent everywhere in
locality reasoning. Declared lateness is read by nothing in plan derivation: the effective-window
summation is gone, the append-only posture probe classifies a row-count increase in a closed
partition as a late arrival rather than a violation, and `smelt explain` prints lateness as an
orchestration fact. The Known Divergence bullets those decisions created are deleted.

## Success criteria (checkable)

1. A partition-grain model whose body contains `smelt.metric(...)` produces
   `PartitionGrainForbidsMetrics` from `file_diagnostics()` (Salsa-direct test and
   `smelt-lsp` `example_workspaces` parity), with an `examples/broken/` fixture; a key-grain or
   full-refresh model with the same call is unaffected.
2. A run window finer than `g_part` is refused with a diagnostic whose text contains the
   model's partition granularity and the exact coarsened `[--event-time-start, --event-time-end)`
   pair that would be accepted; a test asserts the printed pair and that re-running with it
   succeeds.
3. Route 2's key-derived-expression sub-route is consulted before the declared FD: a model whose
   partition projection is provably a per-key constant admits route 2 with no declaration, and
   the maintenance-conformance pool carries a recipe for it; the declared-FD sub-route still
   admits when derivation is undecidable.
4. A declared `key_recurrence` disagreeing with the derived bound refuses at plan time with
   `KeyedRecurrenceDeclarationMismatch` naming both values (`DiagnosticCode` variant, catalogue
   row, test); an agreeing declaration is accepted; a declaration on an underivable model still
   takes the declared route. Every key-set comparison in `locality.rs`/`propagate.rs` is over
   sets (a test permutes column order and asserts an identical verdict).
5. `compute_effective_window` takes no lateness input; `rg -n lateness crates/smelt-logical/src`
   finds no read of `mutation_profile.lateness` outside `smelt explain`'s world-fact printing,
   asserted by a grep gate alongside `walk_coverage`. The per-column `data_latency` frontmatter
   key (`models.md`) is a hard error with a fix-it naming `mutation_profile.lateness` on the
   source, from `file_diagnostics()` with LSP parity; the runtime's `data_latency_days`
   widening path is deleted, and the `examples/` and docs-site carry no use of the key.
6. The append-only posture probe on a closed partition: a row-count increase is reported as a
   late arrival (an observed delta the next run re-processes) and does not fail the run; a
   decrease or fingerprint change still fails it. Both cases tested through the real
   `execute_project` pipeline against DuckDB, and the maintenance-conformance gate gains a
   late-append step kind whose oracle is the full refresh over everything landed.
7. `smelt explain` (text and `--json`) prints `lateness` under world-facts labelled as
   orchestration-only; a doc-sync test pins the label.
8. The Known Divergence bullets naming this outcome in `incremental_shapes.md`,
   `model_properties.md` and `diagnostics.md` are deleted; `/smelt:validate incremental_shapes`,
   `model_properties`, `sources` and `diagnostics` clean.
9. `verify-phase.sh`, `maintenance_conformance`, `statement_parity`, `walk_coverage` and
   `example_diagnostics` green; hardening baseline unchanged or lowered.

## Out of scope

- Any scheduler consumption of lateness (`--auto` finality) — the decision only removes it
  from plan derivation; scheduling work is `scheduler-delta-signatures`, which needs a
  human-reviewed plan first.
- Frozen-per-window membership, closure through a fold, ladder rungs 3–4, the
  `contract`/`tests:` grammar boundary — all decided as deferred or permanent; not touched.
- The `EffectiveWindow`/`BoundResult` merge — decided against.
- Route 2's `IN (SELECT DISTINCT …)` slice predicate against DuckDB's MERGE binder limitation
  — a backend gap, unchanged.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | `PartitionGrainForbidsMetrics`: classifier in the partition-grain admission walk, `DiagnosticCode`, `file_diagnostics()` + LSP parity, broken-example fixture, and its Known Divergence bullet | planned |
| 2 | Sub-`g_part` refusal names the coarsened run window; test asserts the printed pair and that it is accepted | pending |
| 3 | Route 2 derived key-derived-expression sub-route, declared FD as fallback; conformance recipe and end-to-end fixture | pending |
| 4 | `KeyedRecurrenceDeclarationMismatch` (derived authoritative, declared is a check); order-independent key-set comparison with permutation test | pending |
| 5 | Retire per-column `data_latency` (hard error + fix-it, LSP parity); remove lateness from `compute_effective_window` and the runtime widening path; grep gate; explain prints it as orchestration-only | pending |
| 6 | Append-only posture probe: increase = late arrival, decrease/fingerprint = violation; conformance late-append step kind | pending |
| 7 | Delete the remaining divergence bullets (those phases 1-6 did not already close); validate the four specs; all gates green | pending |

## Decision log

- 2026-09-05 — Outcome moved `queued` → `active`. Phase 1 now also deletes its own
  `PartitionGrainForbidsMetrics`-is-unimplemented Known Divergence bullet (and updates the
  `partition_residue_probes` bullet ratchet) instead of leaving it to phase 7: shipping an
  implemented refusal alongside a spec bullet calling it unimplemented would be a false spec at
  the phase-1 commit. Phase 7 narrows to the bullets earlier phases did not close.

## Blocked

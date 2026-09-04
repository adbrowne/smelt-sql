# Decision track — incremental programme product calls (2026-09-04)

Follow-up to `docs/research/20260904-incremental-state-review.md` §"Recommended next sequence"
item 4 and `docs/outcomes/20260815-incremental-spec-closure-confirm/closure-report.md`. The
closure report split the open spec bullets into plain backlog and genuine product calls; this
session decided the product calls. Every decision below lands as a spec diff in the same commit
(spec-first rule); the implementation gaps it creates are queued as
`docs/outcomes/20260904-decision-residue/outcome.md`.

Two of the three items the review named turned out to be already decided on 2026-08-16
(`docs/research/20260816-open-questions-triage.md`) and are implementation gaps only:
`PartitionGrainForbidsMetrics` has no classifier or diagnostic, and the sub-`g_part` refusal does
not yet name the coarsened run window. Both are phases of the residue outcome, not decisions.

| # | Question | Decision | Lands in |
|---|----------|----------|----------|
| 1 | Non-deterministic row-set membership or grouping (IS-05) | **Permanent constraint.** Refusal is the product, already stated as key-grain rule 12. The Known Divergence bullet is deleted; frozen-per-window membership is a Future Extension nobody is obliged to build. | `incremental_shapes.md` §Known Divergences (deleted), §Future Extensions |
| 2 | Skeleton-source closure beyond non-aggregating scopes (MP-10) | **Close the question, keep v1.** The restriction is the decided boundary; widening through a fold is a Future Extension with a trigger (a real model refused for this reason). | `model_properties.md` §Future Extensions |
| 3 | Route 2 key-derived-expression sub-route (IS-20) | **Implement the derived sub-route.** Derive-over-declare: the key-to-partition dependency is derived from the SQL where decidable; the declared FD remains the fallback. | residue outcome phase 3 |
| 4 | `columns.<c>.contract` vs a future column `tests:` block (MP-16) | **Leave deferred.** Nothing forces it until declarative tests are designed. | no change |
| 5 | Declared-vs-derived recurrence precedence; key-set comparison (IS-21) | **Derived wins, declared is a check.** Same posture as `grain:`: the derived recurrence is authoritative; a declaration that disagrees is a diagnostic naming both (`KeyedRecurrenceDeclarationMismatch`). Key-set comparison is order-independent. | `incremental_shapes.md` key-grain rule 16; residue outcome phase 4 |
| 6 | `EffectiveWindow` and `BoundResult` as two walks (MP-02) | **Keep separate, close the question.** Different questions with deliberately different fail-closure; recorded as design. | `model_properties.md` §Design |
| 7 | Declared source lateness (MP-04, sources trust rule, route 1 margin, posture probe) | **Lateness is orchestration, never a plan input.** It is consumed only by scheduling and staleness (`--auto`, when a window is treated as final) and printed by `smelt explain`. It never widens a scan, never gates a probe, never licenses a technique, never changes emitted SQL. Late rows are caught by observed deltas, the ledger and probes. Consequences: `compute_effective_window` must stop summing it; the per-column `data_latency` mechanism is retired unbuilt; the append-only posture probe classifies a row-count increase in a closed partition as a late arrival to re-process, never a violation, and does not consult lateness to decide that; walk-migration-residue phase 4 (probe consults lateness) is removed. | `sources.md` §Semantics trust rule; `model_properties.md` §Constraints; `incremental_shapes.md` route 1 text; residue outcome phase 5 |
| 8 | Ladder rungs 3–4 (IS-31) | **Keep deferred, gated on the change-feed design.** No residue outcome re-triages them before that design exists. | `incremental_shapes.md` §Known Divergences (reworded) |

Not asked: the `docs/plans/20260704-model-updates.md` note calling `latest_value`/`versioned`
"unclear" is stale — `models.md` §Design already records that `versioned` is not carried forward
and `latest_value` collapsed into the key grain. Plans are historical and are not edited.

Why lateness is orchestration-only (decision 7). The equivalence invariant is defined over what
has landed: `incremental_state(S) == full_refresh(inputs ∈ S)`. A static margin cannot make that
true — a row later than the margin still lands — and it cannot be verified (a mis-stated margin
is invisible until a row falls outside it). What makes the invariant true is the plan reacting
to landed data: observed deltas, the ledger's frontier, and the probes. Lateness therefore
answers a different question — *when is a window worth treating as final* — which is the
scheduler's, and mixing it into plan derivation coupled a scheduling hint to correctness.

# Property-discovery catalog (human index)

The machine-readable backlog is `catalog.jsonl` — one JSON object per **cell**. The loop
(`.claude/scripts/property-loop.sh`) works cells top-to-bottom, skipping `done`/`blocked`. This file
is the readable mirror; when they disagree, `catalog.jsonl` wins.

**Cell** = `(construct × source_property × technique × layer)`. See the design for the four links
(`docs/research/20260705-property-discovery-loop.md` §2) and the per-cell routine
(`docs/plans/20260705-property-discovery-loop.md`).

Ordering is load-bearing: **infra (`P0-*`) → seed bugs (`SC-*`) → reachable grid (`G-*`)**. Do not
start a property cell while a `P0` cell it depends on is still `pending`.

| id | construct | source property | technique | layer | hypothesis (expected verdict) |
|---|---|---|---|---|---|
| P0-1 | (infra) | — | — | harness | In-process `execute_project` PBT harness — the gating build |
| P0-2 | (infra) | — | — | harness | Run-schedule generator (append-late / in-place-update between runs) + step-`k` snapshot |
| P0-3 | (infra) | — | — | harness | `EXCEPT ALL` all-columns step-`k` oracle + payload-exclusion rule |
| P0-4 | (infra) | — | — | harness | Generator `MutationProfile` self-check |
| P0-5 | (infra) | — | — | linkA | Abstract contract-safety pre-filter (5 adversarial schedule kinds) |
| P0-6 | (infra) | — | — | linkB | Classification diagnostics (analyzer facts vs DuckDB) + skeleton floor |
| **SC-1** | correlated `EXISTS` (7d attribution) | append-only, late conversion **appended between runs** | window-forward / recompute | linkC | `source_bounds` `(0,0)` fallback clamps the late conversion → **REFUTED = bug** |
| **SC-2** | pass-through + additive agg | clocked **mutable**, in-place update between runs | window-forward | linkC | `input_delta` clocked-`Mutable`→`WindowForward` misses back-dated update → **REFUTED = bug** |
| G-01 | additive agg (SUM/COUNT) | append-only | fold-delta | linkC | HOLDS (happy-path control) |
| G-02 | additive agg | append-only, delta **re-delivered** | fold-delta | linkC | double-count if no dedup ledger — tests smelt idempotency |
| G-03 | idempotent agg (MAX/BOOL_OR) | append-only | fold-delta | linkC | HOLDS under all schedules |
| G-04 | idempotent agg (MIN) | mutable-snapshot | fold-delta | linkC | **REFUTED** (observer semantics) — is smelt over-conservative or unsound? |
| G-05 | inner-join enrichment | mutable dimension | column re-derivation | linkC | REFUTED/CONDITIONAL on horizon derivation |
| G-06 | left-join (null-preservation) | append-only + late right side | recompute-region | linkC | HOLDS for recompute; fold strands the NULL row |
| G-07 | holistic agg (MEDIAN / COUNT DISTINCT) | append-only | fold-delta | linkC | REFUSED (non-monoid) — recompute-only |
| G-08 | running total (ROWS UNBOUNDED PRECEDING) | append-only, late row | fold end-state / stored trajectory | linkC | CONDITIONAL (as-of-run) for the trajectory grain |
| G-09 | `UNION ALL` | append-only both arms | recompute-region | linkC | HOLDS — bound derivation composes across arms |
| G-10 | join fan-out on **composite** unique key | append-only | column re-derivation | linkB | `join_shape` single-column-only → mislabeled fan-out |

Verdicts land in `ledger.md`; every REFUTED/CONDITIONAL also lands one line in `unsupported.md`
(the admission-matrix negative space).

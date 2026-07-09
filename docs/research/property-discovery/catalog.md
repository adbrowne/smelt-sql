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
| G-10 | join fan-out on **composite** unique key | append-only | column re-derivation | linkB | **FIXED** — `JoinContext`/`fan_out` now express composite (multi-column) unique keys; a matched superset of a declared key-set proves `OneToOne`, a strict subset still fails closed to `OneToMany` (Phase MP10, `docs/plans/20260707-maintenance-plan-impl.md`) |
| **SC-3** | `MAX(ts)` as event_time + `assert_monotonic` | append-only, late row moves a group's MAX into window | window-forward pushdown | linkC | aggregate clock hits the *Undecidable* arm the declaration may widen → unsound admission → **REFUTED = bug** (B1, `20260707-property-event-time-monotonicity.md`) |
| **SC-4** | stacked window frames across CTEs (7d inner, 3d outer) | append-only, late row 8–10d back | widened scan + clamp | linkC | **REFUTED = bug, FIXED** — max-merged whole-text derivation gave 7d where series composition needs 10d (maintained 2 vs oracle 101); bounds now compose through the property walk (series-add / parallel-max / sibling-slack carry; see ledger SC-4 for the two coverage residues) (B2, `20260707-property-bounded-reach.md`) |
| **SC-5** | window frame + declared `source_lateness` | append-only, late row inside lateness+reach but outside max | widened scan + clamp | linkC | **site fixed; NOT reproducible at linkC** — max→sum corrected in `compute_effective_window`, but its value feeds `filter_start/filter_end` which no live execute path consumes; lateness becomes a scan obligation only under the unbuilt tail-rewrite transform (see ledger SC-5) (B3, same doc) |
| **SC-6** | declared FD over `UNION ALL` (same key, different value per branch) | append-only both arms | once-write / FD admission | linkB | **CONFIRMED = bug, FIXED** — `FunctionalDependency.key` was parsed but never read and no union analysis existed, so a declared FD over a bare `UNION ALL` widened to `Constant`; the walk now derives a per-model `PropertyVector` (grain + set-op barrier) and `functional_dependency_verdict_over_vector` reads the declared key, refusing the union case while still widening a genuinely-undecidable single-branch declaration (B4, `20260707-property-per-key-constancy.md`; see ledger SC-6) |
| **SC-7** | unaligned `DISTINCT`/`HAVING` **inside a CTE body** | append-only, late row | batched admission + partition rewrite | linkC | **REFUTED = bug, FIXED** — CTE bodies were exempt from the admission walks (fail-**open** hole, B5, `20260707-property-partition-alignment.md`); admission now judges every walk-enumerated scope and refuses the model (see ledger SC-7) |
| **SC-8** | `BIT_XOR` combiner classification | mutable (retraction schedule) | fold-delta with retraction | linkB | `discriminants.rs` says `needs_inverse: true`, spec rung 3 says self-inverse group — spec/code drift (B6, `20260707-property-aggregate-algebra.md`) |
| **SC-9** | UDF/opaque function in a skeleton position | append-only | determinism taint gate | linkB | unknown functions default **deterministic** outside the event-time trace → fail-open taint gate → mis-addressed replay (B7, `20260707-property-determinism.md`) |

Verdicts land in `ledger.md`; every REFUTED/CONDITIONAL also lands one line in `unsupported.md`
(the admission-matrix negative space).

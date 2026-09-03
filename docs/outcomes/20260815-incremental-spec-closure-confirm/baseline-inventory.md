# Baseline bullet inventory — 2026-08-15 program

Reconstructed from git history at the program baseline commit `03a431f3`
(`outcome(20260815-definition-delta-migrate): scaffold`, the first commit of the
2026-08-15 program). Enumerates every `§Known Divergences / Open Questions` bullet in the
four anchor specs (`definition_deltas.md`, `incremental_models.md`,
`incremental_shapes.md`, `model_properties.md`) as they stood at that commit — the
denominator phase 2 classifies against. Regenerate the machine artifact with:

```
bash docs/outcomes/20260815-incremental-spec-closure-confirm/extract-baseline.sh > docs/outcomes/20260815-incremental-spec-closure-confirm/baseline-inventory.tsv
```

Verify this file still matches the extractor with:

```
bash docs/outcomes/20260815-incremental-spec-closure-confirm/check-inventory.sh
```

`Disposition` is one of `closed <sha>` / `open` / `drifted` / `residue` (phase 2's classification
vocabulary; see `phases/02-plan.md`), filled in against the current (HEAD) spec text and each
owning outcome's decision log. `current-inventory.tsv` (generated with
`bash extract-baseline.sh HEAD`) is the machine artifact phase 2 joined against.

## definition_deltas.md

| ID | Subsection | Bullet (bold lead-in) | Open Question? | Disposition |
|----|------------|------------------------|-----------------|-------------|
| DD-01 | - | The definition-delta synthesis layer is unwired. (L405) | No | closed 1c7bffea |
| DD-02 | - | `smelt migrate` does not exist (L411) | No | closed 1c7bffea |
| DD-03 | - | The atomicity rule is conditional in practice. (L416) | No | closed 129984d9 |
| DD-04 | - | The conformance harness has no definition-edit step kind yet (L424) | No | closed 7b0bf461 |
| DD-05 | - | No approval store exists (L426) | No | closed 0d5cb0e6 |
| DD-06 | - | Open question — plan-hash scope. (L427) | Yes | closed c186b5fb |
| DD-07 | - | The diagnostic name is narrower than its rule. (L435) | No | closed eab2aa72 |

## incremental_models.md

| ID | Subsection | Bullet (bold lead-in) | Open Question? | Disposition |
|----|------------|------------------------|-----------------|-------------|
| IM-01 | - | The scheduler does not yet consume delta signatures end to end. (L1729) | No | open |
| IM-02 | - | `smelt explain` does not yet print the delta-signature headline (L1740) | No | open |
| IM-03 | - | Per-cell `deferral` is not yet scheduled (L1744) | No | closed 64f20a29 |
| IM-04 | - | `diff_patch` over the region `DeleteInsert` default has no runtime lowering (L1748) | No | drifted (reworded: membership-sensitive case now lowered by 64f20a29 — current text is "has a runtime lowering for the membership-sensitive case only"; plain windowed/partition-grain default still `backend_default`, gap survives under new wording) |
| IM-05 | - | Frontmatter-time grain checking has one narrow gap (L1751) | No | closed 6badd725 |
| IM-06 | - | The write-pin equivalence factor is structural only (L1754) | No | closed 6f8b8083 |
| IM-07 | - | An inadmissible write-*variant* pin has no pre-execution gate (L1757) | No | closed 6f8b8083 |
| IM-08 | - | Observed-delta consumption is partial (L1761) | No | closed a14a75c3 |
| IM-09 | - | No execution technique keys off a maintained-model creation cell (L1766) | No | closed 6badd725 |
| IM-10 | - | Plan-consumer gaps (L1769) | No | closed 242ec4e1 |
| IM-11 | - | Emission remainders (L1775) | No | open |
| IM-12 | - | Locality and diagnostic residues on the maintenance-plan proofs (L1778) | No | closed e885bd6e |
| IM-13 | - | The ledger's warehouse substrate is DuckDB-only (Open Question) (L1790) | Yes | drifted (still present, no longer tagged Open Question — Spark deferral decided, not undecided, per `docs/research/20260816-open-questions-triage.md`; underlying DuckDB-only gap unchanged) |
| IM-14 | - | Graph-layer gaps (L1793) | No | drifted (still present but narrowed: self-edges closed 87df1dcc, key-addressed edge admission closed 15b4073a, `--select` scoping closed acb5e66d; only key-temporal-locality-vs-admission residue remains in current wording) |
| IM-15 | - | Delta detection for `--since-upstream` is explicit-only in v1 (L1799) | No | open |
| IM-16 | - | Straddle attribution without locality is scoped out of the ledger's v1 (L1802) | No | open |
| IM-17 | - | No out-of-band-edit tripwire (Open Question) (L1805) | Yes | closed 2e1f6d19 |
| IM-18 | - | A proposed `on_column_add: backfill \| leave_null \| recompute` policy knob (Open Question) (L1807) | Yes | closed 2e1f6d19 |
| IM-19 | - | The derived model-wide horizon is under construction (L1809) | No | open |
| IM-20 | - | Override-ladder reach (Open Question) (L1811) | Yes | closed 7fc342d3 |
| IM-21 | - | docs-site coverage of the plan's CLI surface is partial (Open Question) (L1817) | Yes | closed 41deabef |
| IM-22 | - | A group merged across two mutable inputs has no group-merge-provenance policy (Open Question) (L1819) | Yes | closed 2e1f6d19 |
| IM-23 | - | `change_feed` sources never get an `UpstreamMutation` cell (Open Question) (L1822) | Yes | drifted (a cell now exists per outcome phase 28c; current wording narrows to "always re-derives from the full input" — original "never gets a cell" claim is now false) |
| IM-24 | - | `INTERSECT`/`EXCEPT` are unclassified set operations (L1825) | No | closed 4f8b9c66 |
| IM-25 | - | Conditional-maintenance gaps (L1829) | No | open (narrowed to the one remaining `supports_fingerprint_sidecar` residue; other listed items closed) |

## incremental_shapes.md

| ID | Subsection | Bullet (bold lead-in) | Open Question? | Disposition |
|----|------------|------------------------|-----------------|-------------|
| IS-01 | The partition grain | One classification call site reads the outer SQL body (L1074) | No | closed 98393e25 |
| IS-02 | The partition grain | The window-function batch-safety check runs on unexpanded outer SQL (L1078) | No | closed 98393e25 |
| IS-03 | The partition grain | Per-source clamp observability is partly emitted (Open Question) (L1081) | Yes | closed 3a6e995a |
| IS-04 | The partition grain | Per-column `data_latency` is unimplemented (L1084) | No | open |
| IS-05 | The partition grain | Non-deterministic row-set-membership or grouping is out of scope (L1086) | No | open |
| IS-06 | The partition grain | CTE-only `event_time_column` references are not yet detected (L1089) | No | closed 5b434b0c |
| IS-07 | The partition grain | Schema evolution on the partition grain is largely a definition delta now (L1092) | Yes | open |
| IS-08 | The partition grain | The `smelt.metric()` interaction is unspecified (Open Question) (L1097) | Yes | drifted (OQ decided 2026-08-16 — refuse the combination; the decision is landed but no classifier/diagnostic implements it yet, so a still-open gap survives as "`PartitionGrainForbidsMetrics` refusal is unimplemented") |
| IS-09 | The partition grain | Per-`ModelDef` overrides for generator-emitted models are not part of the closed field set in v1. (L1099) | No | closed 7fc38775 |
| IS-10 | The partition grain | `g_run >= g_part` auto-coarsening is not implemented (Open Question) (L1101) | Yes | drifted (OQ decided 2026-08-16 — reject with suggestion; landed, but the rejection doesn't yet name the coarsened window, so a narrower still-open gap survives) |
| IS-11 | The partition grain | Monotone-integer `partition_column` has no end-to-end run (L1103) | No | closed 0cd4cbf4 |
| IS-12 | The key grain | A window-forward keyed run with no event-time window silently full-refreshes instead of refusing (L1110) | No | closed af20dfe3 |
| IS-13 | The key grain | The once-write classifier has no nullability route around the fallback case (L1116) | No | open |
| IS-14 | The key grain | A re-run-tolerant keyed model keeps no ledger at all unless additive-graded (Open Question) (L1125) | Yes | closed af20dfe3 |
| IS-15 | The key grain | Snapshot-reconcile admits at most one unclocked source in the FROM clause (Open Question) (L1129) | Yes | drifted (moved to §Future Extensions "Multi-source snapshot-reconcile"; `KeyedSnapshotPostureUnsupported` still refuses ≥2 unclocked sources — decision recorded 2026-08-16 to defer, not fixed) |
| IS-16 | The key grain | `KeyedRetractableContribution` has no implementation (Open Question) (L1132) | Yes | closed 7871e97d |
| IS-17 | The key grain | `safety_overrides:` on a key-addressed model is not a hard error (L1134) | No | closed af20dfe3 |
| IS-18 | The key grain | The reconciliation ledger's fold is transactional on DuckDB only (Open Question) (L1137) | Yes | drifted (still present, bold lead-in reworded with an added clause about the ledger table's existence; same underlying DuckDB-only gap. This is the bullet `docs/outcomes/20260815-keyed-grain-residue` phase 3 ("Transactional ledger fold on every shipped backend") is about, and that phase is `**Status:** blocked` with its own decision log stating the criterion is "deliberately left unmet" — no outcome falsely claims closure, so this is not `residue` either. A prior phase's decision log mislabeled this bullet as `IS-24`; `IS-24` is a different bullet — see below.) |
| IS-19 | The key grain | `smelt explain` prints neither the per-column guarantee ledger nor the derivable forward reach (Open Question) (L1140) | Yes | open |
| IS-20 | The key grain | Key temporal locality route 2 admits only a declared functional dependency (Open Question) (L1143) | Yes | open |
| IS-21 | The key grain | Locality machinery gaps (L1146) | No | open |
| IS-22 | The key grain | The derived execution postures are internal, and one of the three is not derived at all (L1155) | No | drifted (`smelt explain` now prints `Execution postures:` per `docs/outcomes/20260815-keyed-grain-residue` phase 4 — `crates/smelt-cli/src/explain.rs:770-808`; current wording narrows to the still-unbuilt "order-independence is not yet acted on" optimization) |
| IS-23 | The key grain | The generative conformance pool cannot stage NULL payloads (Open Question) (L1160) | Yes | closed f2b412e0 |
| IS-24 | The key grain | Locality open questions (Open Question) (L1165) | Yes | drifted (moved to §Future Extensions "Deletion-adjacent locality relaxations"; same three sub-gaps — recurrence-bound slice pruning, granularity-equality relaxation, slice-scoped deletion — decision 2026-08-16 to defer rather than implement) |
| IS-25 | The key grain | The pattern functions (`smelt.latest`, `smelt.once`, `smelt.current`) are unshipped (L1169) | No | drifted (bold lead-in reworded to "The pattern-function template file does not exist"; same unshipped gap, still present) |
| IS-26 | The key grain | Driver granularity is `day`/`week` only (Open Question) (L1173) | Yes | drifted (moved to §Future Extensions "Wider driver granularities"; still day/week only) |
| IS-27 | The key grain | `--auto` staleness fidelity for all-invertible models is conservative in v1 (Open Question) (L1175) | Yes | drifted (moved to §Future Extensions "Exact `--auto` staleness…"; same gap) |
| IS-28 | The key grain | Self-referential keyed models are rejected (Open Question) (L1177) | Yes | drifted (moved to §Future Extensions "Self-referential keyed models"; still rejected) |
| IS-29 | The key grain | Run-pinning alignment is deferred (Open Question) (L1179) | Yes | drifted (OQ decided 2026-08-16 — both grains now run `NOW()`/`CURRENT_*` as-is, no pinning; current bullet "`NOW()`/`CURRENT_*` are still rejected in keyed models" describes the residual divergence with a decision record instead of as an open question) |
| IS-30 | The key grain | Key deletion is unresolved beyond retention (L1181) | No | closed 2e1f6d19 |
| IS-31 | The key grain | Ladder rungs 3–4 remain specified ahead of this profile's use of them (L1186) | No | open |
| IS-32 | The key grain | The `key_per_partition` grain derives no plan (L1191) | No | open |

## model_properties.md

| ID | Subsection | Bullet (bold lead-in) | Open Question? | Disposition |
|----|------------|------------------------|-----------------|-------------|
| MP-01 | - | Several declared proofs have no consumer wired yet. (L322) | No | open |
| MP-02 | - | `EffectiveWindow` and `BoundResult` remain two separate walks (Open Question). (L331) | Yes | open |
| MP-03 | - | The composition walk is not yet the sole source of every property. (L336) | No | open |
| MP-04 | - | Declared source lateness reaches no live scan today (Open Question) (L344) | Yes | open |
| MP-05 | - | `cumulative.rs`'s whole-SQL window-function admission scan is not yet classified onto the walk (L347) | No | open |
| MP-06 | - | `INTERSECT`/`EXCEPT` are unclassified for filter distribution (L352) | No | drifted (scope narrowed by 4f8b9c66, which built per-set-operation-arm mutation-sensitivity classification independent of this gap; the filter-distribution gap itself is still open, wording reflects the narrower scope) |
| MP-07 | - | Additive-only model-diff can't detect a semantic change under an unchanged expression (Open Question) (L355) | Yes | open |
| MP-08 | - | A keyed-grain output poses no partition-locality question (L359) | No | closed 43a25731 |
| MP-09 | - | `MaintenanceSkeletonColumnAdded` is not yet surfaced as an LSP/CLI diagnostic ahead of a run (L363) | No | closed dec07e11 |
| MP-10 | - | Skeleton-source closure v1 is restricted to non-aggregating enrichment scopes (Open Question) (L367) | Yes | open |
| MP-11 | - | Only one maintenance-cell route consults a declared-RI closure today (L371) | No | open |
| MP-12 | - | Fingerprint projection (P4) has no consumer yet (L376) | No | open |
| MP-13 | - | The append-only posture probe does not consult declared lateness (L378) | No | open |
| MP-14 | - | `SourceUniqueKeyViolated` remains the one probe-registry row with no emitter at all (Open Question) (L383) | Yes | open |
| MP-15 | - | Output-delta shape is derived, typed onto propagation edges, and acted on by dirt propagation, but the keyed dirt-set remains symbolic. (L386) | No | open |
| MP-16 | - | The grammar boundary between `columns.<c>.contract` and a future column `tests:` block is deliberately deferred (Open Question) (L394) | Yes | open |

## Classification summary

Counts per class (80 bullets total):

| Spec | closed | open | drifted | residue | total |
|------|-------:|-----:|--------:|--------:|------:|
| definition_deltas | 7 | 0 | 0 | 0 | 7 |
| incremental_models | 14 | 7 | 4 | 0 | 25 |
| incremental_shapes | 12 | 9 | 11 | 0 | 32 |
| model_properties | 2 | 13 | 1 | 0 | 16 |
| **Total** | **35** | **29** | **16** | **0** | **80** |

No bullet classified as `residue`: every bullet an owning outcome's decision log claims to have
closed was independently confirmed against the repo (code, tests, or a landed decision record),
and `IS-18` — the one bullet flagged at phase-1 as a residue candidate (the transactional
merge-ledger fold, whose owning outcome `20260815-keyed-grain-residue` is blocked on exactly this)
— turned out not to need the `residue` class: that outcome's own decision log honestly states the
criterion is "deliberately left unmet" rather than claiming closure, so the bullet is accurately
`drifted` (reworded, still an open gap), not a false-closure residue. See its row for the
correction of a mislabeling in this outcome's own phase-1/phase-2-planning decision log entries,
which named `IS-24` instead of `IS-18` as the transactional-fold bullet — `IS-24` ("Locality open
questions") is a different bullet, about recurrence-bound slice pruning / granularity relaxation /
slice-scoped deletion, also now `drifted` (moved to §Future Extensions).

`drifted` (16 bullets) worklist for phase 3, split by the two ways spec text goes stale:
- **Reworded but still an accurate, still-open gap** (spec text is fine as documentation, no fix
  needed — informational only): IM-04, IM-13, IM-14, IM-23, IS-08, IS-10, IS-18, IS-22, IS-25,
  IS-29, MP-06.
- **Moved from `## Known Divergences` to `## Future Extensions`** (deliberate 2026-08-16
  reclassification, not a fix — spec text is accurate in its new location): IS-15, IS-24, IS-26,
  IS-27, IS-28.

No row needed the `residue` class, so phase 3 has no owning-outcome reopen to do; its job is
limited to spot-checking whether any of the 13 `drifted` rows above read as accidentally-stale
*in their current wording* (not just relocated) — none of the current-inventory.tsv text checked
in this phase looked stale on inspection, but phase 3 owns the final call per the plan.

## Extraction notes

- The extractor identifies a bullet's "Open Question?" flag by scanning the full bullet body
  (not just the bold lead-in) for the case-insensitive phrase "open question", after collapsing
  whitespace — several bullets wrap `(Open` and `Question)` across a line break, which a naive
  single-line-at-a-time grep misses. This inventory's counts therefore differ from the
  pre-sampled estimate in `phases/01-plan.md` (which undercounted for the same reason):
  `incremental_models` has 7 Open-Question bullets, not 6 (`IM-18`, `IM-22` were the two the
  line-wrap bug hid); `incremental_shapes` has 16, not 13 (`IS-14`, `IS-20`, `IS-27`); everything
  else matches the sample. Per §Tasks item 5, the extractor is trusted over the sample.
- `definition_deltas` `DD-06` ("Open question — plan-hash scope.") states its open-question-ness
  in its own lead-in prose rather than a trailing `(Open Question)` tag; the extractor's
  free-text scan still flags it correctly.
- No bullet in any of the four specs required dropping to represent as a single row: every
  `- **…**` top-level bullet is one table row. Bullets with an internal semicolon-separated list
  (e.g. `IS-21` "Locality machinery gaps", `MP-01` "Several declared proofs…") are single bullets
  with compound prose, not nested markdown sub-bullets, so they extract cleanly as one row each.
- The `(L<n>)` suffix on each Bullet cell is the bullet's starting line number in the baseline
  spec file, carried through from the TSV's 5th column, so a row can be cross-checked against
  `git show 03a431f3:docs/specs/<spec>.md` directly.

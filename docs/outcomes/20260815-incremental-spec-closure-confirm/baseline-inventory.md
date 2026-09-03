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

`Disposition` is `TBD (phase 2)` throughout — filling it in is phase 2's job, not this one's.

## definition_deltas.md

| ID | Subsection | Bullet (bold lead-in) | Open Question? | Disposition |
|----|------------|------------------------|-----------------|-------------|
| DD-01 | - | The definition-delta synthesis layer is unwired. (L405) | No | TBD (phase 2) |
| DD-02 | - | `smelt migrate` does not exist (L411) | No | TBD (phase 2) |
| DD-03 | - | The atomicity rule is conditional in practice. (L416) | No | TBD (phase 2) |
| DD-04 | - | The conformance harness has no definition-edit step kind yet (L424) | No | TBD (phase 2) |
| DD-05 | - | No approval store exists (L426) | No | TBD (phase 2) |
| DD-06 | - | Open question — plan-hash scope. (L427) | Yes | TBD (phase 2) |
| DD-07 | - | The diagnostic name is narrower than its rule. (L435) | No | TBD (phase 2) |

## incremental_models.md

| ID | Subsection | Bullet (bold lead-in) | Open Question? | Disposition |
|----|------------|------------------------|-----------------|-------------|
| IM-01 | - | The scheduler does not yet consume delta signatures end to end. (L1729) | No | TBD (phase 2) |
| IM-02 | - | `smelt explain` does not yet print the delta-signature headline (L1740) | No | TBD (phase 2) |
| IM-03 | - | Per-cell `deferral` is not yet scheduled (L1744) | No | TBD (phase 2) |
| IM-04 | - | `diff_patch` over the region `DeleteInsert` default has no runtime lowering (L1748) | No | TBD (phase 2) |
| IM-05 | - | Frontmatter-time grain checking has one narrow gap (L1751) | No | TBD (phase 2) |
| IM-06 | - | The write-pin equivalence factor is structural only (L1754) | No | TBD (phase 2) |
| IM-07 | - | An inadmissible write-*variant* pin has no pre-execution gate (L1757) | No | TBD (phase 2) |
| IM-08 | - | Observed-delta consumption is partial (L1761) | No | TBD (phase 2) |
| IM-09 | - | No execution technique keys off a maintained-model creation cell (L1766) | No | TBD (phase 2) |
| IM-10 | - | Plan-consumer gaps (L1769) | No | TBD (phase 2) |
| IM-11 | - | Emission remainders (L1775) | No | TBD (phase 2) |
| IM-12 | - | Locality and diagnostic residues on the maintenance-plan proofs (L1778) | No | TBD (phase 2) |
| IM-13 | - | The ledger's warehouse substrate is DuckDB-only (Open Question) (L1790) | Yes | TBD (phase 2) |
| IM-14 | - | Graph-layer gaps (L1793) | No | TBD (phase 2) |
| IM-15 | - | Delta detection for `--since-upstream` is explicit-only in v1 (L1799) | No | TBD (phase 2) |
| IM-16 | - | Straddle attribution without locality is scoped out of the ledger's v1 (L1802) | No | TBD (phase 2) |
| IM-17 | - | No out-of-band-edit tripwire (Open Question) (L1805) | Yes | TBD (phase 2) |
| IM-18 | - | A proposed `on_column_add: backfill \| leave_null \| recompute` policy knob (Open Question) (L1807) | Yes | TBD (phase 2) |
| IM-19 | - | The derived model-wide horizon is under construction (L1809) | No | TBD (phase 2) |
| IM-20 | - | Override-ladder reach (Open Question) (L1811) | Yes | TBD (phase 2) |
| IM-21 | - | docs-site coverage of the plan's CLI surface is partial (Open Question) (L1817) | Yes | TBD (phase 2) |
| IM-22 | - | A group merged across two mutable inputs has no group-merge-provenance policy (Open Question) (L1819) | Yes | TBD (phase 2) |
| IM-23 | - | `change_feed` sources never get an `UpstreamMutation` cell (Open Question) (L1822) | Yes | TBD (phase 2) |
| IM-24 | - | `INTERSECT`/`EXCEPT` are unclassified set operations (L1825) | No | TBD (phase 2) |
| IM-25 | - | Conditional-maintenance gaps (L1829) | No | TBD (phase 2) |

## incremental_shapes.md

| ID | Subsection | Bullet (bold lead-in) | Open Question? | Disposition |
|----|------------|------------------------|-----------------|-------------|
| IS-01 | The partition grain | One classification call site reads the outer SQL body (L1074) | No | TBD (phase 2) |
| IS-02 | The partition grain | The window-function batch-safety check runs on unexpanded outer SQL (L1078) | No | TBD (phase 2) |
| IS-03 | The partition grain | Per-source clamp observability is partly emitted (Open Question) (L1081) | Yes | TBD (phase 2) |
| IS-04 | The partition grain | Per-column `data_latency` is unimplemented (L1084) | No | TBD (phase 2) |
| IS-05 | The partition grain | Non-deterministic row-set-membership or grouping is out of scope (L1086) | No | TBD (phase 2) |
| IS-06 | The partition grain | CTE-only `event_time_column` references are not yet detected (L1089) | No | TBD (phase 2) |
| IS-07 | The partition grain | Schema evolution on the partition grain is largely a definition delta now (L1092) | Yes | TBD (phase 2) |
| IS-08 | The partition grain | The `smelt.metric()` interaction is unspecified (Open Question) (L1097) | Yes | TBD (phase 2) |
| IS-09 | The partition grain | Per-`ModelDef` overrides for generator-emitted models are not part of the closed field set in v1. (L1099) | No | TBD (phase 2) |
| IS-10 | The partition grain | `g_run >= g_part` auto-coarsening is not implemented (Open Question) (L1101) | Yes | TBD (phase 2) |
| IS-11 | The partition grain | Monotone-integer `partition_column` has no end-to-end run (L1103) | No | TBD (phase 2) |
| IS-12 | The key grain | A window-forward keyed run with no event-time window silently full-refreshes instead of refusing (L1110) | No | TBD (phase 2) |
| IS-13 | The key grain | The once-write classifier has no nullability route around the fallback case (L1116) | No | TBD (phase 2) |
| IS-14 | The key grain | A re-run-tolerant keyed model keeps no ledger at all unless additive-graded (Open Question) (L1125) | Yes | TBD (phase 2) |
| IS-15 | The key grain | Snapshot-reconcile admits at most one unclocked source in the FROM clause (Open Question) (L1129) | Yes | TBD (phase 2) |
| IS-16 | The key grain | `KeyedRetractableContribution` has no implementation (Open Question) (L1132) | Yes | TBD (phase 2) |
| IS-17 | The key grain | `safety_overrides:` on a key-addressed model is not a hard error (L1134) | No | TBD (phase 2) |
| IS-18 | The key grain | The reconciliation ledger's fold is transactional on DuckDB only (Open Question) (L1137) | Yes | TBD (phase 2) |
| IS-19 | The key grain | `smelt explain` prints neither the per-column guarantee ledger nor the derivable forward reach (Open Question) (L1140) | Yes | TBD (phase 2) |
| IS-20 | The key grain | Key temporal locality route 2 admits only a declared functional dependency (Open Question) (L1143) | Yes | TBD (phase 2) |
| IS-21 | The key grain | Locality machinery gaps (L1146) | No | TBD (phase 2) |
| IS-22 | The key grain | The derived execution postures are internal, and one of the three is not derived at all (L1155) | No | TBD (phase 2) |
| IS-23 | The key grain | The generative conformance pool cannot stage NULL payloads (Open Question) (L1160) | Yes | TBD (phase 2) |
| IS-24 | The key grain | Locality open questions (Open Question) (L1165) | Yes | TBD (phase 2) |
| IS-25 | The key grain | The pattern functions (`smelt.latest`, `smelt.once`, `smelt.current`) are unshipped (L1169) | No | TBD (phase 2) |
| IS-26 | The key grain | Driver granularity is `day`/`week` only (Open Question) (L1173) | Yes | TBD (phase 2) |
| IS-27 | The key grain | `--auto` staleness fidelity for all-invertible models is conservative in v1 (Open Question) (L1175) | Yes | TBD (phase 2) |
| IS-28 | The key grain | Self-referential keyed models are rejected (Open Question) (L1177) | Yes | TBD (phase 2) |
| IS-29 | The key grain | Run-pinning alignment is deferred (Open Question) (L1179) | Yes | TBD (phase 2) |
| IS-30 | The key grain | Key deletion is unresolved beyond retention (L1181) | No | TBD (phase 2) |
| IS-31 | The key grain | Ladder rungs 3–4 remain specified ahead of this profile's use of them (L1186) | No | TBD (phase 2) |
| IS-32 | The key grain | The `key_per_partition` grain derives no plan (L1191) | No | TBD (phase 2) |

## model_properties.md

| ID | Subsection | Bullet (bold lead-in) | Open Question? | Disposition |
|----|------------|------------------------|-----------------|-------------|
| MP-01 | - | Several declared proofs have no consumer wired yet. (L322) | No | TBD (phase 2) |
| MP-02 | - | `EffectiveWindow` and `BoundResult` remain two separate walks (Open Question). (L331) | Yes | TBD (phase 2) |
| MP-03 | - | The composition walk is not yet the sole source of every property. (L336) | No | TBD (phase 2) |
| MP-04 | - | Declared source lateness reaches no live scan today (Open Question) (L344) | Yes | TBD (phase 2) |
| MP-05 | - | `cumulative.rs`'s whole-SQL window-function admission scan is not yet classified onto the walk (L347) | No | TBD (phase 2) |
| MP-06 | - | `INTERSECT`/`EXCEPT` are unclassified for filter distribution (L352) | No | TBD (phase 2) |
| MP-07 | - | Additive-only model-diff can't detect a semantic change under an unchanged expression (Open Question) (L355) | Yes | TBD (phase 2) |
| MP-08 | - | A keyed-grain output poses no partition-locality question (L359) | No | TBD (phase 2) |
| MP-09 | - | `MaintenanceSkeletonColumnAdded` is not yet surfaced as an LSP/CLI diagnostic ahead of a run (L363) | No | TBD (phase 2) |
| MP-10 | - | Skeleton-source closure v1 is restricted to non-aggregating enrichment scopes (Open Question) (L367) | Yes | TBD (phase 2) |
| MP-11 | - | Only one maintenance-cell route consults a declared-RI closure today (L371) | No | TBD (phase 2) |
| MP-12 | - | Fingerprint projection (P4) has no consumer yet (L376) | No | TBD (phase 2) |
| MP-13 | - | The append-only posture probe does not consult declared lateness (L378) | No | TBD (phase 2) |
| MP-14 | - | `SourceUniqueKeyViolated` remains the one probe-registry row with no emitter at all (Open Question) (L383) | Yes | TBD (phase 2) |
| MP-15 | - | Output-delta shape is derived, typed onto propagation edges, and acted on by dirt propagation, but the keyed dirt-set remains symbolic. (L386) | No | TBD (phase 2) |
| MP-16 | - | The grammar boundary between `columns.<c>.contract` and a future column `tests:` block is deliberately deferred (Open Question) (L394) | Yes | TBD (phase 2) |

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

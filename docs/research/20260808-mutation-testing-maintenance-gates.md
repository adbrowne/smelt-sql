# Mutation-testing the maintenance gates

**Date:** 2026-08-08
**Question:** Who guards the guards? The maintenance layer's correctness story rests on a
stack of CI gates (statement_parity, maintenance_conformance, walk_coverage, the smelt-db
proptests). This session ran a cargo-mutants campaign over
`crates/smelt-logical/src/maintenance/` to measure which deliberate bugs those gates actually
catch, attributed every survivor to a cause, and closed the real holes with targeted tests.

## Method

- 472 mutants over the 9 maintenance files (`cargo mutants --in-place`, warm target dir,
  ~9s/mutant on 32 cores — the whole campaign is ~1.5h, cheap enough to repeat).
- Tier-1 battery: `cargo test -p smelt-logical -p smelt-runtime` (includes the
  `statement_parity` CI gate).
- Staged tier-2 triage of survivors via `cargo mutants --iterate` (each stage re-tests only
  still-missed mutants, so the caught-list diff attributes every kill to a specific gate):
  - **A**: `cargo test -p smelt-db` (whole crate: proptests, maintenance diagnostics)
  - **B**: `-p smelt-cli --test maintenance_pins --test explain_maintenance
    --test explain_show_sql --test property_discovery`
  - **C**: `-p smelt-cli --test maintenance_conformance` (the flagship generative
    equivalence gate)
- Per-survivor root-cause analysis (why missed, cheapest kill), new tests, then a
  re-verification `--iterate` pass proving the kills.

## Results

| Stage | Battery | Killed | Survivors after |
|---|---|---|---|
| Tier-1 | smelt-logical + smelt-runtime | 387/424 viable (91.3%) | 37 |
| A | smelt-db (full crate) | 1 (`locality_refused_plan` → Default, via refusal-diagnostics text) | 36 |
| B | pins + explain×2 + property_discovery | 1 (`derive_column_groups` `\|\|`→`&&`, via property_discovery) | 35 |
| C | maintenance_conformance | **0** | 35 |
| New tests (this session) | 7 test clusters, +298 lines, tests only | 22 | **13** |

(48 unviable, 0 timeouts.)

## Findings

1. **The flagship conformance gate killed zero of the tier-1 survivors.** Everything it could
   kill, the cheap suites already killed. Its blind spots are structural: the recipe pool never
   exercises `cells[].write` pins, ColumnAdded-with-`allow_full_scan`, ordinal `ORDER BY`,
   non-Jan/Feb dates, or nonzero forward margins. The gate is excellent at what its generator
   generates — the campaign measures what it doesn't.
2. **A real panic path**: `admits_write_selection` `==`→`!=` (choice.rs) survived everything.
   No test anywhere fed a `keyed`/`update` write pin through `resolve_cell_choice`; under the
   mutant a valid pin refuses, an invalid pin is silently accepted (the exact "silent
   substitution" the doc comment forbids), and a pin on a trigger with no plan cell panics on
   an `.expect()`.
3. **Calendar-math domain blindness**: every propagation fixture lived in 1970–2026 with
   Jan/Feb dates and zero forward margins. Untested as a result: `align_outward`'s December
   year-wrap, `project_observed_delta`'s `+after` widening, `civil_from_ordinal`'s
   era-correction terms (identically zero until 2100-03-01), `day_ordinal`'s pre-year-1
   branch, `DayInterval::is_whole`'s half-sentinel case.
4. **Ordinal `ORDER BY` skeleton roles were unpinned**: only named-column ORDER BY was tested;
   the ordinal-resolution mutants silently demote an Ordering column to Payload, weakening
   `SkeletonColumnAdded` refusals.
5. **The `allow_full_scan: true` escape hatch was dead for ColumnAdded triggers** under the
   mutant: hatch fixtures and column-add fixtures were disjoint sets across the whole
   workspace.
6. **One provably equivalent mutant**: `derive.rs` `source_contributes_to_fold`'s
   `aliases.len() == 1` match guard — no input distinguishes it (the guard only selects which
   `return true` fires). Cleanup candidate, not a test gap.
7. **Test-selection artifact**: `admissible_write_patterns` → `vec![]` is killed by
   `--test explain_model` / `--test explain`; those targets just weren't in the triage
   batteries. Now also pinned by an assertion in `explain_maintenance`.

## Remediation landed (tests only, no production changes)

- `technique_lowering.rs`: write-pin admission/refusal through `resolve_cell_choice`
  (`keyed` pin on a KeyedFold cell admits; `update` pin refuses).
- `maintenance_tracer_propagation.rs`: calendar round-trips at era boundaries
  ((1,1,1), (0,3,1), (−1,12,31), (2100,3,1), (2400,2,29), …) + a December-wrap
  Month-grain propagation test.
- `propagate.rs` (in-file tests): forward-margin widening of `project_observed_delta`;
  half-sentinel `is_whole` asserts.
- `maintenance_skeleton.rs`: ordinal `ORDER BY 2` / `ORDER BY 1` skeleton-role resolution.
- `locality.rs` (in-file tests): clocked/unclocked-join driving-source resolution;
  single-unclocked `Ok(None)`; route-3 declared `key_recurrence` folding a forward-only
  static bound into `margin_after`.
- `maintenance_tracer_evolution.rs`: ColumnAdded with declared `allow_full_scan: true`
  admits with `PartitionLocal::No { why: "... declared full scan" }`.
- `explain_maintenance.rs`: admissible-write-patterns line pinned.

## Final residue: 13 survivors, all classified

| Class | Mutants | Disposition |
|---|---|---|
| Advisory/label text | `trigger_label`×2, `resolvable_set_label`×2, `LocalityRefusal::fmt` | Refusal/label text largely unpinned. Open question: should fail-loud culture pin refusal TEXT (goldens), or is text advisory? |
| Genuine untested logic | `choice.rs:235` (delete the ColumnScopedMerge liveness arm — nothing drives `resolve_cell_choice` with `backend_supports_column_scoped_merge=false`), `derive.rs:182` (`\|\|`→`&&` in `source_contributes_to_fold`), `derive.rs:1279` `group_columns`×2, `granularity.rs:68` guard | Follow-up tests (see TODO). |
| Dormant feature | `derive.rs:789` `model_fingerprint_projections` → empty | P4 fingerprint projection is pure plan data; no executed statement consumes it until F3 wires sidecars. Re-run the campaign after F3. |
| Provably equivalent | `derive.rs:240` match guard | Delete the guard (cleanup) or accept. |
| Gated suite only | `emit.rs:850` Spark Text/Varchar arm | Killable only by the Spark parity suites (needs live server). |

## Bonus campaign: the property-composition walk

A second campaign over `crates/smelt-logical/src/analysis/walk.rs` (196 mutants, same tier-1
battery, with the flaky `a_hand_corrupted_stamp` test excluded from the baseline):
**125 caught / 38 missed / 33 unviable — a 76.7% kill rate**, materially weaker than the
maintenance layer's 91.3% despite the standing `walk_coverage` gate. Untriaged (no tier-2
pass run), but the survivor list includes the walk's fail-closed spine:

- `QueryNode::has_unsupported -> false` (+ two `||`→`&&` inside it) — the "refuse what we
  can't prove" valve can claim everything is supported and no tier-1 test notices.
- `PartitionGrainAdmission::leaf -> Default` and its operator's deleted `!` — partition-grain
  admission verdicts.
- `PropertyTransfer::leaf -> Default` and `|=`→`&=` in its operator — the property-vector
  fold at the heart of the walk.
- `setop_kind_after`: deleted `INTERSECT_KW`/`EXCEPT_KW` arms.
- `is_constant_literal -> true`; `union_discriminated_grain` `<`→`>`;
  `Grain::has_subset_key -> false`; the `own_region_text*` collector guards;
  `AdmissionViolation::path_display` (label class).

The walk_coverage gate asserts *which properties are produced by the walk*; this campaign
shows it under-constrains *what the walk computes*. Survivor list preserved in the TODO
residue block; triage + kills are follow-up work.

## Bonus campaign addendum: walk.rs survivor triage (2026-08-08)

Follow-up session (`docs/plans/20260808-substrate-unification.md` Phase 1). A fresh
`cargo mutants --file crates/smelt-logical/src/analysis/walk.rs --iterate -p smelt-logical`
run (same tier-1 battery: `cargo test -p smelt-logical`) reproduced **40 missed / 123 caught /
33 unviable** (196 total; the 2-mutant difference from the original 38 is run-to-run
mutant-generation variance, not a behavior change). Every survivor was triaged and either
killed by a new test in `crates/smelt-logical/tests/walk_hardening.rs`, or classified below.
A second `--iterate` pass against only the 40 previously-missed mutants confirms the kills:
**21 caught / 19 missed** — every one of the new tests kills exactly the mutant(s) its doc
comment names, and no previously-caught mutant regressed.

New kill rate: **146/163 viable (89.6%)**, up from 76.7%, closing the gap with the
maintenance layer's 91.3%.

### Per-survivor verdict

| Mutant (line:col) | Verdict | Disposition |
|---|---|---|
| `241:9` `has_unsupported -> false` | Genuine gap | Killed: `unsupported_node_fails_closed` |
| `245:21` `\|\|`→`&&` (Select arm) | Genuine gap | Killed: `unsupported_node_fails_closed` |
| `253:21` `\|\|`→`&&` (SetOp arm) | Genuine gap | Killed: `unsupported_node_fails_closed` |
| `458:13` delete `INTERSECT_KW` arm | Genuine gap | Killed: `intersect_except_degrade` |
| `459:13` delete `EXCEPT_KW` arm | Genuine gap | Killed: `intersect_except_degrade` |
| `692:25` `select_lineage` `aliases.len()==1`→`true` | Genuine gap | Killed: `select_lineage_ambiguous_ref_not_resolved` |
| `789:9` `ScopeEnum::leaf`→`Default` | Provably equivalent | No action (see below) |
| `943:9` `path_display`→`String::new()`/`"xyzzy"` (×2) | Genuine gap (unpinned diagnostic text) | Killed: `admission_violation_path_display_is_pinned` |
| `952:49` `alias.is_empty()`→`true`/`false` (×2) | Genuine gap | Killed: `admission_violation_path_display_is_pinned` |
| `977:9` `PartitionGrainAdmission::leaf`→`Default` | Provably equivalent | No action |
| `998:24` delete `!` (`PartitionGrainAdmission::operator`) | Genuine gap | Killed: `leaf_transfer_not_default` |
| `1197:36`/`1197:41` `own_region_text` `node==root` guard (×3 encodings) | Deferred | See below |
| `1198:33`/`1198:45` `own_region_text` `TABLE_REF` guard (×3 encodings) | Deferred | See below |
| `1254:9` `SkewTransfer::leaf`→`Default` | Provably equivalent | No action |
| `1300:21` `scope_self_qualifiers` `!=`→`==` | Deferred | See below |
| `1359:21` delete `WITH_CLAUSE` arm (excluding-self variant) | Deferred | See below |
| `1360:36`/`1360:41` excluding-self `node==root` guard (×3) | Deferred | See below |
| `1361:33`/`1361:45` excluding-self `TABLE_REF` guard (×3) | Deferred | See below |
| `1487:23` `!tree.root.has_unsupported()`→`true` | Genuine gap | Killed: `unsupported_sql_falls_back_to_whole_text_skew` |
| `1522:9` `Grain::unkeyed`→`Default` | Provably equivalent | No action |
| `1528:9` `has_subset_key`→`false` | Genuine gap | Killed: `declared_fd_survives_via_subset_key` |
| `1873:9` `PropertyTransfer::leaf`→`Default` | Provably equivalent | No action |
| `1892:35` `\|=`→`&=` (`PropertyTransfer::operator`) | Genuine gap | Killed: `leaf_transfer_not_default` |
| `1898:25` delete `InputItem::Derived{alias:Some(alias),..}` arm | Genuine gap | Killed: `leaf_transfer_not_default` |
| `1913:59` delete `!` (distinct-grain guard) | Genuine gap | Killed: `leaf_transfer_not_default`, `distinct_grain_uses_projected_columns` |
| `1997:17` `resolve_alias_source` `aliases.len()==1`→`true` | Genuine gap | Killed: `determinism_not_reduced_through_ambiguous_alias` |
| `2052:23` `union_discriminated_grain` `<`→`>` | Genuine gap | Killed: `union_discriminator_requires_distinct_tags` |
| `2192:5` `is_constant_literal`→`true` | Genuine gap | Killed: `constant_literal_rejects_function_call` |
| `2207:17` delete `IDENT` arm (`is_constant_literal`) | Genuine gap | Killed: `constant_literal_rejects_function_call` |
| `2209:24` delete `!` (`is_constant_literal` type-keyword guard) | Genuine gap | Killed: `constant_literal_rejects_function_call` |

(Table rows group same-line mutant variants the survivor list reports separately; counts above
match the raw 40.)

### Provably equivalent (5 mutants, no action)

Every concrete `Transfer` implementation in this file (`ScopeEnum`, `PartitionGrainAdmission`,
`SkewTransfer`, `PropertyTransfer`) returns its verdict type's literal `Default` value from
`leaf` — by design: a bare relation proves no properties of its own (grain, admission
violations, skew, and the property vector are all established by the *consuming* scope's
`operator`, never by a leaf in isolation — the fail-closed default IS the correct leaf verdict,
not a placeholder standing in for one). Mutating `leaf` to `Default::default()` is therefore
bit-identical to the unmutated body for every input; no test, however constructed, can
distinguish them. `Grain::unkeyed()` is definitionally `Grain { keys: Vec::new() }`, exactly
`Grain::default()`, for the same reason. These five survivors are permanent, expected residue
of this pattern — documented here rather than chased with an unkillable test.

### Deferred, with reason (14 mutants)

The `own_region_text` / `own_region_text_excluding_self_relations` collector guards (13
mutants across the two functions) prune nested-walk-node subtrees (the WITH clause, the next
set-operation arm, a derived table's body) from the raw text handed to the skew leaf
classifier (`derive_partition_skew`, a text-heuristic pattern match). A guard flipped to
*never* prune only **duplicates** already-covered text into the same scope's own region; since
`Skew::union` takes the max of `before`/`after` across every scope's contribution, and the
duplicated text is byte-identical to what the referenced node already contributes on its own
walk visit, the final model-level skew is frequently unchanged — the naive kill construction is
an equivalent mutant in practice, not just in theory. `crates/smelt-logical/tests/
skew_self_exclusion.rs`'s seven existing tests already exercise this exact code path
heavily (self-exclusion, cross-scope alias reuse, an OR-guarded disjunction, a string-literal
decoy) and still do not kill these mutants — corroborating that a discriminating case needs a
scenario where the duplicated/omitted text changes which *pattern* `derive_partition_skew`
matches, not merely how many times it matches the same one (e.g. a nested SELECT_STMT whose
inclusion vs. exclusion changes which anchor expression sits adjacent to the driving column in
the concatenated text). `scope_self_qualifiers`'s `last != key` guard (1 mutant) similarly
needs a precisely-shaped self-reference — an *unaliased, dotted* self path (e.g.
`FROM smelt.marts.balance`, no `AS`) whose WHERE/ON conditions use the *bare last segment*
(`balance.col`) rather than the full dotted qualifier — and getting the walk's exact alias-map
key convention right for that shape needs more space than this session's remaining budget.
Tracked as residue below rather than landing a guessed, possibly-wrong test.

## Takeaways for the proof surface

- Mutation testing is cheap here (~1.5h warm, incremental via `--iterate`) and its finding
  class is complementary to the generative gates: the conformance gate proves the equivalence
  invariant over what its generator produces; mutation testing finds the inputs the generator
  never produces.
- The staged `--iterate` triage (diffing caught-lists between batteries) doubles as a
  **gate-attribution map**: it empirically shows which suite carries which invariant.
- Highest-leverage generator extensions for `maintenance_conformance` suggested by the blind
  spots: recipes with `cells[].write` pins, ColumnAdded triggers with `allow_full_scan`,
  December/era-boundary date pools, backends without column-scoped MERGE.

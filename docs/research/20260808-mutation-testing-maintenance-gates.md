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

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
| 1 | `PartitionGrainForbidsMetrics`: classifier in the partition-grain admission walk, `DiagnosticCode`, `file_diagnostics()` + LSP parity, broken-example fixture, and its Known Divergence bullet | done |
| 2 | Sub-`g_part` refusal names the coarsened run window; test asserts the printed pair and that it is accepted | done |
| 3 | Route 2 derived key-derived-expression sub-route, declared FD as fallback; conformance recipe and end-to-end fixture | done |
| 4 | `KeyedRecurrenceDeclarationMismatch` (derived authoritative, declared is a check); order-independent key-set comparison with permutation test | done |
| 5 | Retire per-column `data_latency` (hard error + fix-it, LSP parity); remove lateness from `compute_effective_window` and the runtime widening path; grep gate; explain prints it as orchestration-only | done |
| 6 | Append-only posture probe: increase = late arrival, decrease/fingerprint = violation; conformance late-append step kind | done |
| 7 | Delete the remaining divergence bullets (those phases 1-6 did not already close); validate the four specs; all gates green | planned |

## Decision log

- 2026-09-05 — Outcome moved `queued` → `active`. Phase 1 now also deletes its own
  `PartitionGrainForbidsMetrics`-is-unimplemented Known Divergence bullet (and updates the
  `partition_residue_probes` bullet ratchet) instead of leaving it to phase 7: shipping an
  implemented refusal alongside a spec bullet calling it unimplemented would be a false spec at
  the phase-1 commit. Phase 7 narrows to the bullets earlier phases did not close.
- 2026-09-05 — Phase 1 done: `PartitionGrainForbidsMetrics` classifier, `DiagnosticCode`, LSP
  parity, `examples/broken/models/partition_grain_forbids_metrics.sql`, catalogue row, and the
  divergence bullet deleted. `partition_residue_probes.rs` ratchet updated 4 → 3.

- 2026-09-05 — Phase 2 planned with no table reshape. Two refusals are distinguished
  explicitly: the window-level one carries the actionable coarsened
  `[--event-time-start, --event-time-end)` pair (criterion 2's "re-running with it succeeds"),
  while the config-level `g_run < g_part` one names the required `timeseries.granularity` plus
  the covering window as context — a window suggestion there would be untrue. Phase 2 also
  folds in the phase-1 summary's residue: the stale "the six this outcome does not own" doc
  comment in `partition_residue_probes.rs`, and the ratchet drop for the bullet it closes.

- 2026-09-05 — Phase 2 done: `coarsen_window_to`/`suggested_window_flags`/`is_grid_aligned`
  helpers land in `windowing.rs`; every `validate_run_window_alignment` misalignment arm and
  both `validate_run_window_against_partition_grid` refusals (window-level and config-level)
  now name the coarsened/covering pair. The window-vs-`g_part`-grid residue (monthly `g_run`
  over weekly `g_part`) is now caught, not just `g_run < g_part`. Spec bullet deleted;
  `partition_residue_probes.rs` ratchet 3 → 2.

- 2026-09-05 — Phase 3 planned with no table reshape (phase 2 surfaced no residue). Two
  planning calls recorded: (a) the derived sub-route is consulted *before* route 2's
  extremal-fold refusal, because a `MAX`/`MIN` over a `unique_key` column is the key itself —
  the same argument commit `293eb5ce` used for `classify_once_write`'s candidate loop — while an
  extremal fold over a non-key column stays refused and remains route 3's shape; (b) the derived
  proof is a CST-based leaf classifier over the model's own select list (no raw-text scan), so
  the property-composition-walk gate stays green without an exception entry.

- 2026-09-05 — Phase 3 done: `smelt_logical::analysis::key_derived::key_derived_partition_
  verdict` lands, wired into `establish_locality` before the extremal check.
  `ComposedRoute::KeyDerived` added to the maintenance-testkit recipe pool (admitted by the
  derived sub-route with no declared FD) and driven to equivalence with the full-refresh oracle
  via `run_windowed_keyed_maintenance` (same channel `KeyDetermined` uses — `classify_cumulative`
  still refuses its scalar-wrapper projection independently of locality admission, the same
  pre-existing gap `KeyDetermined` already hits). New end-to-end fixture
  `crates/smelt-runtime/tests/locality_route2_derived.rs`. Spec bullet and stale-fixture clause
  both deleted from `incremental_shapes.md`. `verify-phase.sh` ALL GREEN (fmt, clippy both
  feature sets, full workspace `cargo test`, `example_diagnostics`), plus every plan-listed
  targeted test green.

- 2026-09-05 — Phase 4 planned with no table reshape (phase 3 surfaced no residue). Three
  planning calls recorded: (a) the mismatch check fires only where the declared
  `key_recurrence.key` set-equals the model's own `unique_key` — a bound declared over a
  different key asserts nothing about this key, so it stays the existing route-3 key-mismatch
  refusal, not a value mismatch; (b) an *agreeing* declaration admits the **derived**
  `LocalitySlice::Window { recurrence_bounded: true }`, not the checked
  `RecurrenceBounded` — rule 16 makes the declaration a check, so the proof-backed slice wins
  and no runtime probe is added where a static proof exists; (c) the refusal is carried as a
  new `LocalityRefusal`/`Refusal` variant with its own `DiagnosticCode`, rather than reusing
  `KeyedForbidsTimeseries`'s message channel, so `file_diagnostics()` and the LSP name the
  spec's own code. The order-independence clause resolved to two real sites: locality's
  declared-key match (already set-based, made explicit) and `propagate.rs`'s
  `push_keyed_dirt`, whose duplicate check compares `keys` as an ordered `Vec` and is the one
  order-sensitive comparison found.

- 2026-09-05 — Phase 4 done: `LocalityRefusal::RecurrenceDeclarationMismatch` lands in
  `locality.rs`, consulted in route 3's statically-derived branch before admission — a matching
  declared `key_recurrence` that agrees is a no-op (admits the derived, unchecked `Window`
  slice), a disagreeing one refuses, and a non-matching key falls through unaffected. Routed via
  a new `Refusal::KeyedRecurrenceDeclarationMismatch` / `recurrence_mismatch_plan` (distinct from
  `locality_refused_plan`, since this is a value check, not a "no route applies" refusal) through
  a new `DiagnosticCode::KeyedRecurrenceDeclarationMismatch` (Error) with LSP slug
  `keyed-recurrence-declaration-mismatch`. Added a shared `key_sets_match` (case-insensitive,
  order-independent) helper used by both route-3 sub-routes, and fixed `propagate.rs`'s
  `push_keyed_dirt` (previously ordered-`Vec` equality) to compare `keys` as a set — the one
  real order-sensitive site the audit found. Spec bullets deleted from `diagnostics.md` and
  `incremental_shapes.md`. `verify-phase.sh` ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`); `partition_residue_probes` ratchet unchanged
  (2, as expected — this phase touches a key-grain bullet, not a partition-grain one);
  `maintenance_conformance --features duckdb composed` green.

- 2026-09-05 — Phase 5 planned with no table reshape (phase 4 surfaced no residue). Three
  planning calls recorded: (a) the hard error is raised on the existing
  `MetadataError::YamlParseError(custom(...))` channel the retired `batched:` sub-block already
  uses, not a new `MetadataError` variant — the retirement needs a fix-it, not its own
  diagnostic code, and this keeps the exhaustiveness gate untouched; (b) the `DataLatency`
  *grammar* stays (`contract.frozen_horizon` parses intervals through it) — only the per-column
  key dies, pinned by its own test; (c) the undocumented per-column `data_latency` on
  **source** columns (`SourceColumnDef`, parsed and read by nothing) is retired in the same
  phase as a `MalformedSource` with the same fix-it, rather than left as a silently-ignored
  dead key — a fail-loud violation the models-side retirement would otherwise make more
  conspicuous. This adds one sentence to `sources.md`'s `MalformedSource` row. Phase 5 also
  deletes its own two Known Divergence bullets (`models.md`, `model_properties.md`) per the
  phase-1 precedent, leaving phase 7 the bullets earlier phases did not close.

- 2026-09-05 — Phase 5 done: `ColumnMetadata::data_latency` and the legacy
  `SourceColumnDef::data_latency` both deleted, refused at parse time via the existing
  `YamlParseError` channel with a shared fix-it naming `mutation_profile.lateness`.
  `compute_effective_window` (`smelt-logical`) and the whole `smelt-runtime::windowing`
  chain (`compute_incremental_windows`/`_impl`/calendar/integer/`_ordered`) drop
  `data_latency_days` entirely — the effective window is the AST-derived reach alone. `smelt
  explain` (text and `--json`) prints a source's declared `mutation_profile.lateness` as
  `orchestration-only fact: lateness = <interval> (never a plan input)` via a new
  `InboundEdgeContract::lateness` field. New grep-gate test
  `smelt-logical/tests/lateness_orchestration_only.rs` pins both the field-read absence and
  the signature. Two golden/tutorial fixtures drifted because `examples/timeseries`'s
  `raw.events` already declared `mutation_profile.lateness` — both regenerated/updated.
  `verify-phase.sh` ALL GREEN; `hardening_budget` baseline unchanged.

- 2026-09-05 — Phase 6 planned with no table reshape (phase 5 surfaced no residue beyond a
  fixture-grep caution, folded into the plan's verification list). Four planning calls
  recorded: (a) `emit_append_only_posture_probe` stays the single owner of the *violation*
  verdict with a narrowed predicate, while the *late-append* verdict is a new pure classifier
  over the baseline snapshot the runtime already executes on a held probe — one SQL round
  trip, no second rendering of the same comparison, mirroring
  `contract/frozen_horizon.rs::late_arrivals`; (b) only a closed partition
  (`check_fingerprint: true`, strictly below the recorded max) can produce a late append —
  the open frontier partition legitimately grows every run and reporting it would make the
  observation pure noise; (c) a delete+insert netting to a count increase reads as a late
  append, because one aggregate fingerprint per partition cannot prove subset-ness — the spec
  says so rather than implying a proof smelt cannot make; (d) the conformance "late-append
  step kind" resolves to turning the harness's probe cadence back ON over the existing
  `ConformanceStep::AppendLateRow` schedules — `render_smelt_yml_for` currently sets
  `probes: {cadence: off}` with a comment citing this exact limitation, so closing the
  limitation closes the workaround with it.

- 2026-09-06 — Phase 6 done: `late_appends` pure classifier lands in
  `smelt-logical`'s maintenance layer; `emit_append_only_posture_probe`'s fingerprint leg now
  also requires an unchanged row count, so a closed partition's pure count increase never trips
  it. `dispatch_and_record_append_only_postures` classifies every held verification against the
  carried baseline, `tracing::warn!`s late appends, and records the count on
  `ProbeRecord.observed` — both `execute.rs` dispatch sites inherit this from the one shared
  function. `render_smelt_yml_for` flipped `probes: {cadence: off}` → `cadence: per_run`
  globally (not pool-scoped): the full 80-test `maintenance_conformance` suite passed unchanged
  under the flip, so the plan's scoped-fallback escape hatch was not needed. New generative case
  `probes::late_append_schedule_holds_with_probes_on` drives an `AppendLateRow`-bearing schedule
  through `execute_project` with probes on. Spec bullet deleted from `model_properties.md`;
  `sources.md`/`run_state.md` narrowed to match. `partition_residue_probes.rs` ratchet
  unchanged (still 2 — this bullet isn't one of that file's partition-grain bullets).
  `verify-phase.sh` ALL GREEN on a clean re-run (one transient `smelt-lsp::example_workspaces`
  timeout under concurrent load, confirmed a flake by standalone re-run).

- 2026-09-06 — Phase 7 planned with no table reshape (phase 6 surfaced no residue beyond a
  known LSP flake). A pre-scan of the four anchors found **no** surviving Known Divergence
  bullet this outcome created — phases 1–6 each deleted their own inline, as the phase-1
  precedent set — so phase 7's real content is the residue those deletions left *elsewhere*:
  `models.md` still lists the retired per-column `data_latency` as a live declared fact in two
  places outside the frontmatter table, and `sources.md`'s `mutation_profile` divergence bullet
  still lists `lateness` among the sub-facts awaiting per-cell admission, which contradicts the
  now-decided orchestration-only trust rule in the same file. Both are corrected here and
  fenced by a new doc-sweep case on the existing `lateness_orchestration_only` grep gate.
  Drift the validate passes surface that phases 1–6 did not cause is classified, not fixed:
  recorded as a Known Divergence bullet if unrecorded, left alone if already recorded.

## Blocked

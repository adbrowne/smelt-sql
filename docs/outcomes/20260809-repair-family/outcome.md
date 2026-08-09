# Outcome: The repair family — per-group recompute and diff-then-patch

**Created:** 2026-08-09
**Status:** done
**Source:** `docs/research/20260809-incremental-rethink.md` §3 T-A/T-B, §6 step 2
**Spec anchors:** `docs/specs/incremental_models.md` (technique families, write-pattern registry), `docs/research/20260724-ivm-pattern-gap-catalogue.md` §A1/§C1

## The outcome

The maintenance plan gains repair techniques: when a non-invertible aggregate
receives a retraction (or a probe detects drift), smelt recomputes only the
affected groups from their bounded input slice — instead of refusing to a full
refresh. A diff-then-patch write pattern (compute the slice, diff against
stored state, write only the difference) exists as a registry entry serving
reconciliation runs and idempotent re-runs.

## Success criteria (checkable)

1. A keyed model with a non-invertible combiner over a mutable/retraction
   source derives a per-group recompute cell (affected keys → bounded
   recompute) where today it refuses (`KeyedReprocessedWindow` / full refresh).
2. Admission is proof-gated: derivable group key, bounded per-group read
   footprint, delta discovery naming the affected keys; anything unprovable
   still refuses by name.
3. `diff_patch` is a registered write pattern with a pure emitter; executed
   statements pass `cargo test -p smelt-runtime --test statement_parity`.
4. `maintenance_conformance` recipes cover retraction → per-group repair and
   reconcile-via-diff-patch, asserted against the full-refresh oracle.
5. `smelt explain` renders repair cells with their key-slice and read bound.
6. All standing gates green; walk rule holds (no new whole-text scans).

## Out of scope

- Derivation-count state for `DISTINCT`/`EXISTS` under retraction (needs the
  rung-2 state machinery pattern; schedule as its own outcome if it grows).
- Shadow-build-and-swap (T-D) and backfill choreography (T-E).

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec: repair techniques — per-group recompute + diff-then-patch semantics, admission obligations, refusal narrowing | done |
| 2 | Delta discovery names affected keys (retraction/mutation → key set, fail-closed) | done |
| 3 | Per-group recompute technique: derivation, admission, emitter | done |
| 4 | `diff_patch` write pattern: registry entry, admission, pure emitter, structural no-authoring leg | done |
| 5 | Refusal narrowing in plan derivation: retraction paths route to a repair cell, unprovable obligations refuse by name; `diff_patch` delete-leg completeness premise | done |
| 6 | Runtime lowering: per-group recompute cells execute; executed-vs-emitted `statement_parity` leg for the repair family | done |
| 7 | Runtime routing for the `diff_patch` write pin (`ChosenTechnique::DiffPatch` → emitter) + its executed-vs-emitted `statement_parity` leg | done |
| 8 | Conformance recipes for repair + diff-patch families | done |
| 9 | Delete-aware affected-key discovery: a full-group deletion in a `mutable_snapshot` source is repaired (obligation 7 soundness) | done |
| 10 | Repair over a decomposed combiner: the candidate/insert supplies the fold's hidden state columns | done |
| 11 | Surface: `smelt explain` rendering, docs-site update | done |

## Decision log

<!-- Dated one-liners appended by plan/implement steps. -->

- 2026-08-09 (plan 1): outcome activated; phase table kept as scaffolded (rung-2 outcome closed clean, nothing to reshape). Phase 1 scoped spec-only: repair family as a §Semantics section, affected-key discovery owned by `model_properties.md`, `diff_patch` as a write-pattern registry entry, two new `Maintenance*` refusal codes.
- 2026-08-09 (implement 1): landed §"The repair family" in `incremental_models.md` — corner placement is column-scoped re-derivation (full read, targeted write), not a new corner; two of its three admission obligations are the *existing* obligations 4/6 cited by number, only affected-key discovery is new (obligation 7); slice completeness reuses key temporal locality rather than a new proof. `diff_patch` landed as a subsection under §"The write-pattern set is open" with its delete leg gated on the same slice-completeness premise. `model_properties.md` gained §"Affected-key discovery" (`derive_affected_keys`, fail-closed, sound-over-approximation-only). Refusal narrowing landed in §"Reprocessing" and both `KeyedReprocessedWindow`/`KeyedRetractableContribution` diagnostics prose (both spec files).

- 2026-08-09 (plan 2): no reshape — phase 1 fixed `derive_affected_keys`'s entry point and verdict shape, so the remaining rows stand as scaffolded. Phase 2 scoped proof-only (pure `smelt-logical` classifier + spec §Surface status flip); plan-cell derivation, emission and refusal wiring stay in phases 3–5. Provenance resolution reuses `analysis::fingerprint`'s walk-backed per-column leaf classifier (parameterised by an output-column filter) rather than a second lineage implementation, keeping the property-composition-walk rule intact.

- 2026-08-09 (implement 2): landed `derive_affected_keys` in
  `crates/smelt-logical/src/analysis/affected_keys.rs` — grain precedence matches
  `row_identity_with_context` exactly (declared `unique_key` else fan-out-gated proven grain,
  no `WholeRow`-style fallback). Reused `analysis::fingerprint`'s leaf classifier via
  `pub(crate)` visibility rather than a copy; introduced zero new `.contains("` sites. Flagged
  for phase 3: a grain column with zero dependency on the delta's own source is treated as
  "no requirement" — untested corner, no pinning spec sentence.

- 2026-08-09 (plan 3): one reshape — phase 5 now also owns the `smelt-runtime` lowering that
  executes a repair cell, since a derived-but-unrouted cell has nothing to lower; phase 3 gives
  the new `Technique` variant an explicit fail-loud lowering arm instead. Phase 3 scoped to
  pure `smelt-logical` machinery (variant, two refusal variants, `repair.rs` admission +
  cell derivation, emitter) plus one spec sentence resolving phase 2's flagged corner: when
  *every* grain column is independent of the delta's source the verdict is `NotDiscoverable`,
  not an unconstrained key set — the repair family never widens to a whole-table repair.

- 2026-08-09 (implement 3): landed `Technique::PerGroupRecompute`, `Refusal::
  RepairKeysNotDiscoverable`/`RepairSliceUnbounded`, `maintenance::repair::
  {admit_per_group_recompute, derive_repair_cell}` and `emit::emit_per_group_recompute` — all
  standalone, unit-proven, not yet called from `derive_maintenance_plan` (phase 5's scope).
  Fixed the "every grain column independent of source" corner directly in `affected_keys.rs`
  (the proof's sole owner), per phase 3's own spec delta. Widened `derive::LocalityInputs`/
  `SourceLink`/`project_source_link` from private to `pub` so `repair.rs` reuses the exact same
  scan-clamp derivation `derive_mutation` uses, rather than a second copy.

- 2026-08-09 (plan 4): one reshape — `statement_parity`'s *executed*-vs-emitted leg for
  `diff_patch` (and the repair cell) moves into phase 5's row, since a pattern nothing routes to
  executes no statements; phase 4 keeps the registry entry, admission, pure emitter and the
  structural no-authoring leg. Criterion 3 is therefore split across phases 4 and 5, not deferred.
  Phase 4 also decides that `diff_patch` is a write *mechanism*, not a new `Technique`: it enters
  the closed enum's namespace via a `WriteSelection::DiffPatch` arm plus a
  `ChosenTechnique::DiffPatch` variant carrying the underlying recompute technique and the
  delete-leg admission, so a pin can never silently degrade to a blanket delete+insert.

- 2026-08-09 (implement 4): landed `diff_patch` as `WriteSelection::DiffPatch` +
  `ChosenTechnique::DiffPatch { recompute: Technique, delete_leg: diff_patch::DeleteLeg }` (no new
  `Technique` variant), `maintenance::diff_patch::admit_diff_patch` (identity via
  `RowIdentity::Key`, comparability reused verbatim from `choice::resolve_write_suppression`,
  slice completeness as a caller-supplied `Result<(), String>`), and `emit::emit_diff_patch` (one
  function, conditional delete-leg statement, not two sibling emitters — the degradation is a
  per-call runtime fact, not a distinct caller population). An incomparable/unproven compared
  column refuses the whole pattern rather than degrading to an unconditional update leg (that
  would just be delete+insert with extra steps). `resolve_cell_choice`'s new `DiffPatch` arm
  always resolves `DeleteLeg::Omitted` today — the real completeness proof is phase 5's to thread
  through.

- 2026-08-09 (plan 5): one reshape — the old phase 5 split in two, since plan-derivation wiring and
  runtime lowering are independently verifiable and the combined row was too wide for one step:
  new phase 5 is derive-layer only (refusal narrowing + the `diff_patch` completeness premise phase
  4 flagged), new phase 6 owns runtime lowering and both executed-vs-emitted `statement_parity`
  legs; conformance and surface shift to 7 and 8. Nothing left the outcome. Phase 5 decides: the
  narrowing hooks `derive_new_data`'s key-grain faithful-fold *source-posture* leg (the retraction
  case criterion 1 names), repair only ever converts a refusal into a cell — never replaces an
  admitted `ColumnScopedMerge`/fold cell — a failed obligation pushes its `Refusal::Repair*`
  *alongside* the existing `NoAdmissibleTechnique`, and the `DeltaShape` is derived from the model's
  own SQL (a `MutableSnapshot` delta is a whole-row snapshot diff) rather than plumbed as a new
  world fact. The combiner-algebra leg (holistic combiner over an append-only source) is not
  narrowed — it is not a success criterion.

- 2026-08-09 (implement 5): landed the narrowing — `derive_new_data`'s faithful-fold
  source-posture failure branch now attempts `repair::admit_per_group_recompute` before refusing,
  pushing a `PerGroupRecompute` cell on success or the additive `Refusal::RepairKeysNotDiscoverable`
  / `RepairSliceUnbounded` alongside the pre-existing `NoAdmissibleTechnique` on failure.
  `derive_repair_cell` now takes the real `Trigger`; added `repair::delta_shape_for_source`
  (reuses `fingerprint_projection`'s leaf classifier, fails closed to empty columns on
  `Projection::FullRow`). `resolve_cell_choice`'s `DiffPatch` arm now grants `DeleteLeg::Complete`
  when the underlying recompute is `PerGroupRecompute` (its own key-temporal-locality premise),
  `Omitted` otherwise. Five pre-existing tests needed their refusal-count assertions widened
  (additive refusal, not a replacement) — no golden/conformance fallout otherwise.

- 2026-08-09 (plan 6): one reshape — the old phase 6 split in two, the same way phase 5 did and for
  the same reason: it carried two independent families' runtime lowering (a repair cell that
  derives itself, and a `diff_patch` write pin that only ever arrives via `resolve_cell_choice`'s
  `ChosenTechnique::DiffPatch`), each with its own `statement_parity` leg and its own execute.rs
  routing site. New phase 6 is the repair family only; new phase 7 owns `diff_patch` routing;
  conformance and surface shift to 8 and 9. Nothing left the outcome — criterion 3 stays split
  across phases 4/7. Phase 6 decides: `candidate_select` is the model's own FULL recompiled SQL
  semi-joined to the affected keys (the shape `execute_staged_membership_recompute` already uses
  for a group-complete recompute), and the cell's `ScanClamp` is pushed into the *affected-keys*
  read (a predicate on the source, where the clamp is actually defined) rather than onto the
  output wrapper, where the partition column need not appear.

- 2026-08-09 (implement 6): landed runtime lowering — `resolve_live_per_group_recompute_cell` +
  `execute_per_group_recompute` (`maintenance_driver.rs`), routed in the keyed run loop's
  window-forward branch *instead of* `execute_cumulative_aggregate` (a repair cell is an
  alternative to `KeyedFold` for the same `NewData` trigger, not a technique dispatched
  alongside it, unlike column-scoped-merge/membership-recompute). `repair_affected_keys_select`
  reuses `widened_scan_predicate` (previously test-only) with typed `TIMESTAMP` region literals
  — the one place a region endpoint is an arithmetic operand. `diagnostics.rs`'s `PerGroupRecompute`
  preview arm now builds real statements; `build_technique_statements` threads `cell: &PlanCell`
  instead of just `trigger`. `docs/specs/incremental_models.md` divergence entry narrowed to
  `diff_patch` routing only (phase 7). Flagged for phase 8: no shipped example workspace reaches
  the repair family yet — the new DuckDB tests stage their own fixture; a real conformance recipe
  needs one too.

- 2026-08-09 (plan 7): no reshape — phase 6 closed clean, phases 8/9 stand as written. Phase 7
  decides: routing extends `resolve_live_per_group_recompute_cell` with a write *mode* rather
  than adding a near-verbatim sibling resolver (a `diff_patch` write over a repair cell reads
  the identical affected-key set, candidate select and key — only the write leg differs); only
  `ChosenTechnique::DiffPatch { recompute: PerGroupRecompute }` is routable (the sole recompute
  granted `DeleteLeg::Complete`), and a `diff_patch` pin over the region `DeleteInsert` default
  fails loud by name rather than falling through to the default write. `emit_diff_patch`'s
  `partition_col`/`region` pair collapses to one caller-composed `slice_predicate` — a keyed
  aggregate output has no partition column, so a region predicate cannot express the only slice
  the routable recompute produces; no shipped statement changes, since nothing routed to this
  emitter before.

- 2026-08-10 (implement 7): landed the routing — `resolve_repair_write` (new, pure) is the
  decision table from `ChosenTechnique` to a `RepairWrite`, split out of
  `resolve_live_per_group_recompute_cell`'s loop so it is independently unit-testable;
  `emit_diff_patch`'s `(partition_col, Region)` pair collapsed to one caller-composed
  `slice_predicate: &str`. Discovered: `resolve_cell_choice`'s `DiffPatch { recompute: <not
  PerGroupRecompute> }` bail arm is real fail-loud code but unreachable via any production call
  path today (this resolver only ever calls `resolve_cell_choice` with a cell whose own technique
  is already `PerGroupRecompute`) — kept as defensive code, spec's Known Divergences entry
  reworded to say the region-`DeleteInsert` case is *unenforced*, not refused, until a future
  phase threads a write-pin check into a resolver that can actually reach it.

- 2026-08-10 (plan 8): no reshape — phase 7 closed clean, phase 9 stands as written. Phase 8
  decides: the repair/diff-patch cases enter the standing gate as typed testkit recipe *data*
  (a new `RepairRecipe` + its renderer, clocked `mutable_snapshot` source with declared
  `unique_key`, mirroring `repair_lowering.rs`'s fixture shape), not hand-written SQL in the
  test — the rule `pinned.rs` already states; they live in a new `repair.rs` module reusing
  `gate.rs`'s `pub` helpers rather than growing that 5.9k-line file. Each case asserts both
  equivalence-vs-oracle *and* that the repair/diff-patch statements were actually executed, so a
  silent full-refresh fallback cannot pass. Phase 7's unenforced-write-pin divergence stays where
  it is (not a success criterion); a real oracle divergence, if any surfaces, becomes a registry
  `KnownBug` entry rather than a weakened assertion.

- 2026-08-10 (implement 8): landed the conformance recipes — `RepairRecipe`/
  `RepairWriteMode` in `smelt-maintenance-testkit`, a promoted `RecordingBackend`
  (needed because repair/diff-patch dispatch doesn't route through
  `RunReporter::maintenance_statements` yet), and 5 new tests in
  `maintenance_conformance/repair.rs`, all green. Discovered two genuine
  production gaps (not test bugs), both registered as `KnownBug` + spec Known
  Divergences: (1) `repair_affected_keys_select` under-approximates when a
  key's entire window contribution is deleted from a `mutable_snapshot`
  source between runs (violates obligation 7 — no follow-up phase scheduled
  yet, flagged for the next planner); (2) `repair_candidate_select` ignores a
  decomposed combiner's hidden state columns, so only `Idempotent`-shaped
  combiners get full equivalence coverage under retraction today.

- 2026-08-10 (plan 9): reshape — phase 8's two discovered production gaps each get a phase ahead of
  surface work, because both are success-criteria work and the rule forbids deferring it out: new
  phase 9 closes the affected-key under-approximation (criteria 1/2/4 — a repair that misses a
  retracted key breaks the very promise criterion 1 names), new phase 10 closes the
  decomposed-combiner hidden-state gap (criterion 4 — only `Idempotent` combiners get equivalence
  coverage under retraction today), surface moves to 11. Phase 9 decides: the delete-aware
  mechanism is the **existing fingerprint sidecar** (`sources.md` §"The fingerprint sidecar")
  re-partitioned at *group* grain — one row per output group key holding an order-insensitive
  digest of that group's contributing rows — so a group that vanishes from the source still has a
  sidecar comparandum and shows up on the diff's `FULL OUTER JOIN`. Not a second lineage
  mechanism, not a tombstone log. Two consequences accepted: the discovery read is a **full**
  source scan (a clamped rescan compared against full stored digests would flag every out-of-clamp
  group, i.e. degrade to whole-table repair every run — the spec already licenses a snapshot
  source degrading to a full read), and the affected-key relation becomes a single canonical
  `delta_key` column joined by key *expression* rather than by columns, uniformly on both paths,
  because a deleted group's typed column values are unrecoverable by construction.

- 2026-08-10 (implement 9): landed group-grain fingerprint-sidecar discovery for the
  `mutable_snapshot` posture — the existing per-row sidecar machinery reused at a distinct
  `projection_identity` text (`repair:group=<cols>:digest=<cols>`, same table, never collides with
  the per-row scheme), digest columns sourced from the cell's own already-derived
  `fingerprint_projections` rather than re-deriving. `emit_per_group_recompute`'s DELETE/INSERT
  joins moved from per-column key joins to the single canonical `delta_key` expression, since a
  vanished group's typed column values are unrecoverable by construction — both the clamped-scan
  and sidecar-diff discovery paths now produce the same one-column relation shape.
  `resolve_live_per_group_recompute_cell` gained a `dialect` parameter and a `RepairDiscovery`
  verdict (`ClampedScan` for append-only, `SidecarDiff` for `MutableSnapshot`, `Err` on non-DuckDB).
  Absent/stale comparandum unions the currently-observed keys with every stored output key (a sound
  over-approximation, self-healing on refresh). Creation runs now seed the sidecar's initial
  comparandum so the first live repair doesn't take the absent-comparandum degradation every time.
  All standing gates green.

- 2026-08-10 (plan 10): no reshape — phase 9 closed clean and phase 11 (surface) stands as
  written; phase 9's three "for the next planner" notes (bit_xor digest collision risk, the
  unconfirmed snapshot-reconcile sidecar seed, the untested *stale* group comparandum) are
  hardening of an already-shipped mechanism, not success-criteria work, and stay out rather than
  becoming rows. Phase 10 decides: the fix is the EXISTING `state_augmented_projection` applied to
  the repair's raw pre-compile model SQL — the same widening `execute_windowed_keyed`/
  `execute_snapshot_reconcile` already apply, not a repair-specific projection — with the
  four-times-duplicated state-column collection promoted to
  `CumulativeClassification::state_columns()`. Second decision: the `diff_patch` suppression
  predicate compares hidden state columns alongside the presented compared columns, so a group
  whose presented value is unchanged but whose state moved is still rewritten (strictly less
  suppression, sound by construction) rather than left with stale state behind a correct value.

- 2026-08-10 (implement 10): landed `CumulativeClassification::state_columns()`, replacing three
  hand-rolled copies; `repair_augmented_model_sql` widens the repair candidate/insert (and the
  `diff_patch` compared-column set) with the fold's own hidden state columns before compiling —
  the same widening the ordinary fold path already applies. Discovered and fixed a real, latent
  bug the widened mutation-loop test exposed: a repair `PlanCell`'s `group` string was built from
  SQL-declaration column order while the canonical `ColumnGroup::name()` (used by
  `matching_write_pin`) is alphabetical, so a `write: diff_patch` pin over a multi-column FD group
  whose SQL order wasn't already alphabetical (`OrderMonotone`'s `(max_by_val, max_by_ord)`)
  silently never matched — fixed by sorting the repair cell's own column list in `derive.rs`
  before building its group string. `repair_pool_upholds_equivalence_under_retraction` now drives
  `OrderMonotone` through the full mutation loop; the matching `KnownBug` registry entry and spec
  divergence are deleted.

- 2026-08-10 (plan 11): no reshape — phase 10 closed clean and phase 11 is the last row; phase 10's
  `group.name()`-adjacent audit note and phase 9's hardening items are not success-criteria work and
  stay out. Phase 11 decides: the affected-key **discovery posture** (clamped current-source scan vs
  group-grain sidecar diff) becomes a pure single-owner predicate in
  `maintenance::repair` that both the runtime resolver and `smelt explain` call — explain never
  re-derives `facts.mutation == MutableSnapshot` itself, and builds the trigger source's facts via
  the existing `smelt_db::queries::maintenance::source_facts`. The repair stanza is
  technique-scoped (`PerGroupRecompute` only), so every non-repair cell's rendering stays
  byte-identical; the `diff_patch` delete-leg verdict comes from the real
  `choice::resolve_cell_choice`, not a display-only re-derivation.

- 2026-08-10 (implement 11): landed the surface work — `RepairDiscoveryPosture` +
  `discovery_posture` in `smelt-logical::maintenance::repair` (single-owner predicate, now called
  by both `maintenance_driver.rs`'s runtime resolver and `smelt explain`), a repair stanza in
  `build_maintenance_plan_report` for `Technique::PerGroupRecompute` cells only (key slice, read
  bound, affected-key discovery mechanism, and — via the real `choice::resolve_cell_choice` — the
  `write: diff_patch` mechanism and delete-leg verdict), the spec sentence on the `smelt explain`
  CLI bullet, and the docs-site guide section + `--technique` name-list corrections
  (`column_scoped_merge`/`per_group_recompute` were missing from both reference pages). All 6
  planned tests pass; standing gates green. This was the outcome's last phase row.

- 2026-08-10 (plan 12, terminal): all 11 rows `done`; success criteria judged met against the phase
  summaries and marked complete. Evidence — (1) retraction over a non-invertible keyed combiner now
  derives a `PerGroupRecompute` cell instead of refusing (phase 5's narrowing of `derive_new_data`'s
  faithful-fold source-posture branch); (2) admission is proof-gated by `derive_affected_keys`
  (phase 2, fail-closed) + the reused scan-clamp bound, with `Refusal::RepairKeysNotDiscoverable` /
  `RepairSliceUnbounded` refusing by name (phases 3/5); (3) `diff_patch` is a registered write
  mechanism (`WriteSelection::DiffPatch` / `ChosenTechnique::DiffPatch`) with the single pure
  emitter `emit_diff_patch`, routed in phase 7 — `cargo test -p smelt-runtime --test
  statement_parity` 21 passed, structural no-authoring leg included; (4) `maintenance_conformance`
  covers retraction → per-group repair and reconcile-via-diff-patch against the full-refresh oracle
  (phase 8's `repair.rs`, 59 passed), with the two gaps it discovered *closed* rather than
  registered — delete-aware group-grain sidecar discovery (phase 9) and decomposed-combiner hidden
  state columns (phase 10, its `KnownBug` entry deleted); (5) `smelt explain` renders a
  technique-scoped repair stanza with key slice, read bound, discovery posture and the `diff_patch`
  delete-leg verdict (phase 11); (6) `verify-phase.sh` green plus `walk_coverage` 4 passed — no new
  whole-text scans. Remaining out-of-scope residue is unchanged and recorded in phases 7 and 9
  summaries (unenforced region-`DeleteInsert` write pin; three sidecar hardening items).

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->

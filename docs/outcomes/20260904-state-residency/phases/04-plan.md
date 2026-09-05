# Phase 4 plan — availability resolution as a pure derivation step

## Objective

Build the late availability-resolution step the degradation contract requires, as pure data and
pure functions in `smelt-logical`'s maintenance layer, and parse the `state.warehouse_tables`
key that feeds it. Advances success criterion 3 (the pure function, the recompute-family
downgrade, the record on the cell) and the parsing half of criterion 5. No call site consumes
the resolution yet — phase 5 wires it; ideal derivation stays untouched, which is itself
normative (`state.md` §"The degradation contract": early resolution violates the spec).

## Spec delta

`docs/specs/smelt_yml.md` §"Top-level keys" — the `state` row currently names only `mode:`.
Extend it to `mode:` + `warehouse_tables:` (`allowed` | `none`, default `allowed`), pointing at
`state.md` §"Opting out of warehouse bookkeeping" for semantics and at `state.md`
§"The degradation contract" for the consequence. One row edit; `state.md` is already the
normative owner (phase 1), so nothing new is asserted here.

## Tests

`crates/smelt-core/src/config.rs` (unit tests, alongside the existing `state.mode` D-47 block):
- `warehouse_tables_defaults_to_allowed` — a `smelt.yml` with no `state:` block, and one with
  `state: { mode: intervals }`, both yield `WarehouseTables::Allowed`.
- `warehouse_tables_none_parses` — `state:\n  warehouse_tables: none` parses to the deny variant.
- `warehouse_tables_unknown_value_is_an_error` — `warehouse_tables: sometimes` fails loudly
  (same shape as the existing `mode: bogus` test), never defaults silently.

`crates/smelt-logical/tests/maintenance_availability.rs` (new):
- `full_availability_changes_nothing` — resolving with every structure available leaves each
  cell's technique identical and records no downgrade.
- `keyed_fold_downgrades_to_the_recompute_family` — a `KeyedFold` cell under an unavailable
  ledger becomes `PerGroupRecompute`.
- `column_scoped_merge_downgrades_to_the_recompute_family` — same for `ColumnScopedMerge`.
- `region_recompute_cells_require_no_structure` — `DeleteInsert`/`PerGroupRecompute` cells are
  untouched even with every structure unavailable.
- `the_record_names_the_original_technique_and_the_missing_structure` — the `StateDowngrade` on
  the downgraded cell carries the pre-downgrade `Technique`, the `StateStructure`, and a reason
  string (the counterfactual `smelt explain` will print in phase 6).
- `downgraded_cells_need_no_structure` — fixpoint: after resolution no cell's technique has a
  required structure that is unavailable.
- `resolution_is_idempotent` — resolving twice leaves the technique and the first record intact
  (the record still names the *original* technique, not the already-downgraded one).
- `warehouse_tables_none_denies_every_engine_resident_structure` — the availability constructor
  built from `WarehouseTables::None` reports every `StateStructure` unavailable regardless of
  what the backend can realise.
- `ideal_derivation_records_no_downgrade` — a plan straight out of
  `derive_maintenance_plan_with_referential_integrity` has `state_downgrade: None` on every
  cell (late resolution: derivation never consults availability).

## Tasks

1. `crates/smelt-core/src/config.rs`: add `WarehouseTables { Allowed (default), None }`
   (`#[serde(rename_all = "lowercase")]`, deny-unknown behaviour matching `StateMode`) and a
   `warehouse_tables: WarehouseTables` field on `StateConfig`; write the three config tests red.
2. `docs/specs/smelt_yml.md`: the `state` row edit above.
3. New `crates/smelt-logical/src/maintenance/availability.rs`, exported from `maintenance/mod.rs`:
   - `pub enum StateStructure { MergeLedger, ReconciliationLedger, ObservedOutputDeltas,
     FingerprintSidecar }` with `as_str()` spellings matching `state.md` §"The state-structure
     inventory" rows, and a doc comment citing that table as the source of the classification.
   - `pub fn required_state_structure(t: Technique) -> Option<StateStructure>` — `KeyedFold` →
     `ReconciliationLedger` (the never-fold-twice frontier record), `ColumnScopedMerge` /
     `InPlaceUpdate` → `MergeLedger` (the transactional merge ledger), `DeleteInsert` /
     `PerGroupRecompute` → `None` (the recompute family needs no bookkeeping to be correct).
     Exhaustive `match`, no wildcard arm, so a new `Technique` is a compile error here.
   - `pub struct StateAvailability` over a `BTreeSet<StateStructure>` of *available* structures,
     with `all()`, `none()`, and a constructor taking `(WarehouseTables, &[StateStructure])`
     (the backend's realisable set) that intersects the two.
4. Same module: `pub fn recompute_equivalent(cell: &PlanCell) -> Technique` — targeted-write
   corners (`Corner::FoldDelta`, `Corner::ColumnMerge`, or a cell carrying a `key_scope`) →
   `PerGroupRecompute`; region-write corners → `DeleteInsert`. Doc-comment the mapping against
   `state.md` "cheapest member of the recompute family that preserves the equivalence invariant".
5. `PlanCell` gains `pub state_downgrade: Option<StateDowngrade>` (`StateDowngrade { original:
   Technique, missing: StateStructure, reason: String }`, `Serialize` for phase 6's `--json`);
   add `state_downgrade: None` to the ~38 existing `PlanCell` literals (`rg -n 'PlanCell\s*\{'`).
6. Same module: `pub fn resolve_availability(plan: &mut MaintenancePlan, avail:
   &StateAvailability)` — for each cell whose `required_state_structure` is unavailable, record
   the downgrade (skipping cells that already carry one, for idempotence) and swap the technique
   for `recompute_equivalent`. Pure; no I/O, no backend, no config types beyond
   `WarehouseTables`.
7. Write the `maintenance_availability.rs` tests green; add a module-level doc comment naming
   this as step 2 of `state.md` §"The degradation contract" and stating that no consumer calls
   it yet (phase 5).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test maintenance_availability`
- `cargo test -p smelt-core config::` (the `state:` block tests)
- `cargo test -p smelt-logical --test walk_coverage` and
  `cargo test -p smelt-runtime --test statement_parity --test execute_parity` — must be
  unchanged: this phase adds a step nothing calls yet.

## Commit message

`feat(state-residency): pure availability resolution + state.warehouse_tables parsing`

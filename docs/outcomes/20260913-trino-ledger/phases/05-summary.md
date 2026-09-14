# Phase 5 summary — The two invariants as standing tests

**Shipped:**
- `crates/smelt-runtime/tests/availability_seam/builders.rs` (new): a census of all 11 `pub fn`
  builder entry points in `smelt-state::{ledger, observed_delta, tombstone}` against the
  `StateStructure` each realises.
  - `every_state_builder_entry_point_is_classified` — structural, two-sided: scans the three
    `smelt-state` files for `pub fn` items whose return type names one of the three
    dialect-refusing result types and diffs against the census by name.
  - `a_builder_answers_exactly_when_its_structure_is_claimed` — calls each real builder with
    sample arguments, over all 4 dialects × 11 entries, and asserts `is_ok()` equals
    `realisable_state_structures(dialect).contains(&structure)`.
  - `state_builder_call_sites_stay_inside_the_gated_modules` — structural: every production
    `::<census-name>(` call site across the whole workspace must live under
    `smelt-runtime/src/maintenance_driver/`, `smelt-runtime/src/execute/project/ledger_reset.rs`,
    `smelt-runtime/src/execute/key_addressed.rs`, `smelt-backend-bigquery/src/`, or
    `smelt-state/src/` itself.
- `crates/smelt-logical/tests/maintenance_availability/realisation.rs`: `has_emitters` trimmed to
  answer only `StateStructure::FingerprintSidecar` (the one row with no `smelt-state` builder);
  the other four rows now call `realisable_state_structures` directly, so
  `every_claimed_structure_has_a_builder` narrowed to the `FingerprintSidecar` row only (the other
  four would otherwise compare `realisable_state_structures` against itself — moved to
  `builders.rs`'s test above). `the_sidecar_claim_matches_the_backend_capability` widened to
  include `BackendCapabilities::trino_iceberg()`.
- `crates/smelt-logical/tests/maintenance_availability/trino_invariants.rs` (new): "absence ⇒
  downgrade" made exhaustive over all 6 `Technique` variants × the 4 `key_scope` shapes (none,
  `UpstreamKeyed`, `DownstreamGrainOverUpstream`, `EnrichmentKeyed`) under Trino's empty
  availability. `every_technique_downgrades_or_needs_nothing_on_trino` asserts, per cell: no
  requirement ⇒ untouched; a requirement ⇒ resolves to `recompute_equivalent`, carries a
  `StateDowngrade` naming the ideal technique and missing structure, and its OWN
  `required_state_structure` is `None` post-resolution (the reachability half — no builder is ever
  reached). `the_ideal_plan_survives_resolution_on_trino` is the non-vacuity twin under
  `StateAvailability::all()`.
- `crates/smelt-cli/tests/trino_explain_downgrade.rs`: `explain_on_trino_downgrades_every_structure_bearing_shape`
  stages one project with a model per structure-bearing shape (keyed fold, column-scoped merge via
  a `LEFT JOIN`-enriched fact/dimension pair, key-addressed `PerGroupRecompute` via a two-model
  chain reading a model edge, succession patch) and asserts `smelt explain --json` exits 0, names
  each shape's ideal technique in `state_downgrade.original`, and prints no `Unsupported*Dialect`
  refusal text.

**Decisions:**
- Split the ledger census by role rather than treating `ledger.rs` as one row: `ledger_upsert_sql`
  realises `StateStructure::MergeLedger` (idempotent bookkeeping); the other five ledger functions
  realise `StateStructure::ReconciliationLedger` (the never-fold-twice refusal). The two structures
  happen to agree on every dialect today, so this has no effect on the current test verdicts — it
  is recorded for when a future dialect realises one without the other.
- Narrowed `every_claimed_structure_has_a_builder` (realisation.rs) to `FingerprintSidecar` only
  rather than leaving it iterate all 5 structures — once `has_emitters` derives the other four from
  `realisable_state_structures` directly (per the plan), comparing them was a tautology; the real
  check for those four now lives in `builders.rs`'s `a_builder_answers_exactly_when_its_structure_is_claimed`,
  which calls the actual builder rather than a restated table.
- Built the column-scoped-merge and key-addressed fixtures for test 7 by hand (not via
  `smelt_maintenance_testkit::recipe::ValueEnrichedRecipe`/`dag::keyed_chain_dag` directly) since
  those helpers are wired for the property-conformance harness (needs a live DuckDB file + specific
  staging conventions); the model/source YAML shapes were read off them and reproduced literally so
  `smelt explain` (offline, no live connection) sees the same admitted techniques.

**For the next planner:**
- The `StateStructure::MergeLedger`/`ReconciliationLedger` split recorded in `builders.rs`'s census
  doc comment is a place to check first if a future dialect (or a BigQuery follow-up) ever realises
  one without the other — the two-sided census test would need per-structure assertions rather than
  the current single `realisable_state_structures` call per dialect if that divergence lands.
- No fixes were needed anywhere in `smelt-state`/`smelt-runtime`/`smelt-backend-bigquery` — every
  existing call site of a census entry point already lives inside the allowed modules, and every
  builder's `is_ok()` already agreed with `realisable_state_structures`. This phase was pure gate
  construction, no behavior change.
- Phase 6 (`contract.deferral` / `frozen_horizon` / `retain_departed` on Trino) can reuse
  `trino_explain_downgrade.rs`'s `stage_trino_four_shapes_project` fixture shapes as a starting
  point if it needs a project with multiple structure-bearing models already staged.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full `cargo test`, `example_diagnostics`); no baseline bumped.
- `cargo test -p smelt-runtime --test availability_seam` — pass (10/10).
- `cargo test -p smelt-logical --test maintenance_availability --test state_realisability_docs` —
  pass (32/32, 4/4).
- `cargo test -p smelt-cli --test trino_explain_downgrade --test trino_spec_freshness` — pass
  (4/4, 5/5).
- `cargo test -p smelt-state --test ledger_dialect` — pass (18/18).
- `bash .claude/scripts/large-file-check.sh` — pass, no bump.

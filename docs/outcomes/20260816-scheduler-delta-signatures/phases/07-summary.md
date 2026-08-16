# Phase 7 summary — live keyed-seed resolution

## Shipped

- `smelt_runtime::propagation::KeyedSeedDiff` + `keyed_seed_diffs_to_read` (pure): one
  `(upstream, consumer)` descriptor per admitted key-addressed edge, reusing
  `crate::execute::model_edges_for` / `build_maint_source_facts` / `maintenance_driver::
  resolve_live_key_addressed_model_edge_cells` — never a second derivation
  (`crates/smelt-runtime/src/propagation.rs`).
- `keyed_seed_diff_result_to_key_values` (classifier: `BackendError::UnsupportedFeature` →
  `KeyValues::Unresolved` naming the dialect, everything else propagates) and
  `fold_keyed_seed_values` (union-across-consumers fold, empty-and-resolved stays resolved).
- `smelt_runtime::propagation_live::resolve_keyed_seeds(backend, config, target, diffs)`: the
  live backend read, one `diff_repair_group_sidecar_changed_keys` call per descriptor, folded per
  upstream. Read-only — never refreshes a sidecar partition.
- `crate::execute::model_edge_source_identity` (`pub(crate)`): the upstream identity formula
  (`smelt.models.<addr>` + schema-qualified table) extracted out of `dispatch_key_addressed_model_edge`
  and reused by the seed resolver.
- `propagation::plan_since_upstream_full` renamed to `plan_since_upstream_live` and made `pub`
  (both channels at once); the two single-channel wrappers unchanged.
- `smelt-cli`'s `run_since_upstream` now resolves live keyed seeds off the same backend
  connection the observed-delta read already opens, and calls `plan_since_upstream_live`.
- Spec: `incremental_models.md` §"Keyed dirt-sets and the narrowed refusal" gained the
  per-`(upstream, consumer)` union-composition sentence; the "live resolution ... not yet wired"
  clause dropped from the scheduler-currency Known Divergences bullet.

## Decisions

- Fixed a real pre-existing bug in `model_edges_for` (`execute.rs`): its `addr` computation used
  raw `segs.join(".")` instead of stripping the `models`/`sources` breadcrumb (`bare_name`), so
  it silently found no edge for any ref spelled `smelt.models.<addr>` — only the bare `smelt.<addr>`
  spelling worked. Discovered because `keyed_seed_diffs_to_read` (which reuses this function)
  returned zero descriptors for a `smelt.models.agg`-spelled fixture. Fixed by delegating to
  `crate::propagation::bare_name`, matching `derive_clamp_and_locality_pass`'s own convention.
  This is a real-world correctness fix for the existing (phase 2-4) key-addressed dispatch path
  too, not just this phase's new code.
- `resolve_keyed_seeds` does not thread per-model frontmatter `target:` overrides (a
  `KeyedSeedDiff` carries bare addresses/db names, not `ModelMetadata`) — documented as a known
  gap in the function's own doc comment. `smelt.yml`-level `models:` overrides still resolve
  correctly via the model address.
- Verified empirically (not from prior assumption) that `smelt.models.<addr>` is NOT a
  diagnostics-accepted ref spelling for a top-level model — the analyzer suggests `smelt.<addr>`
  instead. Test fixtures for this phase use the bare spelling; `KeyedSeedDiff`/`model_edges_for`
  still accept both spellings defensively (via `bare_name`).

## For the next planner

- Row 8 (persisted per-source watermark) can now build on both `resolve_observed_delta_lookup`
  and `resolve_keyed_seeds` as the two live-read precedents.
- The `model_edges_for` bug fix (see Decisions) may be worth a standalone regression note in a
  future audit — it was silently broken for the `smelt.models.X` ref spelling across every
  existing production call site (the ordinary key-addressed dispatch branches, not just this
  phase's new code), yet no existing test caught it because production fixtures for that function
  happened to all use the bare `smelt.X` spelling.
- `keyed_seed_diffs_to_read` recomputes `resolve_live_key_addressed_model_edge_cells` for every
  `(delta, consumer)` pair in the workspace — O(deltas × models), each doing a real maintenance-plan
  derivation. Fine at today's scale (mirrors `observed_delta_keys_to_read`'s own per-delta cost);
  worth revisiting only if a workspace with hundreds of `--source` deltas surfaces as slow.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full `cargo test`,
  `example_diagnostics`).
- `cargo test -p smelt-runtime --test since_upstream_propagation --test key_addressed_model_edge_lowering` — 16 + 28 passed.
- `cargo test -p smelt-cli --test since_upstream` — 11 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 76 passed.
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity` — 4 + 23 passed.
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md` — no matches.

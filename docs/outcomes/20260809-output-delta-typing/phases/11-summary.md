# Phase 11 summary — bare `smelt.<addr>` model-reference leaves resolve through `model_verdicts`

## Shipped

- `crates/smelt-logical/src/analysis/output_delta.rs`: `normalize_model_key` (strips an optional
  `models.` breadcrumb, lowercases) applied on the insert side (`derive_workspace_output_deltas`)
  and a new `lookup_model_verdicts` helper applied on every read site
  (`model_whole_shape`/`seed_for_model_column`/`resolve_bare_table_column_shape`) — the lookup
  tries the normalized key, then the same key with a `models.` breadcrumb, so it matches a map
  keyed either way regardless of which producer built it.
- Bare-name resolution (`seed_for_leaf_name`'s fallback, and the new
  `resolve_bare_table_column_shape` used by `resolve_column_ref_shape`'s `RelationSource::Table`
  arm) now: checks a declared source first, then `model_verdicts`, else fails closed to
  `bare_relation_miss` — a `General{reason}` naming both misses.
- `docs/specs/model_properties.md` §"Output-delta shape" — two-sentence extension to the
  model-reference-leaf paragraph stating breadcrumb-insensitivity and the source-wins-over-model
  precedence for bare names.
- 4 new unit tests in `output_delta.rs` (whole-leaf + per-column bare resolution, breadcrumb
  insensitivity in both directions, source-over-model precedence, both-miss reason), 1 new
  workspace test (`three_hop_chain_with_bare_refs_composes`), 1 new Salsa-layer test
  (`three_hop_bare_ref_chain_edge_is_keyed`, confirmed red on the pre-fix code: it previously
  produced `General { reason: "relation 'a' has no declared mutation profile" }`).

## Decisions

- Confirmed in-plan: for a bare name matching both a declared source and a model verdict, the
  source wins — preserves all existing behaviour exactly (only names that previously fail-closed
  to `General` change), and a source's declared mutation profile is the more specific fact.
- Fixed a shape mismatch the plan hadn't fully spelled out: normalizing only the insert side is
  not sufficient when a test (or a future producer) hands `OutputDeltaTransfer` a raw,
  un-normalized `model_verdicts` map directly (bypassing `derive_workspace_output_deltas`). Added
  `lookup_model_verdicts`, which tries both the normalized key and the same key with a `models.`
  breadcrumb, so lookup is robust to either map-key convention independent of the insert-side fix.
- Avoided a new production `.expect(...)` (would have tripped the `hardening_budget` ratchet):
  `model_whole_shape`'s meet-fold uses `Iterator::fold` seeded from the first shape instead of
  `.reduce(...).expect(...)`.

## For the next planner

- Nothing flagged as out of scope or deferred — phase 10's summary said phase 11 was the only
  remaining item, and this phase closed it without surfacing a new gap. All 6 planned tests plus
  the standing gates (`walk_coverage`, `output_delta_spec`, `output_delta_workspace`,
  `typed_model_edge`, `explain_maintenance`, `maintenance_conformance`, `verify-phase.sh`) are
  green. This appears to close out the phase table for
  `docs/outcomes/20260809-output-delta-typing/outcome.md` (row 11 was the last row) — worth
  checking whether all 6 success criteria are now satisfiable end-to-end before marking the
  outcome `done`.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full workspace
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test output_delta_workspace --test walk_coverage --test output_delta_spec` — 15 passed.
- `cargo test -p smelt-db --test typed_model_edge` — 6 passed.
- `cargo test -p smelt-cli --test explain_maintenance --test maintenance_conformance` — 84 passed.

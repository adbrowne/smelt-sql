# Phase 11 plan — Model-reference leaf resolves a bare `smelt.<addr>` upstream ref

## Objective

Make the model-reference leaf rule hold for **both** ref spellings. Today
`OutputDeltaTransfer` only consults `model_verdicts` when the walk-normalized leaf name
literally carries a `models.` breadcrumb (`output_delta.rs:228`, `:403`); a bare
`smelt.<addr>` ref — the form every fixture and the dag generator emit — falls through to
`seed_for_source_name` and fail-closes to `General`, so composition stops at 2 hops. This
advances criterion 1 (a registered transfer rule must actually fire) and criterion 3's
headline claim that incrementality composes end-to-end through a chain rather than per hop.

## Spec delta

`docs/specs/model_properties.md` §"Output-delta shape", the model-reference-leaf paragraph
(currently lines 215–217). Extend it to state the resolution rule precisely: a leaf
resolves against the referenced model's verdicts **regardless of whether the reference is
spelled with the `models.` breadcrumb** (`smelt.models.<addr>` and `smelt.<addr>` are the
same reference), and for a bare name a declared source with that name wins over a model
verdict — a bare name that matches neither is `General{reason}` naming both misses. Keep it
to two sentences; no other section changes.

## Tests

Red-green, in this order.

1. `crates/smelt-logical/src/analysis/output_delta.rs` (unit)
   `bare_model_reference_leaf_resolves_through_model_verdicts` — `SELECT id, amount FROM
   smelt.upstream` with `model_verdicts` keyed `upstream` yields the upstream's own
   per-column shapes, not `General`. Covers both the whole-leaf (`seed_for_leaf_name`) and
   the per-column-ref (`resolve_column_ref_shape`) paths.
2. `.../output_delta.rs` (unit) `model_key_lookup_is_breadcrumb_insensitive` — a verdict map
   keyed `models.upstream` (the `smelt-db` key form, built from the ref path as spelled)
   resolves for SQL spelling `smelt.upstream`, and a map keyed `upstream` resolves for SQL
   spelling `smelt.models.upstream`. Pins normalization on **both** sides.
3. `.../output_delta.rs` (unit) `declared_source_wins_over_same_named_model_for_bare_ref` —
   a bare name matching both a declared `SourceFacts` and a `model_verdicts` entry seeds
   from the source (precedence regression; no existing behaviour changes).
4. `.../output_delta.rs` (unit) `bare_ref_matching_neither_names_both_misses` — the
   fail-closed `General` reason mentions the unresolved relation (no silent optimism).
5. `crates/smelt-logical/tests/output_delta_workspace.rs`
   `three_hop_chain_with_bare_refs_composes` — a→b→c where b and c use bare
   `smelt.<addr>` refs; c's verdict is the composed shape (`KeyedUpsert` for the keyed
   aggregation chain), and the pre-existing `smelt.models.` chain tests stay green.
6. `crates/smelt-db/tests/typed_model_edge.rs` `three_hop_bare_ref_chain_edge_is_keyed` —
   Salsa layer: for a 3-model project written with bare refs, `model_edges_for` on the tail
   model reports `output_shape: Some(KeyedUpsert{..})` on its inbound edge (today
   `General`). This is the criterion-1/5 end-to-end evidence.

## Tasks

1. Write tests 1–6 red; confirm each fails for the stated reason (not a fixture error).
2. Add one private helper `normalize_model_key(name) -> String` in `output_delta.rs`:
   lowercase, strip an optional leading `models.` breadcrumb (mirrors the phase-6 precedent
   in `analysis::fingerprint::relation_matches_source`).
3. Apply it on the **insert** side in `derive_workspace_output_deltas` (line ~762) so a
   `smelt-db`-built `ModelDeltaInput.address` of `models.<addr>` and a runtime-built
   `canonical_path()` of `<addr>` land on the same key.
4. Apply it on the **lookup** side: `seed_for_model_column` (line ~254) and
   `seed_for_leaf_name` (line ~228).
5. Route bare names through one shared resolver used by both `seed_for_leaf_name` and
   `resolve_column_ref_shape`'s `RelationSource::Table` arm (line ~403), so the two paths
   cannot drift: explicit `sources.` prefix → source only; explicit `models.` prefix →
   model only; bare → declared source if one matches, else `model_verdicts`, else
   `General{reason}` naming both misses.
6. Make the §Spec delta edit.
7. Run the gates; if a pre-existing fixture flips shape (a bare name that was `General` and
   is now typed), verify the new verdict is the correct composed one before updating any
   golden — a widening here would be a real bug, not a fixture refresh.

## Verification

- `bash .claude/scripts/verify-phase.sh` — must be all green.
- `cargo test -p smelt-logical --test output_delta_workspace --test walk_coverage --test
  output_delta_spec`
- `cargo test -p smelt-db --test typed_model_edge`
- `cargo test -p smelt-cli --test explain_maintenance --test maintenance_conformance` — the
  surface and the standing generative gate, both of which read these verdicts.

## Commit message

`fix(output-delta): resolve bare smelt.<addr> model-reference leaves through model_verdicts`

# Phase 2 summary — Walk transfer rules for the output-delta verdict per column group

**Shipped:**
- `docs/specs/model_properties.md` §"Output-delta shape": new leaf/base-relation transfer-rule
  row (`append_only`+clock ⇒ `AppendOnlyWindow{axis}`; `change_feed`+`delta_identity` ⇒
  `KeyedUpsert{identity}`; everything else ⇒ `General{reason}`, fail-closed) and a
  model-reference-leaf sentence (referenced model's own verdict where available, else
  `General`); §Surface maturity row now `partial (derived; not yet consumed by edge typing)`.
- `crates/smelt-logical/src/analysis/output_delta.rs` (new): `OutputDelta` lattice
  (`AppendOnlyWindow`/`KeyedUpsert`/`General`, `rank()`/`meet()`), `OutputDeltaFacts`
  (per-column walk verdict), `SourceFacts` (leaf-seeding input, mirrors
  `input_delta::SourceShape`), `OutputDeltaTransfer` (a `Transfer` impl: leaf seeding,
  selection/projection pass-through via per-column-ref meet, `UNION ALL` per-position meet
  (fail-closed for any other set op, naming it), `GROUP BY`/`DISTINCT` keyed-upsert promotion,
  join meet + `OneToMany` degrade via `join_shape::fan_out`, window columns forced to
  `General`), and entry point `derive_output_delta` (runs the walk, calls the existing
  `maintenance::grouping::derive_column_groups` for the column-group partition, takes the meet
  per group). Registered in `analysis/mod.rs`; `OutputDelta`/`OutputDeltaFacts`/
  `derive_output_delta` re-exported from `lib.rs`.
- `crates/smelt-logical/src/analysis/walk.rs`: `resolve_alias_source` made `pub(crate)` so
  `output_delta.rs` reuses the same column-reference-to-alias resolution `PropertyTransfer`
  uses, instead of a second copy.
- 16 new unit tests in `output_delta.rs` (lattice, all four leaf-seeding cases, selection/
  projection pass-through, UNION ALL meet, GROUP BY over append-only/general, join meet,
  OneToMany degrade, window-column isolation, unregistered-set-op fail-closed, CTE/derived-table
  composition, independent column groups). 2 new tests in `output_delta_spec.rs`
  (`leaf_seeding_row_is_present`; `surface_row_exists_for_output_delta` now asserts the exact
  maturity string).

**Decisions:**
- Output-delta shape is resolved **per column reference**, not per whole scope: a select item's
  shape is the meet of every column ref its expression embeds, each independently chased to its
  own leaf/child. This is what lets `groups_are_independent` hold inside a single joined scope
  (one column reading an append-only side, another reading a mutable side, no blanket
  scope-wide collapse) while still letting a genuinely cross-source expression (`e.x + d.y`)
  take the meet of both — the same "per attribution, not per syntactic position" philosophy
  `maintenance::grouping`'s mutation-sensitivity walk already uses.
- A `OneToMany`-proven join forces **every** output column of that scope to `General`
  (row-multiplication is a whole-relation effect), applied after the per-column-ref resolution
  and before the `GROUP BY` keyed-promotion step, so a fan-out can never be masked by a
  downstream aggregation.
- A reference-free expression (`COUNT(*)`, a literal, an opaque call) fails closed to `General`
  rather than inheriting the scope's dominant shape — no column reference means no source to
  attribute addressability to.
- `derive_output_delta` bridges to the existing `maintenance::grouping::derive_column_groups`
  via a small local adapter (`SourceFacts` → `maintenance::SourceFacts`) rather than threading a
  second source-facts shape through the maintenance layer; `analysis` depending on
  `maintenance::grouping` for this one call is an intra-crate (not cross-crate) reference, so it
  doesn't trip the crate-layering rule, and reuses the grouping logic verbatim per the plan's
  instruction rather than re-deriving it.

**For the next planner:**
- Phase 3 (edge typing) has a working per-group verdict function to call:
  `derive_output_delta(sql, ctx, sources, skeleton_columns) -> Vec<(ColumnGroup, OutputDelta)>`.
  `SourceFacts` construction from real `sources.md` declarations (`SourceInfo` →
  `output_delta::SourceFacts`) is not yet wired anywhere — phase 3's edge typing is the first
  real caller and will need that adapter (mirroring `input_delta::SourceShape::from_source_info`).
- The model-reference leaf (`model_verdicts` field on `OutputDeltaTransfer`) exists but
  `derive_output_delta` always passes an empty map — no cross-model wiring yet. Phase 4's
  consumer fold is the intended place to thread a real upstream-model verdict map through.
- No phase-table reshape needed — phase 2's scope (walk + entry point, no edge/graph/plan
  wiring) matched the plan exactly.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, full workspace `cargo test`,
  `example_diagnostics`).
- `cargo test -p smelt-logical --test output_delta_spec --test walk_coverage` — 6/6 + 4/4 passed.
- `cargo test -p smelt-logical output_delta` — 16/16 unit tests + 1 spec-table test passed.

# Phase 2 plan — Walk transfer rules for the output-delta verdict per column group

**Objective.** Build the `OutputDelta` lattice and its transfer function as a
`Transfer` impl over the shared composition walk in `smelt-logical`, producing a
verdict per output column and folding it to a verdict per column group. Advances
success criteria 1 (transfer rules registered, unregistered operators fail closed)
and 6 (walk_coverage stays green with the new module). No consumer wiring —
edge typing (phase 3) and the plan/graph layer stay untouched.

## Spec delta

`docs/specs/model_properties.md` §"Output-delta shape", transfer-rule table: add
one **leaf row** the phase-1 table omits — a base relation (source/table
reference) seeds its shape from the source's declared mutation profile
(`sources.md`): `append_only` with a declared clock ⇒ `AppendOnlyWindow{axis =
partition column}`; `change_feed` with a `delta_identity` ⇒ `KeyedUpsert{identity}`;
everything else (`mutable_snapshot`, undeclared profile, clockless append-only,
change feed without `delta_identity`, an unresolved reference) ⇒ `General{reason}`,
fail-closed. Mirrors `input_delta_discovery`'s fail-closed default (an undeclared
profile is never optimistic). Also add one sentence after the table: a **model
reference** leaf takes the referenced model's own derived verdict where available,
otherwise `General` — the hook phase 4's consumer fold reads. §Surface maturity
for the row moves `not-yet` → `partial (derived; not yet consumed by edge typing)`.

## Tests

Red-green, all new unless noted.

- `crates/smelt-logical/tests/output_delta_spec.rs`
  - `leaf_seeding_row_is_present` — the transfer table names the leaf/base-relation
    case and its three profile outcomes.
  - `surface_row_exists_for_output_delta` (existing) — update expected maturity.
- `crates/smelt-logical/src/analysis/output_delta.rs` unit tests
  - `lattice_meet_degrades_never_recovers` — `meet` of any two shapes is the
    weaker; `AppendOnlyWindow ⊓ General == General`.
  - `append_only_source_seeds_window_shape` / `mutable_snapshot_source_seeds_general`
    / `change_feed_with_identity_seeds_keyed_upsert` — leaf seeding per profile.
  - `undeclared_profile_seeds_general_naming_the_source` — fail-closed leaf.
  - `filter_preserves_input_shape` / `projection_preserves_input_shape`.
  - `union_all_takes_the_meet_of_arms`.
  - `group_by_over_append_only_emits_keyed_upsert` (keys = the `GROUP BY` output
    columns) and `group_by_over_general_stays_general`.
  - `join_takes_the_meet` and `one_to_many_join_degrades_to_general` (uses the
    existing fan-out proof).
  - `window_function_column_is_general` — and its sibling non-window columns in
    the same scope keep their shape (per-column, not per-scope collapse).
  - `unregistered_operator_is_general_naming_the_operator` — e.g. `INTERSECT`,
    `Unsupported` node.
  - `cte_and_derived_table_compose_through_the_walk` — shape survives one CTE
    layer of renaming.
  - `groups_are_independent` — a model with one `General` column group and one
    `AppendOnlyWindow` group keeps both (no whole-model collapse).

## Tasks

1. Add `crates/smelt-logical/src/analysis/output_delta.rs`; register in
   `analysis/mod.rs`; re-export `OutputDelta` alongside the other verdicts.
2. Define `OutputDelta { AppendOnlyWindow{axis}, KeyedUpsert{keys}, General{reason} }`
   with `rank()` + `meet(self, other)` (degrade-only) and a doc comment citing
   `model_properties.md` §"Output-delta shape".
3. Define `OutputDeltaFacts { columns: Vec<(String, OutputDelta)> }` as the walk
   verdict — per-output-column shape, projection order, `ColumnLineage` used to
   carry a column's shape through renames the same way `PropertyTransfer` does.
4. Implement `OutputDeltaTransfer<'a> { ctx: &'a JoinContext, sources: &'a [SourceFacts] }`
   as a `Transfer`: leaf seeding (task 2 of the spec delta), selection/projection
   pass-through, `UNION ALL` per-position meet, `GROUP BY`/`DISTINCT` factory,
   join meet + `OneToMany` degrade via the existing fan-out proof, window-function
   columns to `General`, everything else `General{reason}` naming the construct.
5. Entry point `derive_output_delta(sql, ctx, sources, skeleton_columns) ->
   Vec<(ColumnGroup, OutputDelta)>`: run the walk, call the existing
   `derive_column_groups`, and take the meet of member columns' shapes per group.
   A degenerate/unresolvable group is `General{reason}`.
6. Make the spec edits of §Spec delta first (spec-first), then the code.
7. Confirm no new `.contains("` scan is introduced without a classification tag
   (walk_coverage picks the new file up automatically).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test output_delta_spec --test walk_coverage`
- `cargo test -p smelt-logical output_delta`

## Commit message

`feat(logical): output-delta shape verdict per column group via the composition walk`

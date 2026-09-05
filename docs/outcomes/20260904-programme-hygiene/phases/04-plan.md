# Phase 04 plan — make the enrichment fixture's record match what smelt derives

## Objective

`examples/timeseries/models/daily_events_enriched.sql` claims in both its frontmatter comment and
its body comment that MP11 wires a live column-scoped `MERGE` for the `{user_name}` cell. It does
not: the model's inner `JOIN` reads `raw.users` in the `ON` predicate, so every column group is
membership-sensitive and the cell derives `Technique::DeleteInsert`. Correct the fixture's comments
to the derived plan, correct the same false claim in the docs-site transcript attributed to this
same model, and delete `docs/TODO.md`'s "worth a follow-up correction" note. Advances success
criterion 5 (and criterion 6's "one consistent record" reading).

## Spec delta

None. No spec's normative surface changes — this phase corrects a fixture comment and a user-doc
transcript to match already-specified derivation, which `docs/specs/incremental_models.md`
§"Per-cell admission" already states correctly.

## Ground truth (already observed at plan time — do not re-derive from memory)

`cargo run -q -p smelt-cli -- explain daily_events_enriched --project-dir examples/timeseries`
reports for `group {user_name} on trigger UpstreamMutation { source: "raw.users" }`:
`corner: RecomputeRegion`, `technique: DeleteInsert`, `region key: WholeRow`,
`locality: NOT partition_local (source: raw.users, why: unclocked source: a change's footprint
projects onto no bounded partition interval of the output)`, `scan clamps: (none)`,
`admissible write patterns: region, full_rebuild`. All four cells derive `DeleteInsert`.

## Tests

No cargo test is added — the outcome is docs-only and no crate behaviour changes. The red-green
oracle is the `smelt explain` invocation above, run before and after the edit; the standing gates
below carry the rest.

## Tasks

1. Re-run the `smelt explain` command above and confirm the ground-truth block still holds (if it
   does not, stop and record the divergence in `docs/TODO.md` rather than writing a comment that
   will drift again).
2. Rewrite the fixture's **frontmatter** comment (`examples/timeseries/models/daily_events_enriched.sql`):
   keep the accurate parts — `event_id` is the source's declared `unique_key`, a `grain: partition`
   output has no `unique_key:` slot so row identity resolves `WholeRow`, and the retired
   `batched.unique_key` spelling never fed row-identity derivation — but drop the claim that the
   column-scoped `MERGE` (MP11) is what keys on it. State instead that the derived technique for
   every cell, `{user_name}` included, is the region `DELETE`+`INSERT` (`Technique::DeleteInsert`),
   and that `WholeRow` identity is what that region recompute is keyed by.
3. Rewrite the fixture's **body** comment's first paragraph: `raw.users` is still an unclocked,
   explicitly `mutation_profile: mutable_snapshot` dimension whose rename broadcasts to the
   `{user_name}` column group's `UpstreamMutation` cell — but that cell derives `DeleteInsert`,
   not a live column-scoped `MERGE`, because the enrichment is an inner `JOIN` reading the
   dimension in its own `ON` predicate (a row-admission read: membership sensitivity is row-scoped,
   so no column group of this `SELECT` can be proven value-only). Cite
   `docs/specs/incremental_models.md` §"Per-cell admission". Leave the `CAST`-vs-`date_trunc`
   paragraph untouched — it is still accurate.
4. `docs-site/docs/guide/incremental-models.md` §"Enrichment joins and dimension updates": the
   `$ smelt explain daily_events_enriched` transcript prints `corner: ColumnMerge` /
   `technique: ColumnScopedMerge` for the `{user_name}` cell, which that model does not derive.
   Replace those two lines with the real `RecomputeRegion` / `DeleteInsert` output, and reopen the
   paragraph that follows so it reads as: this model's plain inner `JOIN` is exactly the shape that
   *cannot* be proven row-preserving, so its dimension cell falls back to the region recompute; the
   targeted column-scoped `MERGE` is what a `LEFT JOIN` against a `unique_key`-declaring dimension
   earns instead. The paragraph already explains that condition correctly — only its opening
   sentence, which asserts this model got the `MERGE`, needs rewriting.
5. `docs/TODO.md`: delete the sentence beginning "Its own doc comment (predating this resolution)
   still claims a live column-scoped `MERGE`…" through "…worth a follow-up correction." Keep the
   surrounding paragraph (the fixture is unaffected by the sensitivity-precision fix) — it is
   accurate and still load-bearing.
6. Re-check no other tracked file restates the corrected claim:
   `rg -n 'column-scoped' examples/ docs-site/docs/guide/incremental-models.md docs/TODO.md` and
   confirm every surviving hit is either generic or about a genuinely `ColumnScopedMerge`-deriving
   shape (`ValueEnrichedRecipe`). Record any hit you cannot resolve in `docs/TODO.md` rather than
   widening this phase.

## Verification

- `cargo run -q -p smelt-cli -- explain daily_events_enriched --project-dir examples/timeseries`
  — output matches every claim the rewritten comments make.
- `rg -n 'MP11|ColumnScopedMerge|column-scoped' examples/timeseries/models/daily_events_enriched.sql`
  — empty (no surviving MERGE claim in the fixture).
- `rg -n 'worth a follow-up correction' docs/TODO.md` — empty.
- `cargo test -p smelt-cli --test example_diagnostics` — the fixture still parses clean.
- `bash .claude/scripts/verify-phase.sh` — green.

## Commit message

`docs(programme-hygiene): correct the enrichment fixture's technique claim to the derived DeleteInsert`

# Phase 7 plan — close the residue: stale spec text, four validate passes, all gates

## Objective

Phases 1–6 each deleted the Known Divergence bullet it closed inline, so this phase is the
sweep for what those deletions *left behind*: spec text elsewhere that still presents a retired
or now-decided fact as live. Then validate the four spec anchors and run the full gate set.
Advances success criteria 8 and 9 (and finishes 5, whose retirement left stale rows in
`models.md`).

## Spec delta

No behaviour changes. Spec edits only, and each one deletes or corrects a now-false claim:

1. `docs/specs/models.md` §"Facts and their fill modes" table (the `*model-only*` row, ~line
   173) and §Design "Declared" paragraph (~line 310) both still list `data_latency` among the
   live per-column declared facts. Phase 5 retired the key (the frontmatter table row ~line 199
   already says **Retired**). Remove `data_latency` from both places so the spec does not
   describe a key that is now a hard error.
2. `docs/specs/sources.md` §Known Divergences, the `mutation_profile` bullet: its closing clause
   lists `lateness` among "the other sub-facts" whose per-cell admission is "still unbuilt".
   Lateness is now decided never to be a plan input at all (`sources.md` §Semantics trust rule;
   `model_properties.md` §Constraints "Declared lateness is orchestration-only"), so listing it
   as awaiting admission contradicts this spec's own semantics. Drop `lateness` from that list
   and say why in half a sentence. Same bullet: note that `key_recurrence`'s declared value is
   now *checked* against the derived bound (`KeyedRecurrenceDeclarationMismatch`), which phase 4
   added after this bullet was last written.
3. Any further stale claim the sweep below turns up in the four anchors, judged by the same
   rule: a sentence that a phase 1–6 change made false gets corrected; anything else is left
   alone and recorded in the summary as out of scope.
4. Bump `last_reviewed: 2026-09-06` on every spec file this phase edits.

## Tests

- `lateness_orchestration_only::specs_do_not_present_per_column_data_latency_as_live` (extend
  `crates/smelt-logical/tests/lateness_orchestration_only.rs`) — greps `docs/specs/*.md` and
  `docs-site/docs/**` for `data_latency` and asserts every surviving occurrence is on a line
  that also names the retirement (`Retired`/`retired`), so a future edit cannot quietly
  reintroduce the key as live surface. Red before the `models.md` edit, green after.
- `cargo test -p smelt-cli --test example_diagnostics` — unchanged expectations; re-run as the
  regression fence for the two `examples/broken/` fixtures phases 1 and 5 added.

## Tasks

1. Extend `lateness_orchestration_only.rs` with the doc-sweep test; confirm it fails on the two
   live `models.md` rows (red).
2. Edit `models.md` rows 173 and 310 per spec delta 1; re-run the test (green).
3. Edit the `sources.md` Known Divergences `mutation_profile` bullet per spec delta 2.
4. Run `/smelt:validate incremental_shapes`, then `model_properties`, `sources`, `diagnostics`.
   For each drift item, classify: (a) caused by phases 1–6 → fix here; (b) pre-existing and
   already recorded as a Known Divergence → leave, note in summary; (c) pre-existing and
   *unrecorded* → add a one-line Known Divergence bullet naming the gap (do not implement it).
5. Bump `last_reviewed` on every edited spec.
6. Re-grep the anchors for residue the validate passes may miss:
   `rg -n 'data_latency|effective window.*lateness|lateness.*widen' docs/specs docs-site/docs`.
7. Write `phases/07-summary.md` including, per spec, the validate verdict and the classification
   of every drift item; then judge success criteria 1–9 against the six summaries and state
   whether the outcome is complete.

## Verification

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN.
- `cargo test -p smelt-logical --test lateness_orchestration_only --test walk_coverage`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity`
- `cargo test -p smelt-cli --features duckdb --test maintenance_conformance` (full suite; the
  cadence-on flip phase 6 landed is the thing to keep green)
- `cargo test -p smelt-cli --test example_diagnostics`
- `cargo test -p smelt-core --test hardening_budget` — baseline unchanged or lowered.
- Four `/smelt:validate` runs clean of drift this outcome owns.

## Commit message

`docs(specs): close decision-residue spec residue and validate the four anchors`

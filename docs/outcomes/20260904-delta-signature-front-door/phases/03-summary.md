# Phase 3 summary — the guide opens on delta signatures

## Shipped

- `docs-site/docs/guide/incremental-models.md`: replaced `## How it works` with `## What a
  model emits` — names the delta concept, the three-point scale
  (`append-only within a window ⊑ keyed upsert ⊑ general`), addressing (window/key set/
  whole-table), a real `smelt explain user_daily_spend` headline, the "derived, never declared"
  rule, and forward pointers to Configuration, the composed shape, contract relaxations,
  Rebuilding, and Schema Evolution.
- Demoted the DELETE+INSERT mechanics to `### What a partition-shaped run does` under
  `## Running incremental models`, reworded as the technique the derived plan assigns to a
  partition-shaped model's `NewData` trigger, not a fixed strategy.
- New standing gate `crates/smelt-cli/tests/docs_front_door.rs` (3 tests): first-section
  content check, front-door headline byte-pin against real `smelt explain` output, and a
  `four corners` ratchet across all of `docs-site/docs/`.
- All 11 inbound anchors named in the plan preserved (verified via `rg` for
  `incremental-models.md#...` references).

## Decisions

- Chose `user_daily_spend` as the front-door example (headline `keyed upsert`, not `general`) —
  no timeseries example model emits `append-only within a window` at the top level, and a real
  non-degraded headline teaches the concept better than a `general` one. `daily_events_enriched`
  is referenced immediately after as the degraded contrast case (already pinned by
  `explain_docs_freshness.rs`, so no new pin collision).
- The four-corners ratchet test was already vacuously green (`rg` found zero hits under
  `docs-site/docs/` before this phase) — written anyway per the plan, to hold criterion 2's
  second half against regression rather than leaving it unenforced.
- Left the `## Incremental strategies` section (line ~784) and other DELETE+INSERT mentions in
  batching/self-referential/window-function sections alone — those describe the technique's
  mechanics in a specific already-correct context, not a "the strategy is fixed" claim.

## For the next planner

- Phase 4 (rename `backbuild-synthesis.md` to the rebuild verb) and phase 5 (validate/close-out)
  are unaffected by this phase's edits — no reshape needed.
- Not done here, out of scope per the outcome: extending `docs_front_door.rs`'s headline pin to
  cover more than one example model, or auditing the rest of the guide file line-by-line for
  every DELETE+INSERT mention (the plan scoped this to a front-door rewrite).

## Gates

- `cargo test -p smelt-cli --test docs_front_door` — 3/3 green.
- `cargo test -p smelt-cli --test explain_docs_freshness --test tutorial_freshness --test cli_docs_coverage` — green.
- `rg -in "four.corners" docs-site/docs/` — no output.
- Anchor check (`rg -no 'incremental-models\.md#[a-z0-9-]+' docs-site/docs/ docs/ | sed 's/.*#/#/' | sort -u`) — all 11 plan-listed anchors present.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN.

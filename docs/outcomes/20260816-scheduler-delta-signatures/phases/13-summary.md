# Phase 13 summary — validate + close out

## Shipped

- `docs/specs/incremental_models.md` §Known Divergences: the scheduler-currency bullet is
  rewritten headline-first ("delta signatures **are** the dispatch currency for key-addressed
  model edges, not yet for everything the scheduler runs") and narrowed to the three real
  residues — (a) `dispatch_widened` on an uncovered inbound input, (b) whole-table currency
  surviving below the dispatch seam in `propagate_with_keys` for a non-keyed-grain downstream,
  (c) `change_feed`/`UpstreamMutation` and per-cell `deferral`. Dropped the stale "only when a
  caller feeds them in as a seed" clause (phase 7/12 landed live resolution).
- `docs/specs/incremental_models.md` §Future Extensions: removed the "Automatic,
  watermark-diffed `--since-upstream`" bullet — it described exactly what phase 8 shipped; its
  real residue (raw sources with no `covered_intervals`) already lives in the Known Divergences
  "Automatic snapshot diffing" bullet, verified still accurate.
- `docs/specs/diagnostics.md`: the `MaintenanceRepairKeysNotDiscoverable` /
  `MaintenanceRepairSliceUnbounded` paragraph no longer claims the repair family "has no
  deriving proof, technique, or emitter" (false since `20260809-repair-family`); rewritten to
  say both codes have no `DiagnosticCode` variant but are surfaced pre-execution by
  `smelt explain` via `maintenance::ledger::render_refusal`. Fixed the dangling
  `§Known Divergences "The contract, plan, and graph layer"` link (heading no longer exists) to
  point at `§"The repair family"`.
- `crates/smelt-logical/tests/refusal_catalogue_sync.rs` (new): `every_render_refusal_code_has_a_catalogue_row`
  exhaustively instantiates every `Refusal` variant (a `match` with no `_` arm makes a missed
  variant a compile error) and asserts each `render_refusal` code has a catalogue row;
  `repair_family_divergence_note_is_not_stale` pins the paragraph's wording so it cannot regress.
- `docs-site/docs/reference/cli.md` §"Forward propagation with `--since-upstream`": reworded the
  opening sentence off "caller-declared" per-source deltas (stale since phase 6/7/8) to name the
  watermark/observed-delta/live-seed sources; added a paragraph on the dispatch-widen downgrade
  (`dispatch_widened`) that phases 2–4 shipped but the CLI page never documented.

## Decisions

- Kept the scheduler-currency bullet rather than deleting it — residue (b) (whole-table
  currency below the dispatch seam) is real per phase 12's own decision record, so deleting
  would overclaim. Matches the phase plan's explicit instruction.
- Treated the diagnostics-catalogue residue as a truth edit, not a new `DiagnosticCode`
  variant — adding variants for the three refusals is feature work outside this outcome's
  criteria (per the phase-13 planning note).
- Did not touch `docs/specs/run_state.md` or `docs/specs/incremental_shapes.md` — grepped both
  for the divergence bullets the plan called out and found nothing stale (only the
  `--since-upstream` bullet at `incremental_models.md` line ~2049 needed checking, and it was
  already correctly narrowed by phase 8).

## For the next planner

- Nothing outstanding from this phase. The outcome's success criteria are all met: criterion 6
  (spec truth) closes with this phase's edits; criterion 7 (gates) is green end to end,
  including `maintenance_conformance`'s scheduler-driven keyed→partition recipes from phase 12.
- Out-of-scope items already recorded in the outcome (`derive_affected_keys` KeyScope
  over-projection, live change-feed folds, per-cell deferral scheduling) remain exactly where
  the outcome placed them — this phase did not touch them.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, full `cargo test` workspace,
  `example_diagnostics`). Required `DUCKDB_LIB_DIR=/home/andrew/.local/lib/duckdb` (not
  `/usr/local/lib`, which has no `libduckdb.so` in this environment) and a >2-minute timeout.
- `cargo test -p smelt-cli --test maintenance_conformance` — 79 passed.
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` — 4 + 23 passed.
- `cargo test -p smelt-logical --test walk_coverage --test refusal_catalogue_sync` — 2 + 4 passed.
- `cargo test -p smelt-core --test hardening_budget` — 3 passed (the printed "REGRESSION" line
  is the gate's own self-test probe asserting it *would* catch a regression, not a real one).
- `cargo test -p smelt-types --test unknown_census` — 4 passed.
- `cargo test -p smelt-db --test integration diagnostics_catalogue` — 1 passed (362 filtered,
  as expected — `diagnostics_catalogue` is one test module inside the shared `integration`
  binary).
- `cargo test -p smelt-cli --test since_upstream --test explain_maintenance` — 29 + 13 passed.
- Drift check: `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md docs/specs/run_state.md
  docs/specs/incremental_shapes.md docs/specs/diagnostics.md docs-site/docs/reference/cli.md`
  — only the timeless-oracle banner sentence in `diagnostics.md` matches (expected).

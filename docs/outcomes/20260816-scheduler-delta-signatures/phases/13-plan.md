# Phase 13 plan — validate + close out

## Objective

Restore spec truth for everything phases 2–12 landed (success criterion 6), close the
diagnostics-catalogue residue phase 10 flagged, bring the docs-site prose in line with the
now-live delta consumption and dispatch behaviour, and sweep every standing gate so the
outcome can be judged complete (criterion 7). No new runtime behaviour lands in this phase —
it is a truth-and-verification phase.

## Spec delta

The phase is mostly spec/doc edits; they come first, before the gate sweep.

1. `docs/specs/incremental_models.md` §Known Divergences, the **"The scheduler does not yet
   consume delta signatures end to end"** bullet — narrow to the residue that actually
   remains. Stale clauses to remove, each with phase evidence:
   - "but only when a caller feeds them in as a seed" — phase 7 landed plan-time live keyed-seed
     resolution off the group-grain sidecar; phase 12 proved it end to end through the real
     `--since-upstream` path (`keyed_partition_scheduler_sweep_from_model_upstream_matches_oracle`
     asserts the resolved restriction names exactly the touched ids).
   - Re-word the headline claim: signatures now *are* the dispatch currency for key-addressed
     model edges (derived, composed, dispatched, restriction-carrying, live-seeded). State the
     real residue: (a) an inbound input that is not itself key-addressed widens to the ordinary
     route with a reported `dispatch_widened` downgrade; (b) a non-keyed-grain downstream of an
     admitted keyed edge still widens to whole-table in `propagate_with_keys` once its upstream
     is visited (phase 12 decision), so interval currency survives below the dispatch seam;
     (c) `change_feed`/`UpstreamMutation` folds and per-cell `deferral` scheduling remain, each
     already tracked by its own bullet/outcome.
   - Keep the tracking links; drop this outcome's link only if the bullet's residue is fully
     owned elsewhere (it is not — keep it, pointing at the residue outcomes).
2. `docs/specs/incremental_models.md` — verify (do not blindly edit) that the
   `--since-upstream` explicit-delta bullet is already narrowed to automatic snapshot diffing
   only (phase 8) and that the `smelt explain` headline / guarantee-ledger bullets are gone
   (phases 9–10). Fix any residue found; record what was checked in the summary.
3. `docs/specs/diagnostics.md` §Known Divergences — the paragraph listing
   `MaintenanceReachNotDerivable` / `MaintenanceRepairKeysNotDiscoverable` /
   `MaintenanceRepairSliceUnbounded` is stale: it claims the repair family "has no deriving
   proof, technique, or emitter" (it has all three since `20260809-repair-family`). Rewrite to
   the truth: these three codes have no `DiagnosticCode` enum variant, so they never reach
   `file_diagnostics()`; they *are* surfaced pre-execution by `smelt explain`'s refusal block
   via `maintenance::ledger::render_refusal`, which names them by exactly these catalogue
   strings, and a future `DiagnosticCode` addition must reuse the same string.
4. `docs-site/docs/reference/cli.md` §"Forward propagation with `--since-upstream`" — the
   phrase "walks it forward from **caller-declared** per-source deltas" predates phases 6–8.
   Reword: deltas come from the paired `--landed`, from a persisted watermark, and are refined
   live from the recorded observed-delta table; keyed dirt additionally resolves affected key
   *values* live from the group-grain sidecar. Add one short paragraph on the dispatch
   downgrade: a downstream whose inbound inputs are not all key-addressed runs the ordinary
   route and the run report says so (`dispatch_widened`) — never a silently dropped component.

## Tests

1. `crates/smelt-logical/tests/refusal_catalogue_sync.rs::every_render_refusal_code_has_a_catalogue_row`
   — for each `Refusal` variant, `render_refusal`'s `code` appears as a row in
   `docs/specs/diagnostics.md` (drift net; expected green on arrival, must stay green).
2. `crates/smelt-logical/tests/refusal_catalogue_sync.rs::repair_family_divergence_note_is_not_stale`
   — the `diagnostics.md` paragraph mentioning `MaintenanceRepairKeysNotDiscoverable` no longer
   claims the repair family has no deriving proof/technique/emitter, and does mention
   `smelt explain` as the surfacing path. Red until spec delta 3 lands.
3. `docs/specs/incremental_models.md` scheduler bullet: no automated test — verified by the
   drift-check task below and quoted in the summary.

## Tasks

1. Land spec delta 3 red-green against test 2; add test 1 alongside it.
2. Land spec delta 1 — rewrite the scheduler-currency bullet against phase 2/4/6/7/8/12 summary
   evidence; quote in the phase summary which clause each phase closed.
3. Do spec delta 2's audit pass: re-read every `incremental_models.md` Known Divergences bullet
   this outcome touched and confirm it matches shipped behaviour; fix residue found.
4. Land spec delta 4 (docs-site `cli.md`); regenerate any golden fixture the edit disturbs
   (`explain_show_sql_daily_events_golden.txt`, tutorial pages via
   `python3 examples/web_analytics/generate_tutorial.py`) only if a gate demands it.
5. Drift check: `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md docs/specs/run_state.md
   docs/specs/incremental_shapes.md docs/specs/diagnostics.md docs-site/docs/reference/cli.md`
   must show no matches outside a timeless-oracle banner (`/smelt:validate incremental_models`'s
   own drift signal).
6. Run the full standing-gate sweep (below); record each gate's result in the summary.

## Verification

- `bash .claude/scripts/verify-phase.sh` — must be ALL GREEN.
- `cargo test -p smelt-cli --test maintenance_conformance --quiet`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity --quiet`
- `cargo test -p smelt-logical --test walk_coverage --test refusal_catalogue_sync --quiet`
- `cargo test -p smelt-core --test hardening_budget --quiet`
- `cargo test -p smelt-types --test unknown_census --quiet`
- `cargo test -p smelt-db --test integration diagnostics_catalogue --quiet`
- `cargo test -p smelt-cli --test since_upstream --test explain_maintenance --quiet`

## Commit message

`docs(scheduler-delta-signatures): close out — narrow divergence bullets, sync refusal catalogue, sweep gates`

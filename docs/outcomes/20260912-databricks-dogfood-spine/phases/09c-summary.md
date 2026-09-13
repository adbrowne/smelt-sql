# Phase 9c summary — criterion 8 closed, Databricks succession fold root-caused and fixed offline

## Shipped

- `crates/smelt-cli/tests/github_activity_dbx_oracle.rs`: `EQUIVALENCE_DIVERGENCE_REGISTRY` now
  carries the `gold_events_enriched`/`UnorderedColumnDivergence` entry (mirroring
  `github_activity_dual_target.rs`'s own registered entry for the same relation), plus tests
  `the_committed_equivalence_report_shows_no_violation`, `equivalence_registry_entries_are_all_live`
  and a `divergent_relations` helper. 20/20 on this suite (was 17).
- **Root cause, confirmed offline (no workspace):**
  `crates/smelt-runtime/src/maintenance_driver/succession/mod.rs::resolve_live_succession_cell`
  treated any state-downgraded cell (technique no longer `Technique::SuccessionPatch`) as "not
  live" and returned `Ok(None)`, so a succession-grain model on a target with no realisable
  `TombstoneLedger` (Spark/Databricks: `realisable_state_structures` returns nothing) fell through
  to the generic `DeleteInsert` driver — which has no `(key, clock)` fold at all. That reproduces
  row 8's exact symptom: Databricks' `silver_actor_naming` row count equals the raw source count,
  with individual `(actor_id, created_at)` pairs duplicated up to 7x.
- **Fix, single-owned in the succession driver:** `SuccessionCell` gained a `state_downgraded: bool`
  field; `resolve_live_succession_cell` now stays live for a downgraded cell (recognised via
  `PlanCell::state_downgrade.original == Technique::SuccessionPatch`); the dispatch site
  (`crates/smelt-runtime/src/execute/project/mod.rs`) now forces the full-rebuild route
  (`rebuild_succession_state`, which folds via `emit_succession_full_rebuild`'s
  `ROW_NUMBER() ... = 1`) whenever `cell.state_downgraded`, in addition to the existing
  `request.full_refresh`/`force_full_refresh`/`request.rebuild` triggers — never the window-forward
  patch loop, which needs a live tombstone ledger for cross-run correctness.
- New offline differential test:
  `crates/smelt-cli/tests/explain_maintenance/databricks_succession_differential.rs` —
  `silver.actor_naming`'s cell resolves `Technique::SuccessionPatch` on `dev` (DuckDB) and
  downgrades to `Technique::DeleteInsert` (with a recorded `state_downgrade`) on the Databricks
  dialect (`SqlDialect::SparkSQL`), computed directly against the real `examples/github_activity`
  project with no CLI flag and no connection. `crates/smelt-cli/tests/explain_maintenance/support.rs`
  gained `example_dir`/`plan_result_for` helpers for this.
- Resolver-level regression test:
  `state_downgraded_cell_still_dispatches_marked_for_full_rebuild` (renamed from
  `state_downgraded_cell_is_not_dispatched`, whose assertion inverted) in
  `crates/smelt-runtime/src/maintenance_driver/succession/tests.rs`.
- `.claude/large-file-baseline.txt` updated (my fix grew `execute/project/mod.rs` by 4 lines;
  the update also swept in unrelated drift on `github_activity_bq_oracle.rs`/
  `github_activity_dual_target.rs` from earlier phases that had never been captured).

## Decisions

- **`smelt explain` has no `--target` flag today** — `ExplainArgs` (`crates/smelt-cli/src/main.rs`)
  carries no such option; the plan's literal `smelt explain --target dev`/`--target databricks`
  command does not exist. The differential is instead computed directly against the library
  functions `smelt explain` itself calls (`maintenance_plan_report` + `resolve_availability`),
  which is a strictly more precise experiment than shelling out would have been anyway — no new CLI
  surface was added since it was not needed to reach the answer.
- The premise the plan's hypothesis stated first ("if [dev and databricks] show the *same*
  technique, the defect is in the model's own SQL... if not, [it's the write path]") predicted
  the interesting branch would be "same technique". The measured result was the *other* branch —
  the technique **differs** (downgraded on Databricks) — which is actually the more direct and
  cheaper explanation than either branch the hypothesis posed; no live Spark reproduction (the
  hypothesis's fallback step) was needed.
- Chose route 3 from `## Blocked` ("fix the write path"), justified as small: the change is
  confined to one struct field, one boolean check, and one added disjunct in an existing
  condition — no new emitter, no printer change, no backend statement authoring.

## For the next planner

- **9d must re-run with a refreshed oracle sweep, not just the parity snapshot** — task 7 of this
  plan said "parity only, or parity plus a refreshed oracle sweep if a maintenance statement
  changed." No maintenance *statement* changed (the fix is a dispatch-routing decision — which
  existing, already-tested emitter runs, not a new one), but the *execution shape* of every
  incremental window for `silver.actor_naming` on Databricks changed: every run now does a full
  rebuild rather than a window-forward patch. 9d should re-run both the dual-target parity sweep
  and the equivalence-oracle sweep (all three windows) to get fresh, correct numbers under the fix
  — the currently-committed `08-parity.json` and `09b-equivalence.json` were measured against the
  pre-fix (buggy) behaviour and should be treated as stale for `silver_actor_naming` once 9d lands.
- **Cost/performance is now a live consideration for this model**: every incremental run of
  `silver.actor_naming` (and any other succession-grain model that lands on a target with no
  realisable `TombstoneLedger`) re-derives the whole presented table from the whole source, not
  just the window. For the fixture's scale this is invisible; on the follow-on
  `databricks-correctness` outcome (or a future scale test) this is worth flagging as a real
  cost tradeoff of the current architecture's "no TombstoneLedger on Delta" premise — the same
  premise the 2026-09-13 Catalog Commits research note (in this outcome's decision log) questions.
  Not investigated further here — out of this phase's offline, no-live-run scope.
- **The `## Blocked` entry's earlier candidate route 1** ("read `succession/execute.rs`'s
  MERGE/patch statement... possibly the same root cause as the Catalog Commits note") turned out
  to be a different mechanism than guessed — not a MERGE atomicity problem at all, but a dispatch
  gate reading a post-downgrade technique. The Catalog Commits/cross-table-transaction question
  from the 2026-09-13 research note is therefore **not** resolved or touched by this phase and
  remains open triage for a future planner.
- Nothing left the outcome; nothing added to `## Out of scope`.

## Gates

- `cargo test -p smelt-cli --test github_activity_dbx_oracle` — 20/20, registry non-empty.
- `cargo test -p smelt-cli --test github_activity_dual_target` — 23/23, unchanged.
- `cargo test -p smelt-runtime --lib succession` — 14/14 (including the renamed/updated test).
- `cargo test -p smelt-cli --test explain_maintenance` — 56/56 (including the new differential).
- `cargo test -p smelt-runtime --test statement_parity` — 41/41.
- `cargo test -p smelt-logical --test walk_coverage` — 14/14.
- `cargo test -p smelt-cli --test maintenance_conformance` — 104/104.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full `cargo test`, `example_diagnostics`).

# Phase 1 summary — delta-signature headline

**Shipped:**
- `smelt_db::model_output_delta_for(db, ws, file)` (`crates/smelt-db/src/lib.rs`) — a model's
  own derived `OutputDelta`, single-owned via a new `own_output_delta_shape` helper that
  `ref_model_edge` now also calls, so a downstream's edge view of a model and that model's own
  headline can never disagree (pinned by
  `crates/smelt-db/tests/typed_model_edge.rs::own_output_delta_matches_downstream_edge_view`).
- `DeltaSignatureHeadline` + `delta_signature_headline()` + `render_text()`
  (`crates/smelt-cli/src/explain.rs`) — renders all four shapes (`keyed_upsert` bare/composed,
  `append_only_window`, `general`, and the `None` no-shape case as a degraded `general`).
- `build_maintenance_plan_report` prepends `model <name>  (emits: …; grain: …)` as the report's
  first line; `build_maintenance_plan_json`/`ExplainMaintenanceJson` carry the same struct as a
  top-level `delta_signature` field — one struct renders both surfaces.
- 7 new tests in `crates/smelt-cli/tests/explain_maintenance.rs` covering all four shapes, the
  grain-label cross-check, `--json`/text agreement, and doc-sync.
- Spec edits: `incremental_models.md` §Surface "CLI" Headline bullet (general-verdict rendering
  added, first-line + `--json` parity stated) and §Known Divergences (headline clause deleted,
  narrowed to the per-column-ledger/run-shape gap); `docs/specs/cli.md` new "Delta-signature
  headline" paragraph; `docs-site/docs/reference/cli.md` explain section documents both surfaces.

**Decisions:**
- `None` (no derivable shape at all) renders as `general (degraded by: "no derivable
  output-delta shape"), not delta-addressable` rather than a third code path — keeps the
  render function total over `Option<&OutputDelta>` without a silent fallback.
- Golden fixture `explain_show_sql_daily_events_golden.txt` regenerated (2-line diff: the new
  headline only) — legitimate output change, not drift.
- `docs-site/docs/examples/web-analytics/deduplication.md` regenerated via
  `generate_tutorial.py` (2-line diff) to keep `tutorial_freshness` green — a single-file fix
  scoped to unblocking this phase's own gate, not the broader tutorial-page sweep phase 2 owns.

**For the next planner:**
- Phase 2 ("confirm every explain excerpt carries the headline") should re-run
  `generate_tutorial.py` project-wide (only `deduplication.md` needed it this phase, since it's
  the only committed excerpt whose model has a maintenance plan) and check the freshness gate
  stays green.
- Phase 3's "purge four-corners text" and phase 4's backbuild-synthesis rename are untouched —
  not this phase's scope.
- No new gaps surfaced beyond what phase 2–5 already list.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace test suite, example_diagnostics).
- `cargo test -p smelt-db --test typed_model_edge` — 7/7 pass.
- `cargo test -p smelt-cli --test explain_maintenance --test explain_model --test
  explain_show_sql --test cli_docs_coverage` — 35/35, 27/27, 9/9, 3/3 pass.

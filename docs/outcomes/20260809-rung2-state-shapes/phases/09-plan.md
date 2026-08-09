# Phase 9 plan — `smelt explain` renders hidden state as internal state

## Objective

Close success criterion 4's second half: the hidden `__`-suffixed state columns rows 5–7
made real are currently invisible everywhere except the physical table — `smelt explain`
reports a plan that never mentions them, so a modeller cannot see what their model
actually stores. This phase adds an internal-state section to the plan report (text and
`--json`), sourced from the one owner of state derivation, and finishes the residual
surface text.

## Spec delta (spec-first; the implement step makes these edits)

1. `docs/specs/incremental_models.md` §Surface "CLI" — extend the `smelt explain <model>`
   bullet: the report additionally lists, per presented column that carries decomposed
   state, its hidden state columns and the presentation map `π` that recomputes the
   presented value from them, labelled as internal state and explicitly *not* part of the
   model's public schema. §"Decomposed state (rung 2) in keyed models"'s existing
   one-liner ("`smelt explain` renders state columns as internal state … surface detailed
   alongside the CLI's other plan output") then points at real text.
2. `docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan report" — same section
   described for the text report; §"`smelt explain --json` output schema" — new
   `state_columns` array (append-stable field addition, §Constraints item 5), each entry
   `{"presented_column": …, "state_columns": [...], "presentation_expr": …}`.
3. `docs-site/docs/reference/smelt-explain.md` — a short "Internal state columns"
   subsection with a rendered example; `docs-site/docs/reference/cumulative-aggregate.md`
   — one cross-link from the decomposed-fold / order-monotone / once-write paragraphs
   ("`smelt explain <model>` shows the hidden state a model stores").

No behaviour outside the report changes; no obligation text needs deleting (rows 5 and 7
already removed theirs — verified: the surviving "no companion projection is required"
sentences are the *corrected* statements, and the one remaining once-write Known
Divergence describes a real residual limitation, not a rung-2 gap).

## Tests

Red-green, in this order:

1. `smelt-logical` `rules::cumulative` — `state_summary_reports_hidden_columns_for_avg`:
   the new pure summarizer over a classification whose column carries `AVG` state returns
   one entry naming `<col>__sum`, `<col>__count` and the `π` expression.
2. `smelt-logical` — `state_summary_is_empty_for_stateless_columns`: a `SUM`/`MAX`-only
   classification produces no entries (the section must not appear for rung-1 models).
3. `smelt-logical` — `state_summary_covers_order_monotone_and_once_write`: `MAX_BY`'s
   `(v, o)` and a fallback-bearing once-write's `(value, written)` both report.
4. `smelt-db` (`tests/maintenance*.rs`, alongside the existing plan-report tests) —
   `maintenance_plan_report_carries_state_columns`: a keyed `AVG` model's
   `MaintenancePlanResult.state_columns` is populated; a keyed `SUM` model's is empty.
5. `smelt-cli/tests/explain_maintenance.rs` — `explain_renders_internal_state_section`:
   the text report for a keyed `AVG` model contains the state section naming both state
   columns and says they are not in the model's public schema.
6. `smelt-cli/tests/explain_maintenance.rs` — `explain_omits_state_section_when_no_state`:
   a keyed `SUM` model's report has no state section (no empty header).
7. `smelt-cli/tests/explain_maintenance.rs` — `explain_json_reports_state_columns`:
   `--json` carries the `state_columns` array with the same content as the text section.

## Tasks

1. Make the spec + `cli.md` edits above (spec-first), then the two docs-site edits.
2. Add pure `state_column_summary(&CumulativeClassification) -> Vec<StateColumnSummary>`
   to `crates/smelt-logical/src/rules/cumulative.rs` (re-exported from the crate root):
   one entry per `AggregatorColumn` whose `state` is `Some`, carrying
   `presented_column`, the `StateColumn` names, and `presentation_expr`. It reads the
   already-derived `AggregatorColumn::state` — it must not re-decide which spellings are
   state-bearing (single owner: `classify_cumulative` / `decompose_to_state`).
3. Add `state_columns: Vec<StateColumnSummary>` to `MaintenancePlanResult`
   (`crates/smelt-db/src/queries/maintenance.rs`). Populate it in the Salsa wrapper
   `smelt_db::maintenance_plan_report` (`crates/smelt-db/src/lib.rs`), which already has
   the model SQL, `refs`, and the resolved `SourceInfo`s: build the
   `SourceTimeseriesMap`/declared-FD inputs there, call `classify_cumulative`, and pass
   its result to the task-2 summarizer. A classifier `Err` (non-keyed or unadmitted
   model) yields an empty vector, never a panic and never a partial guess — the wrapper
   stays an input-builder per the Salsa purity rule.
4. Render the section in `build_maintenance_plan_report`
   (`crates/smelt-cli/src/explain.rs`), after the cells block, from the new field only —
   the CLI derives nothing. Omit the header entirely when the vector is empty.
5. Add `state_columns` to `build_maintenance_plan_json`'s output struct (new field only;
   nothing renamed or removed).
6. Confirm no other consumer of `MaintenancePlanResult` (`smelt-ui/src/build.rs`,
   `smelt-cli/src/bakeoff.rs`) needs updating beyond construction sites.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test walk_coverage` (no new whole-text scans introduced)
- `cargo test -p smelt-cli --test explain_maintenance --test explain_show_sql`
- `cargo test -p smelt-cli --test maintenance_conformance 2>&1 | tail -20` (53/53 unchanged)

## Commit message

`feat(explain): render decomposed-state columns as internal state in the maintenance plan report`

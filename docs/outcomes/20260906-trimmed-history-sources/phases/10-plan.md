# Phase 10 — Explain and docs: bound vs. required reach

## Objective

Surface the retention proof the previous phases derived. `smelt explain <model>` renders each
declared-`retention:` source's retained bound against the model's required reach — in text and in
`--json` — and the docs-site documents the declaration, the refusal and the degradation. Advances
success criterion 7 (and closes the user-visible half of criteria 2 and 4: a refusal and a
downgrade a user cannot see are not "never silent" in any useful sense).

## Spec delta

`docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan report" — add a **Retention reach.**
paragraph immediately after **State downgrade.**, in the same shape as its neighbours:

- Text: a `Retention:` section, one row per source carrying a bounded proof or a recorded
  downgrade, omitted entirely for a model referencing no `retention:` source. Rows, in the
  `retention_reaches`-then-`retention_downgrades` order the plan already fixes (both sorted by
  source):
  - `<source>: retained <n>s, required reach <n>s — within bound`
  - `<source>: retained <n>s, required reach <n>s — exceeds bound (SourceRetentionExceeded)`
  - `<source>: retained <n>s, reach unprovable — downgraded (SourceRetentionDowngraded): <reason>`
  Seconds, matching the units `refusal_diag.rs` already renders for `SourceRetentionExceeded`; no
  new interval formatter.
- `--json`: an append-stable top-level `retention` array (§Constraints item 5), omitted when empty,
  never `null`. Entries: `{"source": "...", "verdict": "within"|"exceeds"|"unprovable",
  "retained_secs": n, "required_lookback_secs": n, "reason": "..."}`, with
  `required_lookback_secs` present only for the bounded verdicts and `reason` only for
  `unprovable`.
- State the read-not-derive rule explicitly: both renderings read `MaintenancePlan::
  retention_reaches`/`retention_downgrades` verbatim; `smelt explain` derives no retention verdict
  of its own (maintenance-plan purity).

`docs/specs/diagnostics.md` — no code changes; both codes already exist. Only the docs-site
mirror below is new.

## Tests

New file `crates/smelt-cli/tests/explain_model/retention.rs` (registered in `main.rs`), using
`support::build_report_for`:

- `explain_text_reports_the_bound_and_the_required_reach` — a `retention:`+`timeseries:` source
  under an admissible model renders the `within bound` row with both quantities.
- `explain_text_reports_an_exceeding_reach_as_refused` — a reach past the bound renders the
  `exceeds bound (SourceRetentionExceeded)` row.
- `explain_text_reports_a_recorded_downgrade` — an unprovable reach renders the
  `SourceRetentionDowngraded` row naming source, retained bound and reason.
- `explain_text_omits_the_retention_section_without_a_retained_source` — no `Retention:` line at
  all for a model over undeclared-retention sources (no empty-section noise).

In `crates/smelt-cli/tests/explain_model/json_output.rs`:

- `explain_json_carries_a_retention_entry_per_bounded_source` — verdict, `retained_secs` and
  `required_lookback_secs` match the text row's quantities for the same project.
- `explain_json_omits_retention_when_no_source_declares_a_bound` — the key is absent, not `null`,
  not `[]`.

In `crates/smelt-cli/tests/explain_maintenance/docs_and_technique.rs` (mirroring
`docs_site_diagnostics_reference_lists_every_succession_code`, two-sided):

- `docs_site_diagnostics_reference_lists_every_source_retention_code` — every `SourceRetention*`
  code named in `docs/specs/diagnostics.md` appears in `docs-site/docs/reference/diagnostics.md`
  and vice versa.

## Tasks

1. Write the `docs/specs/cli.md` spec delta above (spec-first).
2. Red: add the four text tests, the two JSON tests and the doc-sync test; confirm they fail.
3. Text: in `build_maintenance_plan_report`, render the `Retention:` section from
   `result.plan.retention_reaches` / `result.plan.retention_downgrades` — no signature change,
   the whole `MaintenancePlanResult` is already in hand. Place it after the state-downgrade
   rendering so the report order matches the spec's paragraph order.
4. JSON: add `ExplainRetentionJson` and the `retention: Vec<ExplainRetentionJson>` field
   (`#[serde(skip_serializing_if = "Vec::is_empty")]`) to `ExplainMaintenanceJson`; take the two
   plan slices as new parameters on `build_maintenance_plan_json`; update its one call site in
   `crates/smelt-cli/src/commands/explain.rs` to pass `&result.plan.…`.
5. Docs-site — `docs/guide/sources.md`: a "Bounded history (`retention:`)" section covering the
   declaration (rolling, declared-never-observed, requires `timeseries:`), the plan-time and
   whole-table-recompute refusals, and the recorded downgrade, with a short `smelt explain`
   excerpt. Any fenced block containing `Maintenance plan: ` must lead with the
   `model <name>  (emits: …)` headline or `explain_docs_freshness` fails.
6. Docs-site — `docs/reference/sources-yml.md`: the `retention:` field row (well-formedness rules
   and the `MalformedSource` refusals for a bad interval, zero interval, or no `timeseries:`).
7. Docs-site — `docs/reference/smelt-explain.md`: the `Retention:` text rows and the `--json`
   `retention` array, matching the spec delta verbatim in shape.
8. Docs-site — `docs/reference/diagnostics.md`: `SourceRetentionExceeded` and
   `SourceRetentionDowngraded` entries, satisfying the new two-sided doc-sync test.
9. Fix the now-stale comment in `examples/github_activity/models/sources/raw/github_events.yml`
   claiming `retention:` is "unconsumed by any maintenance logic today".
10. Green: run the gates below; if `explain.rs` crosses its `.claude/large-file-baseline.txt`
    entry (2578), extract the retention rendering into `crates/smelt-cli/src/explain/retention.rs`
    rather than updating the baseline.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test explain_model --test explain_maintenance --test explain_docs_freshness --test cli_docs_coverage --test docs_front_door`
- `cargo test -p smelt-cli --test example_diagnostics`
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`feat(explain): render retained bound versus required reach in text and --json`

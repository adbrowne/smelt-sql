# Phase 9 plan — Fixture and docs for the succession grain

## Objective

Give the succession grain a real, diagnostics-clean example workspace and its user-facing
documentation: `examples/scd2_succession/` carrying `customer_changes` (arrival-partitioned,
`append_only`, `is_deleted NOT NULL`) and `customer_history` (row-local projection +
`LEAD`/`LAG` + `QUALIFY NOT is_deleted`), a docs-site guide page for the shape, and the twelve
succession codes on the diagnostics reference. Advances success criterion 9 in full; leaves
only criterion 10 (spec closure) to phase 10.

## Spec delta

None — this phase renders already-specified surface. One non-normative housekeeping edit is in
scope: `docs/specs/incremental_shapes.md` §References §"The succession grain" gains a **User
docs** bullet pointing at the new guide page (the same shape other specs' References use).

## Tests

1. `crates/smelt-cli/tests/example_diagnostics/smoke_and_migration.rs::scd2_succession_no_diagnostics`
   — `check_workspace_no_diagnostics("examples/scd2_succession")` (Salsa/CLI leg of criterion 9).
2. `crates/smelt-lsp/tests/example_workspaces.rs::scd2_succession_workspace_clean`
   — `assert_example_workspace_clean("scd2_succession")` (real-LSP leg; catches the
   asymmetric-discovery bugs the Salsa-direct test misses).
3. `crates/smelt-cli/tests/explain_maintenance/succession.rs::example_workspace_customer_history_is_a_succession_cell`
   — builds the maintenance-plan report for `examples/scd2_succession`'s `customer_history` via
   the existing `build_report_for` helper and asserts `grain: succession`,
   `identity: (customer_id, effective_ts)`, `technique: succession-patch`, and the
   `internal state:` tombstone-ledger line. Without this the fixture could be diagnostics-clean
   yet silently *not* recognised as succession.
4. `crates/smelt-cli/tests/explain_maintenance/docs_and_technique.rs::docs_site_diagnostics_reference_lists_every_succession_code`
   — parses the `Succession*` code names out of `docs/specs/diagnostics.md` §"Succession grain"
   (derived, not restated) and asserts every one appears in
   `docs-site/docs/reference/diagnostics.md`; two-sided (a docs-site name that is not a spec
   code also fails).
5. `crates/smelt-cli/tests/explain_maintenance/docs_and_technique.rs::succession_guide_page_is_navigated_and_covers_the_grain`
   — asserts `docs-site/mkdocs.yml`'s nav lists the new page and that the page names the
   admitted SQL outline, `QUALIFY NOT <flag>`, both partitioning postures (arrival vs
   event-time), and the tombstone ledger.

## Tasks

1. Create `examples/scd2_succession/`: `smelt.yml` (duckdb dev target, `paths: [models]`),
   `models/sources/customer_changes.yml` (`mutation_profile: append_only`, `timeseries:`
   `event_time_column: effective_ts` / `partition_column: ingested_date` / `granularity: day`,
   columns `customer_id INTEGER nullable: false`, `effective_ts TIMESTAMP nullable: false`,
   `tier VARCHAR`, `region VARCHAR`, `is_deleted BOOLEAN nullable: false`,
   `ingested_date DATE nullable: false`), and `models/customer_history.sql` — `refresh:
   incremental` frontmatter only, projecting `customer_id, tier, region, effective_ts AS
   valid_from`, `LEAD(effective_ts) OVER (PARTITION BY customer_id ORDER BY effective_ts) AS
   valid_to`, `… IS NULL AS is_current`, `FROM smelt.sources.customer_changes`,
   `QUALIFY NOT is_deleted`. Add a short `README.md` for the workspace.
2. Add tests 1 and 2; run them red-first (the workspace must not exist yet for red, so write
   the tests before step 1 if working strictly red-green — otherwise assert the *succession*
   recognition test 3 red first, which is the substantive claim).
3. Add test 3 to the existing `explain_maintenance/succession.rs` module; fix the fixture until
   it is recognised (most likely failure: a nullability or `mutation_profile` shape the
   classifier refuses — the refusal diagnostic names which rule).
4. Write `docs-site/docs/guide/scd2-succession.md`: what the shape is and why it needs no
   declaration, the admitted SQL outline (§"Succession-grain admission"), the derived `(k, t)`
   identity, the delete filter, the optional pre-window clamp, arrival- vs event-time
   partitioning, the tombstone ledger as internal state, what `smelt explain` shows, and a
   table of the refusal codes with the fix for each. Timeless-oracle rule applies: no phase or
   plan vocabulary. Link `examples/scd2_succession/` as the worked example.
5. Add the nav entry to `docs-site/mkdocs.yml` directly after `Incremental Models`, and a
   cross-link from `docs-site/docs/guide/incremental-models.md`.
6. Add a `## Succession grain` section to `docs-site/docs/reference/diagnostics.md` listing all
   twelve codes (the eleven analysis-time codes plus the runtime `SuccessionClockTie`) with
   severity and one-line trigger, plus one worked `### Example:` in the page's existing style
   (`SuccessionDeleteFilterMisplaced` is the most instructive).
7. Add tests 4 and 5.
8. Add the `examples/scd2_succession/` row to `examples/README.md`'s directory table and the
   **User docs** bullet to `incremental_shapes.md` §References §"The succession grain".

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, full workspace test,
  `example_diagnostics`)
- `cargo test -p smelt-lsp --test example_workspaces --quiet 2>&1 | tail -20`
- `cargo test -p smelt-cli --test explain_maintenance --quiet 2>&1 | tail -20`
- `cargo test -p smelt-cli --test cli_docs_coverage --test docs_front_door --test
  explain_docs_freshness --quiet 2>&1 | tail -20`
- `cd docs-site && uv run mkdocs build 2>&1 | tail -20` (nav entry + no broken links)
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`docs(succession): example workspace and docs-site guide for the succession grain`

# Phase 2 plan — every explain excerpt in the docs-site carries the headline

## Objective

Advance success criterion 4 (and the docs half of criterion 1): re-derive the
web-analytics tutorial pages so the pipeline-generated excerpts are provably current, and
bring the two *hand-pasted* `smelt explain` report excerpts elsewhere in the docs-site
(`reference/cli.md`, `guide/incremental-models.md`) back in step with what the binary now
prints — headline first. A new standing gate keeps all three in step from here on, so a
future change to the headline's form fails a test instead of silently ageing the docs.

## Spec delta

None. The headline's form and both surfaces were specified in phase 1
(`docs/specs/incremental_models.md` §Surface "CLI", `docs/specs/cli.md` §"Delta-signature
headline"); this phase only makes the published excerpts match that spec.

## Interpretation of criterion 4 (see Decision log)

The tutorial doc-sync pipeline (`examples/web_analytics/generate_tutorial.py` +
`tutorial_pages/`) is scoped to `docs-site/docs/examples/web-analytics/`; extending it to
arbitrary reference/guide pages is a pipeline rewrite this outcome does not call for. The
criterion's intent — no stale hand-pasted explain output anywhere — is met instead by
regenerating the pipeline pages *and* gating the non-pipeline excerpts against real CLI
output from `examples/timeseries` (which defines both `daily_events` and
`daily_events_enriched`, the two models those excerpts show).

## Tests

New `crates/smelt-cli/tests/explain_docs_freshness.rs`:

1. `every_maintenance_plan_excerpt_leads_with_the_headline` — scan every fenced block under
   `docs-site/docs/**/*.md` containing a `Maintenance plan: <name>` line; assert the block's
   first non-`$`-prompt line is `model <name>  (emits: …; grain: …)`. Fails today on
   `reference/cli.md` and `guide/incremental-models.md`.
2. `cli_reference_sample_matches_real_explain_output` — run `env!("CARGO_BIN_EXE_smelt")
   explain daily_events --project-dir examples/timeseries` and assert the committed
   `reference/cli.md` sample block (minus the `$ …` prompt line) is byte-identical to stdout.
3. `incremental_guide_headline_matches_real_explain_output` — same run for
   `daily_events_enriched`; assert the committed block's headline line and
   `Maintenance plan:` line are byte-identical to the real output's first lines (the rest of
   that excerpt is deliberately elided with `...`, so only the un-elided prefix is compared).
4. `tutorial_freshness` (existing, `crates/smelt-cli/tests/tutorial_freshness.rs`) — must
   stay green after regeneration; no new test, just the gate this phase re-runs.

## Tasks

1. Red: add `explain_docs_freshness.rs` with the three tests above; confirm 1–3 fail.
2. Run `python3 examples/web_analytics/generate_tutorial.py` from `examples/web_analytics/`;
   `git diff docs-site/docs/examples/web-analytics/` — expect no diff (phase 1 already
   regenerated `deduplication.md`); commit any diff that does appear as a legitimate
   regeneration and say so in the summary.
3. Replace the `reference/cli.md` `$ smelt explain daily_events` block with the real current
   output of that command against `examples/timeseries` (keep the `$ …` prompt line).
4. Update the `guide/incremental-models.md` `$ smelt explain daily_events_enriched` excerpt's
   un-elided prefix from the same real run — headline line included; leave the `...`
   elisions and surrounding prose alone (phase 3 owns that page's prose).
5. Green: tests 1–3 pass; `cargo test -p smelt-cli --test tutorial_freshness` green.
6. Write `phases/02-summary.md` (shipped / decisions / for the next planner / gates).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test explain_docs_freshness --test tutorial_freshness --test explain_maintenance --test cli_docs_coverage`
- `git diff --stat docs-site/docs` — changes confined to the two excerpt blocks plus any
  legitimate tutorial regeneration.

## Commit message

`docs(explain): every docs-site explain excerpt leads with the delta-signature headline`

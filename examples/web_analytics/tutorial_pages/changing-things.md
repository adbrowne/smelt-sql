# Backfills, new columns, and late updates

<!-- PLACEHOLDER: intro — the three ways an existing pipeline changes. -->

## Backfilling a range

```bash
smelt backbuild silver.events_parsed --start 2026-04-01 --end 2026-04-19 --dry-run
```

<!-- smelt-generate: @cwd=tutorial_stages/03_late_data @render=skeleton backbuild silver.events_parsed --start 2026-04-01 --end 2026-04-19 --dry-run -->

??? example "Full dry-run transcript — `smelt backbuild silver.events_parsed --start 2026-04-01 --end 2026-04-19 --dry-run`"

    <!-- smelt-generate: @cwd=tutorial_stages/03_late_data backbuild silver.events_parsed --start 2026-04-01 --end 2026-04-19 --dry-run -->

<!-- PLACEHOLDER: chunking walkthrough. -->

## Adding a column

<!-- PLACEHOLDER: the is_purchase change (stage 04). -->

```bash
smelt diff --select silver.events_parsed
```

<!-- smelt-generate: @cwd=tutorial_stages/04_add_column @fixture-schemas @render=text @expect-exit=1 diff --select silver.events_parsed -->

<!-- PLACEHOLDER: ALTER + NULL history, backfill:, NOT NULL caveat. -->

## When upstream data changes

```bash
smelt run --since-upstream \
  --source silver.events_parsed --landed 2026-04-10..2026-04-11 --dry-run
```

<!-- smelt-generate: @cwd=tutorial_stages/05_enrichment @render=dirty-set run --since-upstream --source silver.events_parsed --landed 2026-04-10..2026-04-11 --dry-run -->

<!-- PLACEHOLDER: propagation walkthrough; note the canonical example's
self-referential table makes the propagation graph refuse. -->

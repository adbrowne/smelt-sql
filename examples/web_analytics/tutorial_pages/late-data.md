# Duplicates and late data

<!-- PLACEHOLDER: intro prose. -->

## Deduplicating redeliveries — and a refusal

<!-- smelt-include: tutorial_stages/02_dedup_refused/models/silver/events_parsed.sql -->

```bash
smelt run --select silver.events_parsed \
  --event-time-start 2026-04-10 --event-time-end 2026-04-11 --dry-run
```

<!-- smelt-generate: @cwd=tutorial_stages/02_dedup_refused @render=text @expect-exit=1 run --select silver.events_parsed --event-time-start 2026-04-10 --event-time-end 2026-04-11 --dry-run -->

<!-- PLACEHOLDER: why refused, why the override is justified. -->

## Accepting late arrivals

<!-- smelt-include: tutorial_stages/03_late_data/models/silver/events_parsed.sql -->

## The derived lookback

<!-- smelt-generate: @cwd=tutorial_stages/03_late_data explain silver.events_parsed --show-sql --json --period 2026-04-10..2026-04-11 -->

<!-- PLACEHOLDER: the [D-3, D+1) read window, dbt/Spark contrast. -->

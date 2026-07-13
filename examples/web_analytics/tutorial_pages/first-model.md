# A first incremental model

<!-- PLACEHOLDER: intro prose. -->

## The model

<!-- smelt-include: tutorial_stages/01_first_model/models/silver/events_parsed.sql -->

<!-- PLACEHOLDER: frontmatter walkthrough, run commands. -->

```bash
smelt build --event-time-start 2026-04-10 --event-time-end 2026-04-11
```

## What actually runs

<!-- smelt-generate: @cwd=tutorial_stages/01_first_model explain silver.events_parsed --show-sql --json --period 2026-04-10..2026-04-11 -->

<!-- PLACEHOLDER: DELETE+INSERT walkthrough. -->

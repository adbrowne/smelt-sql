# Sessions and the cross-midnight backfill

<!-- PLACEHOLDER: intro prose — why any bounded sessionizer must cut. -->

## The sessions model

<!-- smelt-include: models/silver/sessions.sql -->

<!-- PLACEHOLDER: sessionize function summary + link; frontmatter notes. -->

## What a one-day run executes

<!-- smelt-generate: @render=skeleton explain silver.sessions --show-sql --json --period 2026-04-10..2026-04-11 -->

??? example "Full emitted SQL — `smelt explain silver.sessions --show-sql --period 2026-04-10..2026-04-11`"

    <!-- smelt-generate: explain silver.sessions --show-sql --json --period 2026-04-10..2026-04-11 -->

<!-- PLACEHOLDER: window walkthrough. -->

## The cross-midnight rewrite

<!-- PLACEHOLDER: the 2026-05-04 00:03 event narrative. -->

<!-- smelt-generate: @render=skeleton explain silver.sessions --show-sql --json --period 2026-05-04..2026-05-05 -->

??? example "Full emitted SQL — `smelt explain silver.sessions --show-sql --period 2026-05-04..2026-05-05`"

    <!-- smelt-generate: explain silver.sessions --show-sql --json --period 2026-05-04..2026-05-05 -->

## The alternative: let the session's own start decide

<!-- PLACEHOLDER: sessions_chained condensed treatment. -->

<!-- smelt-generate: @render=skeleton explain silver.sessions_chained --show-sql --json --period 2026-04-10..2026-04-11 -->

??? example "Full emitted SQL — `smelt explain silver.sessions_chained --show-sql --period 2026-04-10..2026-04-11`"

    <!-- smelt-generate: explain silver.sessions_chained --show-sql --json --period 2026-04-10..2026-04-11 -->

<!-- PLACEHOLDER: never-idle comparison table (pinned by e2e tests), trade-offs. -->

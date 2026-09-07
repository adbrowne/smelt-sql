---
materialization: table
refresh: full
---
-- Cumulative `WatchEvent` (GitHub's "star" event) count by day, over
-- `gold.events_enriched`. Full refresh, like `marts.repo_leaderboard`: no
-- incremental window to reason about over an already-maintained gold table.
--
-- Thin on purpose: the fixture holds 47 `WatchEvent`s over 30 days
-- (measured) — enough to pin an exact cumulative total in a test, not
-- enough to look like an analytics product on its own. Built anyway
-- because the research doc's full sketch names it and it is a real
-- consumer of `gold.events_enriched`
-- (`docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md` decision log).
SELECT
    event_date,
    SUM(daily_stars) OVER (ORDER BY event_date) AS cumulative_stars
FROM (
    SELECT
        event_date,
        COUNT(*) AS daily_stars
    FROM smelt.gold.events_enriched
    WHERE type = 'WatchEvent'
    GROUP BY event_date
) per_day
ORDER BY event_date

---
materialization: table
refresh: full
---
-- Total events per repo, ranked. Full refresh: reads the already-maintained
-- `gold.repo_activity_daily` and `gold.repo_dim`, no incremental window to
-- reason about here (mirrors `marts.naming_history`'s own precedent).
--
-- Reads like a leaderboard should NOT be trusted as a popularity signal:
-- the sample skews hard to newly-created bot repos (92% `PushEvent`, median
-- repo has one event, one bot repo has 527 — `README.md`). The top row is
-- the known 527-event bot repo, reproducing that skew rather than hiding
-- it (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/04-plan.md`).
SELECT
    a.repo_id,
    d.current_repo_name,
    SUM(a.event_count) AS total_events,
    SUM(a.distinct_actors) AS total_distinct_actor_days
FROM smelt.gold.repo_activity_daily a
JOIN smelt.gold.repo_dim d ON a.repo_id = d.repo_id
GROUP BY a.repo_id, d.current_repo_name
ORDER BY total_events DESC

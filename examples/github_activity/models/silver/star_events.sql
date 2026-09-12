---
timeseries:
  event_time_column: created_at
  partition_column: event_date
  granularity: day
refresh: incremental
grain: partition
---
-- Typed extraction of `WatchEvent`'s own payload field — see
-- `push_events.sql` for the shared shape and rationale. `WatchEvent` is
-- GitHub's "starred a repo" event; `marts.star_growth` already derives star
-- counts straight from `type = 'WatchEvent'` on `silver.events_deduped`
-- without this model, so this one exists purely to complete the four-model
-- fan-out criterion 4 names and to prove the extraction path on the
-- payload's smallest, simplest shape.
--
-- Probed directly against the fixture: all 47 `WatchEvent` rows carry
-- `{"action":"started"}` and nothing else — `star_action` is a constant
-- column here (GitHub never emitted a `WatchEvent` other action; `stopped`
-- was removed from the event stream before this API's payload shape). Kept
-- rather than dropped: the point of this model is the typed-extraction
-- shape, not that every extracted column varies in this particular sample.
SELECT
    id,
    actor_id,
    actor_login,
    repo_id,
    repo_name,
    created_at,
    event_date,
    JSON_EXTRACT_TEXT(payload, '$.action') AS star_action
FROM smelt.silver.events_deduped
WHERE type = 'WatchEvent'

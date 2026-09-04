---
materialization: table
refresh: incremental
grain: partition
timeseries:
  partition_column: event_date
  event_time_column: event_timestamp
  granularity: day
# `event_id` is the fact source's own declared `unique_key`
# (`models/sources/raw/events.yml`) — but a `grain: partition` output has no
# top-level `unique_key:` slot of its own (declaring one makes an output
# key-shaped, `docs/specs/models.md` §"The Relation Contract"), so this
# cell's own row identity resolves `WholeRow`, not a declared key. That is
# still the correct row identity for the derived `RecomputeRegion` /
# `Technique::DeleteInsert` corner (`docs/specs/incremental_models.md`
# §"Per-cell admission", `PartitionLocal::Yes`) when a `raw.user_status`
# mutation drives the `{status}` cell — the retired
# `batched.unique_key: [event_id]`
# sub-block spelling (now `merge_key:` in smelt.yml) this model used to
# carry here never fed row-identity derivation for a partition-grain output
# either (`derive::ModelInputs::declared_unique_key`
# is empty for every `Grain::Partition`), so dropping it changes nothing.
---
-- Fact (events) enriched with a CLOCKED, mutable dimension
-- (raw.user_status) — the genuine scan-clamp corner (`PartitionLocal::Yes`)
-- the `UpstreamMutation` cells derive as `RecomputeRegion` /
-- `Technique::DeleteInsert`, distinct from `daily_events_enriched.sql`'s
-- accepted-full-scan corner (`PartitionLocal::No`, driven by the unclocked
-- `raw.users`). `raw.user_status` is clocked (`timeseries.partition_column:
-- changed_at`) and this join carries an explicit, derivable window
-- predicate on that column relative to the fact's own `event_timestamp` —
-- `project_source_link` (`smelt-logical::maintenance::derive`) links it to the
-- output partition axis via a genuine `ScanClamp` instead of falling back
-- to the accepted-full-scan corner.
SELECT
    e.event_id,
    date_trunc('day', e.event_timestamp) AS event_date,
    e.user_id,
    e.event_type,
    s.status
FROM smelt.sources.raw.events e
JOIN smelt.sources.raw.user_status s
  ON e.user_id = s.user_id
 AND s.changed_at BETWEEN e.event_timestamp - INTERVAL '1 day'
                       AND e.event_timestamp + INTERVAL '1 day'

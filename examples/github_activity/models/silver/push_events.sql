---
timeseries:
  event_time_column: created_at
  partition_column: event_date
  granularity: day
refresh: incremental
grain: partition
---
-- Typed extraction of `PushEvent`'s own payload fields, the first of the
-- four fan-out models (`push_events`, `pr_events`, `issue_events`,
-- `star_events`) criterion 4 names
-- (`docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md`). A plain
-- `WHERE type = ` filter over `silver.events_deduped` — Form A relative to
-- it (`event_date` is a passthrough of the exact column that model already
-- computes), so the filter needs no window function and no
-- `safety_overrides`.
--
-- `JSON_EXTRACT_TEXT` is the registry's canonical name
-- (`crates/smelt-types/src/signatures/builtins/remaining.rs`): it emits as
-- `JSON_EXTRACT_STRING` on DuckDB, `GET_JSON_OBJECT` on Spark and
-- `JSON_VALUE` on BigQuery, and its declared return type is `Text` on every
-- dialect — a numeric field is still extracted as text and `CAST` to its
-- real type explicitly (`push_id` below), never inferred from the JSON
-- itself.
--
-- Fields probed directly against the fixture
-- (`seeds/github_events_sample.parquet`, `duckdb ... json_keys(payload)`):
-- every `PushEvent` row in the sample carries exactly
-- `{repository_id, push_id, ref, head, before}`, 100% present, no other
-- key ever observed. `repository_id` is dropped — it duplicates the
-- event's own `repo_id`, which is already more trustworthy (survives a
-- rename; `payload.repository_id` does not track one).
SELECT
    id,
    actor_id,
    actor_login,
    repo_id,
    repo_name,
    created_at,
    event_date,
    CAST(JSON_EXTRACT_TEXT(payload, '$.push_id') AS BIGINT) AS push_id,
    JSON_EXTRACT_TEXT(payload, '$.ref') AS git_ref,
    JSON_EXTRACT_TEXT(payload, '$.head') AS head_sha,
    JSON_EXTRACT_TEXT(payload, '$.before') AS before_sha
FROM smelt.silver.events_deduped
WHERE type = 'PushEvent'

---
timeseries:
  event_time_column: created_at
  partition_column: event_date
  granularity: day
refresh: incremental
grain: partition
---
-- Typed extraction of `PullRequestEvent`'s own payload fields — see
-- `push_events.sql` for the shared shape (Form A filter over
-- `silver.events_deduped`, `JSON_EXTRACT_TEXT` cast to the real type)
-- and `docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md` criterion
-- 4 for why this fan-out exists.
--
-- Probed directly against the fixture: `action`, `number` and
-- `pull_request.{id, head.ref, base.ref}` are 100% present across all 366
-- `PullRequestEvent` rows. `pull_request` also carries `label`/`labels` or
-- `assignee`/`assignees` on a minority of rows (the archive's schema is a
-- tagged union over `action`), neither extracted here — the fan-out proves
-- typed extraction works, not that it mirrors the whole GitHub schema
-- (`docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md`). Nor
-- is `pull_request.title`/`.user`/`.merged` extracted: this trimmed
-- BigQuery Archive payload (unlike the full historical archive) does not
-- carry them — every field below was confirmed present, not assumed from
-- GitHub's public schema docs.
--
-- `pr_action` (not `action`) because `ACTION` is a DuckDB keyword in some
-- grammar positions (`MERGE ... ON CONFLICT ... DO NOTHING`); matching
-- `issue_action` below for the same reason on `issue_events.sql`.
SELECT
    id,
    actor_id,
    actor_login,
    repo_id,
    repo_name,
    created_at,
    event_date,
    JSON_EXTRACT_TEXT(payload, '$.action') AS pr_action,
    CAST(JSON_EXTRACT_TEXT(payload, '$.number') AS BIGINT) AS pr_number,
    CAST(JSON_EXTRACT_TEXT(payload, '$.pull_request.id') AS BIGINT) AS pr_id,
    JSON_EXTRACT_TEXT(payload, '$.pull_request.head.ref') AS head_ref,
    JSON_EXTRACT_TEXT(payload, '$.pull_request.base.ref') AS base_ref
FROM smelt.silver.events_deduped
WHERE type = 'PullRequestEvent'

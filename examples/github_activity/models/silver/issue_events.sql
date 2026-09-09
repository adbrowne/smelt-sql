---
timeseries:
  event_time_column: created_at
  partition_column: event_date
  granularity: day
refresh: incremental
grain: partition
---
-- Typed extraction of `IssuesEvent`'s own payload fields — see
-- `push_events.sql` for the shared shape and rationale.
--
-- Not `IssueCommentEvent` (a different `type`, and the fixture's largest
-- payloads by far — up to 89,669 bytes, comment bodies): `issue_events`
-- covers only `IssuesEvent` (opened/closed/labeled/... state transitions on
-- the issue itself), matching the fan-out's four named models exactly
-- (`docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md` criterion 4).
--
-- Probed directly against the fixture: `action`, `issue.number`,
-- `issue.state` and `issue.title` are 100% present across all 128
-- `IssuesEvent` rows, and `issue.title` tops out at 128 bytes here — small
-- enough to keep as a plain column rather than treating it like the
-- comment-body payloads this model deliberately excludes. `label`/`labels`
-- and `assignee`/`assignees` appear on a minority of rows and are not
-- extracted, for the same reason `pr_events.sql` skips them.
--
-- `issue_action` (not `action`) — see `pr_events.sql`'s note on `pr_action`.
SELECT
    id,
    actor_id,
    actor_login,
    repo_id,
    repo_name,
    created_at,
    event_date,
    JSON_EXTRACT_TEXT(payload, '$.action') AS issue_action,
    CAST(JSON_EXTRACT_TEXT(payload, '$.issue.number') AS BIGINT) AS issue_number,
    JSON_EXTRACT_TEXT(payload, '$.issue.state') AS issue_state,
    JSON_EXTRACT_TEXT(payload, '$.issue.title') AS issue_title
FROM smelt.silver.events_deduped
WHERE type = 'IssuesEvent'

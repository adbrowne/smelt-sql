-- The pinned sample contract for examples/github_activity/.
--
-- This exact query defines the rows both legs of the pipeline see. The DuckDB
-- leg runs it once to produce the committed Parquet fixture; the BigQuery
-- loader must reproduce it verbatim into `raw.github_events`, or the two
-- targets are no longer comparable and the dual-target parity check is
-- meaningless.
--
-- Notes that are not obvious from reading it:
--   * `githubarchive.day` is daily-SHARDED tables, not a partitioned one, and
--     it also holds views (`yesterday`, …). A bare `day.*` wildcard therefore
--     fails with "Views cannot be queried through prefix" — the `2026*` prefix
--     excludes them, and `_TABLE_SUFFIX` (MMDD, lexicographically ordered) is
--     what prunes the scan.
--   * The sample is `MOD(repo.id, 1000) = 0`: `repo.id` is stable under rename,
--     where a `repo.name` prefix would silently drop a repo the moment it was
--     renamed — corrupting exactly the rename history the succession work needs.
--   * `payload` is projected as the raw JSON string the archive stores. It is
--     the single most expensive column in the table, and BigQuery bills bytes
--     scanned — the column projection, not the sample filter, is what governs
--     the bill, so nothing else is added speculatively. The typed silver
--     fan-out (`push_events`, `pr_events`, `issue_events`, `star_events`)
--     extracts from it; without it those models have no subject.
SELECT
  id,
  type,
  created_at,
  actor.id    AS actor_id,
  actor.login AS actor_login,
  repo.id     AS repo_id,
  repo.name   AS repo_name,
  org.id      AS org_id,
  public,
  payload
FROM `githubarchive.day.2026*`
WHERE _TABLE_SUFFIX BETWEEN '0805' AND '0903'
  AND MOD(repo.id, 1000) = 0
ORDER BY created_at, id

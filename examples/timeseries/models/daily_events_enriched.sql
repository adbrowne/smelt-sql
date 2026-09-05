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
# cell's own row identity resolves `WholeRow`. That is what every cell's
# derived technique, `{user_name}` included, is keyed by: a region
# `DELETE`+`INSERT` (`Technique::DeleteInsert`) — the retired
# `batched.unique_key: [event_id]` sub-block spelling (now
# `merge_key:` in smelt.yml) this model used to carry here never fed
# row-identity derivation for a partition-grain output either
# (`derive::ModelInputs::declared_unique_key` is empty for every
# `Grain::Partition`), so dropping it changes nothing.
maintenance:
  scan_bounds:
    per_source:
      raw.users:
        allow_full_scan: true
---
-- Fact (events) enriched with the dimension (users). `raw.users` is an
-- unclocked, explicitly `mutation_profile: mutable_snapshot` dimension:
-- renaming a user broadcasts to every fact row that references them — the
-- `{user_name}` column group's `UpstreamMutation` cell. But the enrichment
-- reads `raw.users` in an inner `JOIN`'s own `ON` predicate, a row-admission
-- read: membership sensitivity is row-scoped, so no column group of this
-- `SELECT` can be proven value-only, and the cell falls back to the region
-- recompute `DELETE`+`INSERT`, not a narrower per-column write
-- (docs/specs/incremental_models.md §"Per-cell admission").
--
-- `event_date` is a `CAST`, not `date_trunc(...)`: the P1 skeleton-source-
-- closure proof's per-column provenance conjunct (`model_properties.md`
-- §"Skeleton-source closure") resolves a `CAST`/arithmetic expression's
-- inner column reference correctly but currently misresolves a bare
-- `FUNCTION_CALL`-shaped one once 2+ FROM sources are in scope (the
-- call's own name token is misread as an unqualified column reference,
-- short-circuiting before its argument is visited) — tracked as a
-- pre-existing `smelt-parser`/`smelt-logical::analysis::skeleton_closure`
-- gap, not something T3 over external sources (`docs/plans/
-- 20260715-composed-axes-conditional-maintenance.md` Phase F5) fixes.
-- `CAST` produces the identical day value and is the supported form.
SELECT
    e.event_id,
    CAST(e.event_timestamp AS DATE) AS event_date,
    e.user_id,
    e.event_type,
    u.user_name
FROM smelt.sources.raw.events e
JOIN smelt.sources.raw.users u ON e.user_id = u.user_id

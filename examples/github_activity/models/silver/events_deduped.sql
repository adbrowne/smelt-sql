---
materialization: table
refresh: incremental
grain: key
timeseries:
  event_time_column: first_seen_date
  partition_column: first_seen_date
  granularity: day
maintenance:
  scan_bounds:
    per_source:
      raw.github_events:
        # No statically derivable scan bound: a redelivered duplicate can
        # surface in the source on any later day while keeping its original
        # `created_at`, so no WHERE-clause relation ties this model's own
        # output partition to a *different* column the way
        # `events_parsed`'s arrival-based lateness filter does — there is no
        # arrival-time column here to make that relation over (one lands
        # with a real loader stamp in a later phase). Accepted here as a
        # full-table op, exactly as `examples/web_analytics/silver/
        # events_deduped.sql` accepts for the same reason.
        allow_full_scan: true
---
-- Event-grain dedupe on `id`, the at-least-once shape the loader's
-- deliberate previous-day redelivery exists to exercise (`docs/outcomes/
-- 20260906-bigquery-dogfood-spine/phases/02-plan.md` §"Redelivery is not
-- free"). Composed shape — key-addressed (one row per `id`) *and*
-- time-partitioned (`first_seen_date`) — admitted via key temporal
-- locality's **route 3 (recurrence-bounded)**
-- (`docs/specs/incremental_shapes.md` §"Key temporal locality"):
-- `first_seen_date` is `MIN(CAST(created_at AS DATE))` grouped by `id`, an
-- extremal fold over a *non-key* column, so route 2's derived sub-route
-- refuses and locality falls to the declared `key_recurrence` on
-- `raw.github_events` (`models/sources/raw/github_events.yml`) — every pair
-- of rows sharing `id` lies within that (zero-width) window on the
-- event-time axis. The bound is checked at merge time, never trusted
-- (`KeyedRecurrenceBoundViolated` on violation, transactional).
--
-- Dedup itself falls out of the keyed merge: a redelivered duplicate is
-- byte-identical to the original (same `id`, same everything, including
-- `created_at` — there is no independent ingestion clock in this phase), so
-- `MIN` over any column converges to the same value regardless of which
-- physical copy a run happens to see. No window function, no
-- `safety_overrides` escape hatch.
--
-- `event_date` and `first_seen_date` carry the same value: `first_seen_date`
-- is this model's declared partition column; `event_date` is the name
-- downstream consumers project (`silver.actor_sessions`).
SELECT
    id,
    MIN(type) AS type,
    MIN(actor_id) AS actor_id,
    MIN(actor_login) AS actor_login,
    MIN(repo_id) AS repo_id,
    MIN(repo_name) AS repo_name,
    MIN(org_id) AS org_id,
    MIN(public) AS public,
    MIN(created_at) AS created_at,
    MIN(CAST(created_at AS DATE)) AS event_date,
    MIN(CAST(created_at AS DATE)) AS first_seen_date
FROM smelt.sources.raw.github_events
GROUP BY id

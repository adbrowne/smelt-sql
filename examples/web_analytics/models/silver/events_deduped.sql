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
      raw.events:
        # A late-arriving/redelivered event past this key's already-merged
        # state has no statically derivable scan bound for its mutation
        # cell — accepted here as a full-table op rather than bounded.
        allow_full_scan: true
---
-- Event-grain dedupe under the **composed shape** — key-addressed (one row
-- per `event_id`) *and* time-partitioned (`first_seen_date`)
-- (`docs/specs/incremental_shapes.md` §"Key temporal locality (the
-- time-partitioned output)"). This is the flagship demonstration of "the two
-- axes are orthogonal": identity (`event_id`) and time (`first_seen_date`)
-- are declared independently, and their combination is what makes this
-- model — unlike a bare keyed lookup — a clocked node the rest of the DAG
-- can propagate through (§"What the composed shape uniquely enables").
--
-- **Admission route: 3 (recurrence-bounded), declared sub-route.**
-- `first_seen_date` is not a `unique_key` column (route 1) and there is no
-- declared functional dependency (route 2); locality is established
-- instead by `smelt.sources.raw.events`'s declared
-- `mutation_profile.key_recurrence` (`models/sources/raw/events.yml`) —
-- every pair of rows sharing `event_id` lies within that window on the
-- event-time axis. The bound is *checked*, never trusted: `smelt-runtime`
-- verifies at merge time that no delta row matches a stored key outside the
-- declared window (`KeyedRecurrenceBoundViolated` on violation,
-- transactional — the target is left unchanged, never partially written).
-- This model deliberately carries **no** WHERE-clause lateness filter of
-- its own: route 3's static sub-route (`establish_locality` in
-- `crates/smelt-logical/src/maintenance/locality.rs`) recognizes a WHERE
-- filter bounding the driving source's own partition column by a fixed
-- `INTERVAL` offset against itself (the same Form B pattern
-- `silver.events_parsed` uses) and would admit *that* proof-backed,
-- unchecked static bound before ever consulting the declared
-- `key_recurrence` below — silently taking the wrong sub-route despite the
-- source declaring one. The runtime derives the merge target's scan bound
-- from the established `LocalitySlice` alone (`RecurrenceBounded`'s own
-- `margin_before`/`margin_after`, `crates/smelt-runtime/src/
-- maintenance_driver.rs`), not from anything this model's own SELECT
-- filters, so no self-imposed WHERE bound is needed for correctness here.
--
-- **Redelivery, absorbed by the composed shape instead of a QUALIFY
-- window.** `silver.events_parsed` drops a redelivered duplicate with a
-- `QUALIFY ROW_NUMBER` ordering window keyed by event_id and ordered by
-- arrival_time, keeping only the earliest arrival — a
-- `batched.safety_overrides.allow_window_functions` escape hatch.
-- This model instead folds every duplicate through a per-column extremal
-- combiner (`MIN`) keyed by `event_id`: a redelivered duplicate is
-- byte-identical to the original except `arrival_time` (`datagen.yaml`'s
-- `redelivery:` block), so `MIN` over any of its columns converges to the
-- same value regardless of which copy a run happens to see — dedup falls
-- out of the keyed merge itself, with no window function and no
-- safety-override needed. `MIN`'s argument may be an arbitrary expression
-- (the classifier only requires the *outer* call be a single, unwrapped
-- aggregate — `crates/smelt-logical/src/rules/cumulative.rs`'s
-- `is_direct_function_call`), so the `amplitude_id` computation and the
-- JSON payload extraction — both pure functions of one row's own columns —
-- fold through `MIN` exactly like a bare column reference would.
--
-- Two output columns carry the same value: `first_seen_date` is this
-- model's declared partition column (feeding the `timeseries:` block
-- above); `event_date` is the historical column name every existing
-- downstream consumer (`silver.events_enriched`, `silver.sessions`,
-- `silver.sessions_chained`, `silver.device_user_edges`,
-- `gold.eventstream_with_identity`, `gold.identity_forward_only`) already
-- reads — kept so the FROM-clause swap from `silver.events_parsed` to this
-- model needed no further column-name rewrite downstream.
--
-- `silver.events_parsed` is kept as-is (the QUALIFY-based "before" shape) —
-- no downstream model reads it anymore, but its own `smelt build`/`smelt
-- test` coverage stays green so it can still teach the workaround it
-- replaces.
SELECT
    event_id,
    MIN(device_id) AS device_id,
    MIN(user_id) AS user_id,
    MIN(CASE WHEN user_id IS NOT NULL
             THEN 'u:' || CAST(user_id AS VARCHAR)
             ELSE 'd:' || CAST(device_id AS VARCHAR)
        END) AS amplitude_id,
    MIN(CAST(event_time AS TIMESTAMP)) AS event_ts,
    MIN(CAST(event_date AS DATE)) AS event_date,
    MIN(CAST(event_date AS DATE)) AS first_seen_date,
    MIN(utm_campaign) AS utm_campaign,
    MIN(json_extract_string(payload, '$.event_name')) AS event_name,
    MIN(json_extract_string(payload, '$.platform')) AS platform,
    MIN(json_extract_string(payload, '$.url')) AS url
FROM smelt.sources.raw.events
GROUP BY event_id

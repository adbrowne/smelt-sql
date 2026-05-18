# Web analytics — three-way user stitching

A self-contained smelt example demonstrating a bronze→silver→gold pipeline over
JSON-encoded web events, with three parallel user-identity-resolution algorithms
surfaced side-by-side in a single wide event-level table so the algorithmic
tradeoff is observable row-by-row.

## Reference

Inspired by the Amplitude identity-stitching methodology
([docs](https://amplitude.com/docs/data/sources/instrument-track-unique-users)).
The three algorithms below are not faithful reproductions of any Amplitude
implementation, but they cover the same algorithmic spectrum
(in-session → per-device → cross-device).

### Forward-only

Within-session resolution. Each session's identifiable events are tagged with
the latest in-session signed-in user, via `arg_max(user_id, event_ts) FILTER
(WHERE user_id IS NOT NULL) OVER (PARTITION BY session_id)`. No cross-session
propagation, no per-device election, no clustering. Sessions with zero
signed-in events stay anonymous.

### Backward-fill

Per-device canonical-user election. The most-frequent signed-in user across
all sessions on a device wins; ties are broken by first-seen, then by smallest
user_id. Once a user has signed in on a device, every event on that device
retroactively belongs to that user — regardless of session.

### Connected-components

Cross-device clustering via union-find over the `(device, user)` co-occurrence
graph. The cluster's representative is the smallest user_id in the cluster,
and every event of every device in the cluster is tagged with that
representative. Implemented as 8-iteration unrolled label propagation over
`silver/device_user_edges`; true recursive-CTE fixed-point convergence is a
possible future extension.

## Pipeline

```
bronze/raw_events
  └── silver/events_parsed           (parse JSON event_payload column)
        ├── silver/sessions          (30-min inactivity + platform boundary,
        │                             incremental with 7-day lookback)
        ├── silver/device_user_edges (device-user co-occurrence evidence)
        ├── gold/identity_forward_only        (per-session resolution)
        ├── gold/identity_backward_fill       (per-device election)
        └── gold/identity_connected_components (cross-device clustering)
              └── gold/eventstream_with_identity (wide event-level table
                                                   joining all three)
                    ├── marts/daily_active_users_by_method
                    └── marts/identity_method_comparison
```

Source files:

- [`models/bronze/raw_events.sql`](models/bronze/raw_events.sql)
- [`models/silver/events_parsed.sql`](models/silver/events_parsed.sql) +
  [`functions/parse_event_payload.sql`](functions/parse_event_payload.sql)
- [`models/silver/sessions.sql`](models/silver/sessions.sql) +
  [`functions/sessionize.sql`](functions/sessionize.sql)
- [`models/silver/device_user_edges.sql`](models/silver/device_user_edges.sql)
- [`models/gold/identity_forward_only.sql`](models/gold/identity_forward_only.sql)
- [`models/gold/identity_backward_fill.sql`](models/gold/identity_backward_fill.sql)
- [`models/gold/identity_connected_components.sql`](models/gold/identity_connected_components.sql)
- [`models/gold/eventstream_with_identity.sql`](models/gold/eventstream_with_identity.sql)
- [`models/marts/daily_active_users_by_method.sql`](models/marts/daily_active_users_by_method.sql)
- [`models/marts/identity_method_comparison.sql`](models/marts/identity_method_comparison.sql)

## Inline tests

- [`tests/session_boundary_invariants.test.sql`](tests/session_boundary_invariants.test.sql) —
  asserts the 30-minute inactivity rule and the platform-boundary split
  produce the expected session_id assignments on a mocked event sequence.
- [`tests/forward_only_resolution_invariants.test.sql`](tests/forward_only_resolution_invariants.test.sql) —
  asserts within-session `arg_max` resolution: a session's `forward_only_user_id`
  is the latest non-null `user_id` observed inside the session window; sessions
  with no signed-in observations resolve to NULL.
- [`tests/backward_fill_resolution_invariants.test.sql`](tests/backward_fill_resolution_invariants.test.sql) —
  asserts the per-device canonical-user election (most-frequent user wins;
  first_seen + user_id tiebreaks).
- [`tests/connected_components_resolution_invariants.test.sql`](tests/connected_components_resolution_invariants.test.sql) —
  asserts the cluster representative on a 3-device / 3-user shared-device
  fixture: all events resolve to the cluster minimum.
- [`tests/dau_monotonicity_invariants.test.sql`](tests/dau_monotonicity_invariants.test.sql) —
  asserts the `identified_events_forward_only ≤ identified_events_backward_fill
  ≤ identified_events_connected_components` subsumption invariant on the DAU
  mart, plus per-day `dau_*` shape including the cluster-collapse case where
  `dau_connected_components < dau_backward_fill`.

## Run locally

```bash
smelt-datagen --config datagen.yaml --scale-factor 0.01
duckdb target/dev.duckdb < setup_sources.sql
smelt build
smelt test
```

`scale_factor` controls the dataset size: `0.01` produces ~10K events over 60
days across ~1500 devices and ~500 users — small enough for a laptop dev loop,
large enough that all three identity algorithms produce non-trivial output.
Bump to `0.1` or `1.0` for fuller datasets (1M events at `scale_factor=1.0`).

## Inspect the marts

### Daily active users under each method

```sql
SELECT *
FROM main.marts_daily_active_users_by_method
ORDER BY event_date
LIMIT 7;
```

Each row reports per-day `(total_events, dau_forward_only, dau_backward_fill,
dau_connected_components, identified_events_forward_only,
identified_events_backward_fill, identified_events_connected_components)`.
The `identified_events_*` columns are subsumption-monotonic by construction
(every event that forward-only identifies is also identified by backward-fill,
which is also identified by connected-components — this is the verification
gate). The `dau_*` columns are **not** monotonic in the same direction:
connected-components collapses cross-device clusters to a single
representative, so `dau_connected_components` can be **smaller** than
`dau_backward_fill` on days where shared devices are active. Both shapes are
intentional — `identified_events` measures reach, `dau` measures cardinality
after cluster collapse.

### Pairwise method comparison

```sql
SELECT *
FROM main.marts_identity_method_comparison
ORDER BY comparison_name;
```

Three rows, one per pair (`backward_vs_connected`, `forward_vs_backward`,
`forward_vs_connected`), each reporting `(comparison_name, total_events,
agree_events, disagree_events, only_left_identified, only_right_identified,
both_null_events)`.
The disjointness invariant
`agree + disagree + only_left + only_right + both_null = total_events` holds
on every row. The qualitative shape is dataset-dependent: with 10% shared
and 5% multi-device users in the synthetic linked_choice distribution,
`disagree_events` is non-trivial on every pair because the three algorithms'
resolution rules disagree whenever a device has multiple distinct signed-in
users with different elections (most-frequent vs latest-in-session vs
cluster-minimum). `only_right_identified` measures how much *broader* the
right-hand algorithm's reach is — for `forward_vs_backward` and
`forward_vs_connected` this is the dominant non-agree bucket, reflecting that
backward-fill and connected-components identify events across sessions and
devices that forward-only leaves anonymous.

### How shared-device events differ

```sql
SELECT
    forward_only_user_id,
    backward_fill_user_id,
    connected_components_user_id,
    COUNT(*) AS events
FROM main.gold_eventstream_with_identity
WHERE forward_only_user_id IS NOT NULL
  AND backward_fill_user_id IS NOT NULL
  AND forward_only_user_id != backward_fill_user_id
GROUP BY 1, 2, 3
ORDER BY events DESC
LIMIT 10;
```

Surfaces the top divergences row-by-row. Each row is a `(forward, backward,
connected)` triple over a population of events: forward-only resolved the
session to one signed-in user, backward-fill resolved the device to a
different most-frequent user, and connected-components picked the cluster
minimum. Useful for sanity-checking the three algorithms on real divergent
cases rather than aggregate statistics.

## How this example was built

Multi-session implementation tracked in
[`docs/plans/20260517-web-analytics-example.md`](../../docs/plans/20260517-web-analytics-example.md).

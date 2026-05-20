# Web analytics — amplitude_id and three refinements (incremental)

A self-contained smelt example demonstrating a bronze→silver→gold pipeline over
JSON-encoded web events, with an Amplitude-style always-present `amplitude_id`
as the no-merging baseline plus three parallel refinements that progressively
merge the identity space, surfaced side-by-side in a single wide event-level
table so the algorithmic tradeoff is observable row-by-row.

Every model that has a natural time dimension is incremental, partitioned by
day. The pipeline is driven by `run_incremental.py`, which generates data and
then walks the datagen window day-by-day, invoking `smelt run` per day with a
2-day window to honour the 1-day lookback the session and forward-only models
need.

## Reference

Inspired by the Amplitude identity-stitching methodology
([docs](https://amplitude.com/docs/data/sources/instrument-track-unique-users)).
The methods below are not faithful reproductions of any Amplitude
implementation, but they cover the same algorithmic spectrum
(no-merging → in-session → per-device → cross-device).

### amplitude_id (no-merging baseline)

`silver/events_parsed` synthesises a never-NULL `amplitude_id` per event:
`'u:' || user_id` when the event carries a signed-in `user_id`, else
`'d:' || device_id`. The `'u:'` / `'d:'` prefixes keep the two ID namespaces
disjoint, so cross-device or cross-user collisions are impossible. This is the
identity Amplitude would emit *before* any cross-event merging — every event
has an identity, but two events on the same anonymous device share a `'d:'`
identity, and two signed-in events from the same user on different devices
share a `'u:'` identity.

### Forward-only

Within-session refinement. Each session's identifiable events are tagged with
the latest in-session signed-in user, via `arg_max(user_id, event_ts) FILTER
(WHERE user_id IS NOT NULL)` grouped by `session_id`, prefixed with `'u:'`.
No cross-session propagation, no per-device election, no clustering. Sessions
with zero signed-in events resolve to NULL at the model boundary and the
eventstream COALESCEs them to the device-prefix `amplitude_id`.

### Backward-fill

Per-device canonical-user election. The most-frequent signed-in user across
all sessions on a device wins; ties are broken by first-seen, then by smallest
user_id. Once a user has signed in on a device, every event on that device
retroactively belongs to that user — regardless of session. Output is
`'u:' || elected user_id`; devices that never had a signed-in event fall back
to the device-prefix `amplitude_id` at the eventstream COALESCE.

### Connected-components

Cross-device clustering via union-find over the `(device, user)` co-occurrence
graph. The cluster's representative is `'u:' || min(user_id)` in the cluster,
and every event of every device in the cluster is tagged with that
representative. Implemented as 8-iteration unrolled label propagation over
`silver/device_user_edges_cumulative`; true recursive-CTE fixed-point
convergence is a possible future extension.

## Pipeline

```
bronze/raw_events                  (view; passthrough)
  └── silver/events_parsed         (INCR by event_date)
        ├── silver/sessions        (INCR by session_start_date; 1-day lookback)
        │     └── gold/identity_forward_only         (INCR by session_start_date)
        └── silver/device_user_edges                 (INCR by event_date; per-day rows)
              └── silver/device_user_edges_cumulative (view; rolls per-day rows up)
                    ├── gold/identity_backward_fill        (view; rebuilt on query)
                    └── gold/identity_connected_components (view; rebuilt on query)
        ↓
        gold/eventstream_with_identity (INCR by event_date)
              ├── marts/daily_active_users_by_method (INCR by event_date)
              └── marts/identity_method_comparison   (view; global 3-row aggregation)
```

Source files:

- [`models/bronze/raw_events.sql`](models/bronze/raw_events.sql)
- [`models/silver/events_parsed.sql`](models/silver/events_parsed.sql) +
  [`functions/parse_event_payload.sql`](functions/parse_event_payload.sql)
- [`models/silver/sessions.sql`](models/silver/sessions.sql) +
  [`functions/sessionize.sql`](functions/sessionize.sql) +
  [`functions/compute_session_start_date.sql`](functions/compute_session_start_date.sql)
- [`models/silver/device_user_edges.sql`](models/silver/device_user_edges.sql)
- [`models/silver/device_user_edges_cumulative.sql`](models/silver/device_user_edges_cumulative.sql)
- [`models/gold/identity_forward_only.sql`](models/gold/identity_forward_only.sql)
- [`models/gold/identity_backward_fill.sql`](models/gold/identity_backward_fill.sql)
- [`models/gold/identity_connected_components.sql`](models/gold/identity_connected_components.sql)
- [`models/gold/eventstream_with_identity.sql`](models/gold/eventstream_with_identity.sql)
- [`models/marts/daily_active_users_by_method.sql`](models/marts/daily_active_users_by_method.sql)
- [`models/marts/identity_method_comparison.sql`](models/marts/identity_method_comparison.sql)

## Incremental shape

Six models are incremental; the rest are views.

| Model                                         | Materialization | Partition column   |
|-----------------------------------------------|-----------------|--------------------|
| `silver/events_parsed`                        | INCR table      | `event_date`       |
| `silver/sessions`                             | INCR table      | `session_start_date` |
| `silver/device_user_edges`                    | INCR table      | `event_date`       |
| `gold/identity_forward_only`                  | INCR table      | `session_start_date` |
| `gold/eventstream_with_identity`              | INCR table      | `event_date`       |
| `marts/daily_active_users_by_method`          | INCR table      | `event_date`       |
| `bronze/raw_events`                           | view            | —                  |
| `silver/device_user_edges_cumulative`         | view            | —                  |
| `gold/identity_backward_fill`                 | view            | —                  |
| `gold/identity_connected_components`          | view            | —                  |
| `marts/identity_method_comparison`            | view            | —                  |

### Why some identity models stay views

The two global identity algorithms (backward_fill, connected_components) need
the cumulative `(device, user)` edge set to produce correct per-device
elections and clusters. Splitting `silver/device_user_edges` into a per-day
incremental table plus a `silver/device_user_edges_cumulative` view keeps the
daily-run cost proportional to that day's signed-in events while still
exposing the full edge set to the two algorithms. They remain views and are
re-evaluated on every query against `gold/eventstream_with_identity`.

`marts/identity_method_comparison` is a 3-row global aggregation with no time
dimension, so no `partition_column` exists; it stays a view too.

### Why the driver runs a 2-day window

`gold/identity_forward_only` is incremental by `session_start_date`. A session
that started yesterday but received its latest signed-in event today should
have its mapping refreshed. The driver achieves this by always running
`smelt run --event-time-start D-1 --event-time-end D+1`, so day D's iteration
re-resolves both today's and yesterday's session-start partitions. The 2-day
window catches:

- sessions whose latest signed-in event arrives one day after session start
- sessions straddling midnight (the 30-minute inactivity rule still applies)

It does **not** catch signed-in events arriving ≥2 days after session start.
Accepted limitation for the example.

### Day-by-day is not equivalent to a full rebuild on the global identity columns

This is the load-bearing insight of the example.

`backward_fill` and `connected_components` are *global* — their per-device
output depends on the cumulative `(device, user)` edge set across all dates.
When day D's run materialises `gold/eventstream_with_identity` for that day,
the LEFT JOINs to the two views see only the edges visible at the time of
day D's run.  Day D+1 may add edges that would have changed day D's mapping,
but day D's rows are not retroactively rewritten by DELETE+INSERT-per-
partition.

So the day-by-day pipeline produces *as-of-day-D* identity per day, while a
single full-window rebuild produces a single global snapshot.  The two
disagree on the global identity columns and agree exactly on the local ones:

| Column                                        | Day-by-day vs full-window |
|-----------------------------------------------|---------------------------|
| `total_events`, `event_date`                  | always equal              |
| `dau_raw`, `identified_events_raw`            | always equal              |
| `dau_forward_only`, `identified_events_forward_only` | always equal       |
| `dau_backward_fill`, `identified_events_backward_fill` | usually differ   |
| `dau_connected_components`, `identified_events_connected_components` | usually differ |

This mirrors how production streaming pipelines emit "as-of" daily metrics
rather than retroactively backfilling history every run.  `verify_incremental_equivalence.py`
asserts the local-column equality and prints the global-column divergence as
a sanity check.

### One known cost: `sessions` reads all events per partition

Smelt injects the partition filter on the *outermost* SELECT only. Because
`sessionize` is a transparent function, its `LAG OVER` runs over the entire
`silver/events_parsed` table on every iteration before the outer
`WHERE session_start_date >= D AND session_start_date < D+1` filter is
applied. The output is correct and only today's rows are written, but the
compute scales with all-history events, not just today's. Source-level filter
pushdown is on smelt's roadmap; until it lands, this is the price of running
`sessionize` inside an incremental model.

## Run locally

```bash
python run_incremental.py --scale-factor 0.01
```

Default behaviour: wipe `target/dev.duckdb`, regenerate data via
`smelt-datagen`, materialise the raw source tables via `setup_sources.sql`,
then loop day-by-day across the datagen window (2026-03-19 .. 2026-05-17 by
default, 60 days). Each iteration invokes `smelt run --event-time-start D-1
--event-time-end D+1`. After the loop the script runs `smelt test` so all
inline invariants are checked against the final cumulative state.

Per-iteration output:

```
[datagen] 0.2s
[setup] 0.1s
[day  1/60] 2026-03-19  smelt run [prev=2026-03-18 next=2026-03-20]  0.2s
[day  2/60] 2026-03-20  smelt run [prev=2026-03-19 next=2026-03-21]  0.2s
...
[tests] 0.1s

=== summary ===
  60 days replayed in 12.5s (0.21s/day)
```

Useful flags:

- `--start-date 2026-04-01` — start from a later date.
- `--days 7` — only process 7 days from the start date.
- `--scale-factor 0.1` — larger datagen output (default 0.01 ≈ 10K events).
- `--skip-datagen` — reuse the existing `data/` Parquet and `target/dev.duckdb`
  (useful when iterating on model SQL after a fresh datagen run).

A per-iteration timing report is written to `.last_run.json`.

### Equivalence verification

```bash
python verify_incremental_equivalence.py --scale-factor 0.01 --days 5
```

Runs both pipelines (full-window single rebuild + day-by-day replay) against
the same datagen output, then asserts the local-column equality and prints
the expected global-column divergence.  See "Day-by-day is not equivalent to
a full rebuild on the global identity columns" above for what each column
means.

## Inspect the marts

### Daily active users under each method

```sql
SELECT *
FROM main.marts_daily_active_users_by_method
ORDER BY event_date
LIMIT 7;
```

Each row reports per-day `(total_events, dau_raw, dau_forward_only,
dau_backward_fill, dau_connected_components, identified_events_raw,
identified_events_forward_only, identified_events_backward_fill,
identified_events_connected_components)`.

The `identified_events_*` columns are fully four-way monotone per day:

- `identified_events_raw ≤ identified_events_forward_only ≤
  identified_events_backward_fill ≤ identified_events_connected_components` —
  each method only ever promotes events from the device-fallback `'d:'`
  namespace to the `'u:'` namespace, never the other way around.

The `dau_*` columns are **not** all-pairs monotone per day. The four methods
are different partitions of the same identity space rather than a single
refinement chain. The only guaranteed per-day DAU inequality is
`dau_backward_fill ≥ dau_connected_components` (cluster collapse is a strict
per-device coarsening). Examples of the non-monotonic pairs in real datasets:

- `dau_forward_only` can exceed `dau_raw` on days where many anon events
  belong to sessions whose signins happened on *earlier* days. forward_only
  "inherits" those identities into the day's events; raw doesn't.
- `dau_backward_fill` can exceed `dau_forward_only` on days where many
  devices have only anon-only sessions on that day but signed-in events on
  other days. backward_fill recovers a `'u:'` identity for those events
  (the device's canonical user); forward_only leaves them at `'d:device_id'`.

### Pairwise method comparison

```sql
SELECT *
FROM main.marts_identity_method_comparison
ORDER BY comparison_name;
```

Three rows, one per pair (`backward_vs_connected`, `forward_vs_backward`,
`forward_vs_connected`), each reporting `(comparison_name, total_events,
agree_user_events, agree_device_events, disagree_events, only_left_user,
only_right_user)`.
The disjointness invariant
`agree_user + agree_device + disagree + only_left_user + only_right_user
= total_events` holds on every row. The qualitative shape is
dataset-dependent: with 10% shared and 5% multi-device users in the synthetic
linked_choice distribution, `disagree_events` is non-trivial on every pair
because the three algorithms' resolution rules disagree whenever a device has
multiple distinct signed-in users with different elections (most-frequent vs
latest-in-session vs cluster-minimum). `only_right_user` measures how much
*broader* the right-hand algorithm's reach is — for `forward_vs_backward`
and `forward_vs_connected` this is the dominant non-agree bucket, reflecting
that backward-fill and connected-components promote events to a real-user
identity that forward-only leaves at the device-fallback `'d:'` namespace.
The `agree_device_events` bucket counts events where *both* methods fell back
to the device — the "no real user known" cell of the partition.

### How shared-device events differ

```sql
SELECT
    forward_only_amplitude_id,
    backward_fill_amplitude_id,
    connected_components_amplitude_id,
    COUNT(*) AS events
FROM main.gold_eventstream_with_identity
WHERE forward_only_amplitude_id LIKE 'u:%'
  AND backward_fill_amplitude_id LIKE 'u:%'
  AND forward_only_amplitude_id != backward_fill_amplitude_id
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

## Inline tests

- [`tests/session_boundary_invariants.test.sql`](tests/session_boundary_invariants.test.sql) —
  asserts the 30-minute inactivity rule and the platform-boundary split
  produce the expected session_id assignments on a mocked event sequence.
- [`tests/device_user_edges_per_day_invariants.test.sql`](tests/device_user_edges_per_day_invariants.test.sql) —
  asserts the per-day aggregation shape (`daily_event_count`, `daily_first_seen`,
  `daily_last_seen`) and that anonymous events are excluded.
- [`tests/forward_only_resolution_invariants.test.sql`](tests/forward_only_resolution_invariants.test.sql) —
  asserts within-session `arg_max` resolution: a session's `forward_only_amplitude_id`
  is `'u:' || ` the latest non-null `user_id` observed inside the session window;
  sessions with no signed-in observations resolve to NULL at the model boundary
  (the eventstream COALESCEs them to the device fallback downstream).
- [`tests/backward_fill_resolution_invariants.test.sql`](tests/backward_fill_resolution_invariants.test.sql) —
  asserts the per-device canonical-user election (most-frequent user wins;
  first_seen + user_id tiebreaks) against the cumulative edges view.
- [`tests/connected_components_resolution_invariants.test.sql`](tests/connected_components_resolution_invariants.test.sql) —
  asserts the cluster representative on a 3-device / 3-user shared-device
  fixture against the cumulative edges view: all events resolve to the
  cluster minimum.
- [`tests/dau_monotonicity_invariants.test.sql`](tests/dau_monotonicity_invariants.test.sql) —
  asserts the `identified_events_*` four-way monotonicity on the DAU mart
  (`raw ≤ forward_only ≤ backward_fill ≤ connected_components`) and the
  per-day `dau_*` shape including the cluster-collapse case where
  `dau_connected_components < dau_backward_fill` (Day 2 of the fixture).

## How this example was built

Multi-session implementation tracked in
[`docs/plans/20260517-web-analytics-example.md`](../../docs/plans/20260517-web-analytics-example.md).

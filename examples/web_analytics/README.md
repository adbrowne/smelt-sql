# Web analytics — amplitude_id and three refinements (incremental)

A self-contained smelt example demonstrating a bronze→silver→gold pipeline over
JSON-encoded web events, with an Amplitude-style always-present `amplitude_id`
as the no-merging baseline plus three parallel refinements that progressively
merge the identity space, surfaced side-by-side in a single wide event-level
table so the algorithmic tradeoff is observable row-by-row.

Every model that has a natural time dimension is incremental, partitioned by
day. The pipeline is driven by `run_incremental.py`, which generates data and
then walks the datagen window day-by-day, invoking `smelt run` per day with a
single-day `[D, D+1)` window. Models that need a wider source read (such as
`gold/identity_forward_only`) declare their lookback via a Form B date filter
in their SQL body; the planner derives the bound automatically.

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

### Redelivery and lateness

`bronze/raw_events` is materialized as a table (rather than the project's
default view) so that `silver/events_parsed`'s `QUALIFY` window function
sees a real base table instead of an inlined view definition — see the
inline comment in
[`models/bronze/raw_events.sql`](models/bronze/raw_events.sql) for the
DuckDB binder defect this sidesteps.

`silver/events_parsed` is also where the raw feed's ingestion noise gets
cleaned up, upstream of every identity refinement below. Two independent
concerns land in one model:

- **Redelivery.** The bronze feed is at-least-once — a small fraction of
  events arrive twice, byte-identical except for `arrival_time` (the
  redelivered copy's arrival is later than the original). `QUALIFY
  ROW_NUMBER() OVER (PARTITION BY event_id ORDER BY arrival_time) = 1` keeps
  the earliest-arriving copy per `event_id` and drops the rest. Because
  `silver.events_parsed` carries `safety_overrides.allow_window_functions:
  true`, the analyzer accepts this window even though it partitions by
  `event_id` rather than the model's own `event_date` partition column —
  safe here because a redelivered duplicate is always written into the
  *same* `event_date` partition as its original (the datagen redelivery
  post-pass never moves a row across partition files), so the window never
  needs to see across a partition boundary to resolve one event_id's
  duplicates.
- **Lateness.** An event's ingestion clock (`arrival_time`) can trail its
  occurrence clock (`event_time`) by up to 3 days. The model accepts an
  event into its `event_date` partition only when `event_date BETWEEN
  CAST(arrival_time AS DATE) - INTERVAL '3 days' AND CAST(arrival_time AS
  DATE)` — a Form B filter the planner reads as a genuine 3-day lookback on
  the `bronze.raw_events` source, *derived* from the SQL rather than
  declared in YAML (`docs/specs/incremental_shapes.md` §"Derive lookback from
  the model's SQL, not from frontmatter"). Run `smelt explain
  silver.events_parsed --json` (whole-project form, not the single-model
  report) to see it surfaced: `source_bounds.bronze.raw_events` reports
  `before: "P3D", after: "PT0S"`, and `batch_safety` reports
  `bounded_safe(chunk=…,context=3d)`.

Because DELETE+INSERT is idempotent, a partition that gets rebuilt more than
once (e.g. a later daily run re-touching an earlier partition once a
previously-missing late arrival lands) converges to the same result as a
single full-window rebuild — this is exactly what
`web_analytics_dedup_matches_full_rebuild`
(`crates/smelt-cli/tests/e2e/per_partition_equivalence.rs`) asserts: zero
duplicate `event_id`s and identical `(event_id, event_date)` sets between the
day-by-day pipeline and a one-shot rebuild.

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
`silver/device_user_edges`; true recursive-CTE fixed-point convergence is a
possible future extension.

## Pipeline

```
bronze/raw_events                  (table; passthrough)
  └── silver/events_parsed         (INCR by event_date)
        ├── silver/sessions        (INCR by session_start_date; clock-anchored cut)
        │     └── gold/identity_forward_only         (INCR by session_start_date)
        ├── silver/sessions_chained (INCR by session_start_date; root-anchored,
        │     self-referential, ordered — see [Sessions](#why-sessions-spans-midnight-with-a-bounded-lookback))
        ├── silver/device_user_edges                 (refresh: keyed)
        │     ├── gold/identity_backward_fill        (view; rebuilt on query)
        │     └── gold/identity_connected_components (view; rebuilt on query)
        ├── silver/events_enriched (INCR by event_date) ← silver/sessions, silver/sessions_chained
        │     (event-grain: dual session_id/utm_campaign pairs, one per cut rule)
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
  [`functions/sessionize.sql`](functions/sessionize.sql) (clock-anchored cut, bounded cross-midnight sessionization — see [Sessions](#why-sessions-spans-midnight-with-a-bounded-lookback))
- [`models/silver/sessions_chained.sql`](models/silver/sessions_chained.sql)
  (root-anchored cut, self-referential, ordered execution)
- [`models/silver/device_user_edges.sql`](models/silver/device_user_edges.sql)
- [`models/silver/events_enriched.sql`](models/silver/events_enriched.sql) (see [Event-grain enrichment](#event-grain-enrichment))
- [`models/gold/identity_forward_only.sql`](models/gold/identity_forward_only.sql)
- [`models/gold/identity_backward_fill.sql`](models/gold/identity_backward_fill.sql)
- [`models/gold/identity_connected_components.sql`](models/gold/identity_connected_components.sql)
- [`models/gold/eventstream_with_identity.sql`](models/gold/eventstream_with_identity.sql)
- [`models/marts/daily_active_users_by_method.sql`](models/marts/daily_active_users_by_method.sql)
- [`models/marts/identity_method_comparison.sql`](models/marts/identity_method_comparison.sql)

The generated tutorial page
([Generated tutorial page](#generated-tutorial-page) below) walks the
sessions/sessions_chained split in full — same 30-minute inactivity +
platform-boundary rule and 5-minute first-touch attribution in both tables,
differing only in where their session-length cap's phase comes from (the
clock vs. the session's own root), which is exactly what flips
`silver.sessions_chained`'s execution from window-independent/parallel to
self-referential/ordered.

## Incremental shape

Seven models are incremental, one is a cumulative aggregate (`refresh: keyed`), one is a
plain (non-incremental) table, and the rest are views.

| Model                                         | Materialization     | Partition column     |
|-----------------------------------------------|---------------------|----------------------|
| `silver/events_parsed`                        | INCR table          | `event_date`         |
| `silver/sessions`                             | INCR table (window-independent) | `session_start_date` |
| `silver/sessions_chained`                     | INCR table (self-referential, ordered) | `session_start_date` |
| `silver/device_user_edges`                    | table + refresh: keyed | (driven by source)   |
| `silver/events_enriched`                      | INCR table          | `event_date`         |
| `gold/identity_forward_only`                  | INCR table          | `session_start_date` |
| `gold/eventstream_with_identity`              | INCR table          | `event_date`         |
| `marts/daily_active_users_by_method`          | INCR table          | `event_date`         |
| `bronze/raw_events`                           | table               | —                    |
| `gold/identity_backward_fill`                 | view                | —                    |
| `gold/identity_connected_components`          | view                | —                    |
| `marts/identity_method_comparison`            | view                | —                    |

### Why device_user_edges is cumulative

The two global identity algorithms (backward_fill, connected_components) need
the cumulative `(device, user)` edge set to produce correct per-device
elections and clusters. `silver/device_user_edges` uses
`materialization: table` + `refresh: keyed` so each daily run only
aggregates that day's signed-in events and merges them into the running
cumulative state via the SQL aggregator → cross-partition combiner mapping
(COUNT→SUM, MIN→MIN, MAX→MAX). The backward_fill and connected_components
models read this cumulative table directly as a lookup; they remain views and
are re-evaluated on every query against `gold/eventstream_with_identity`.

`marts/identity_method_comparison` is a 3-row global aggregation with no time
dimension, so no `partition_column` exists; it stays a view too.

### Why the driver passes a single-day window

`gold/identity_forward_only` is incremental by `session_start_date`. A session
that started yesterday but received its latest signed-in event today should
have its mapping refreshed. The model declares this need via an explicit Form B
date filter on the `events_parsed` join:

```sql
WHERE e.event_date
    BETWEEN s.session_start_date - INTERVAL '1 day'
        AND s.session_start_date + INTERVAL '1 day'
```

The planner reads this filter and widens the `events_parsed` source read by 1
calendar day automatically, so the driver only needs to pass `[D, D+1)` per
iteration. The filter catches:

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

### Why `sessions` spans midnight with a bounded lookback

`silver/sessions` reconstructs sessions across midnight while reading and
writing only a bounded window — the property that keeps per-day cost flat as
history grows. Its cap is **clock-anchored**: a session is cut at the
midnight ending day D iff it has an event in the first 30 minutes of day D
(`[D 00:00, D 00:30)`). A session rooted before `00:30` dies at its own
day's end; a session rooted at or after `00:30` may cross one midnight but
always dies at the *second* one (the crossing gap is itself under the
30-minute inactivity threshold, so the crossing session always has an event
before `00:30` of the new day). Every session therefore spans at most two
calendar days — under 48 hours — computable purely from the event's own
timestamp, with no memory of the session's own history required. That is
what keeps `silver/sessions` window-independent: any partition can build in
any order, including in parallel, even under a device that never idles for
the full 30 minutes (see [the never-idle comparison in the generated
tutorial](../../docs-site/docs/examples/web-analytics-maintenance.md#same-rule-three-fates-the-never-idle-comparison)
for what a clock-anchored cap does differently from an unbounded-history
cap in that scenario). `silver/sessions_chained` right below cuts on a
different signal — the session's own root age rather than the clock — which
is what forces it to be self-referential and ordered instead.

The sessionization lives in the reusable `smelt.functions.sessionize` function.
Each `LAG`/`MAX OVER` in its body carries a
`RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW` frame (`max_lookback`):
the planner derives a bound from those frames — bound derivation runs on the
*expanded* SQL, so a frame declared inside the function body is honored —
and widens the `silver/events_parsed` read accordingly, so a session whose
events straddle midnight is reconstructed as **one** row instead of being
split at the partition boundary. `sessions.sql` restates the ≤2-day span as
an explicit, checkable `HAVING MAX(event_ts) - MIN(event_ts) < INTERVAL '2 days'`
assertion, so the cap is visible and verifiable in the emitted SQL rather
than only implicit in the window-frame mechanics that happen to enforce it.

Because the partition column `session_start_date` is *derived* and can skew
earlier than the events that update it (a session that started yesterday
gains events today), the model carries a Form B filter
(`event_date BETWEEN session_start_date AND session_start_date + INTERVAL '1 day'`)
that widens the **write** window for a `[D, D+1)` run to `[D-1, D+2)` —
half-open, covering partitions `D-1`, `D`, and `D+1`. The planner's
DELETE+INSERT deletes the same widened window the INSERT writes, so
re-running consecutive days stays idempotent (no duplicate rows in the
lookback partition).

Session identity is `(device_id, session_start_ts)` — stable across run windows,
so a session reprocessed in a different window keeps the same `session_id`.
`silver/sessions_chained` uses the identical identity scheme.

### Session campaign attribution

`silver/sessions` also tags each session with `utm_campaign`: the earliest
non-NULL campaign among the session's own events within the first 5 minutes
of session start, `ARG_MAX(utm_campaign, -epoch_us(event_ts)) FILTER (WHERE
utm_campaign IS NOT NULL AND event_ts <= session_start_ts + INTERVAL '5
minutes')` — a MIN_BY-style pick expressed with `ARG_MAX`, the recognized
aggregate this codebase's type inference supports (see
`gold/identity_backward_fill.sql`'s `arg_max` usage), keyed by the *negated*
timestamp so the value returned is the one at the smallest `event_ts`. A
campaign arriving later in a long-running session never attributes — only the
first 5 minutes count, mirroring first-touch campaign attribution in
production analytics pipelines. Because the 5-minute attribution window sits
well inside the clock-anchored cap, it never needs a wider source read than
the sessionization already declares.

### Event-grain enrichment

`silver/events_enriched` demonstrates a maintenance-plan creation cell over
**three** maintained-model upstreams in the same body, rather than one. It
joins every `silver/events_parsed` row to both its `silver/sessions` row
(clock-anchored, 1-day cap) and its `silver/sessions_chained` row
(root-anchored, 2-day cap), the same join shape `gold/eventstream_with_identity`
uses for its own single upstream, and projects **two** id/campaign pairs —
`session_id`/`session_utm_campaign` (primary; what the gold identity models
consume) and `session_id_chained`/`session_utm_campaign_chained` (additive) —
alongside the event's own raw `utm_campaign`, so all of it can be compared
row-by-row. Per-event divergence between the two session ids is directly
queryable.

`smelt explain silver.events_enriched` shows one creation cell per model
upstream, each carrying the clamp derived from that edge
(`docs/specs/incremental_models.md` §"Upstream model edges"):

```
  - group {*} on trigger NewData { source: "silver.events_parsed" }
      scan clamps:
        - source=silver.events_parsed column=event_date before=Seconds(0) after=Seconds(0)
  - group {*} on trigger NewData { source: "silver.sessions" }
      scan clamps:
        - source=silver.sessions column=session_start_date before=Seconds(172800) after=Seconds(172800)
  - group {*} on trigger NewData { source: "silver.sessions_chained" }
      scan clamps:
        - source=silver.sessions_chained column=session_start_date before=Seconds(172800) after=Seconds(172800)
```

`events_parsed`'s edge is a direct 1:1 read (`event_date` is this model's own
partition column, unfiltered against that upstream) — a `Bounded(0,0)` clamp.
`sessions`'s and `sessions_chained`'s edges both carry a ±2-day clamp — each
the downstream's own join-window Form B filter (±1 day for `sessions`, ±2
days for `sessions_chained`) composed with that upstream's own derived skew
(`sessions`'s own ±1-day Form B relation), so the wider of the two dominates
for both. Because no edge write-rebases the output partition column, a run
touching one `event_date` partition only ever writes that partition here:
running one additional arrival day changes exactly its own `event_date`
partition and leaves every previously-written partition byte-identical
(`crates/smelt-cli/tests/e2e/per_partition_equivalence.rs
::web_analytics_events_enriched_narrow_update`).

## Run locally

```bash
python run_incremental.py --scale-factor 0.01
```

Default behaviour: wipe `target/dev.duckdb`, regenerate data via
`smelt-datagen`, materialise the raw source tables via `setup_sources.sql`,
then loop day-by-day across the datagen window (2026-03-19 .. 2026-05-17 by
default, 60 days). Each iteration invokes `smelt run --event-time-start D
--event-time-end D+1`; models with Form B filters (e.g.
`gold/identity_forward_only`) have their source reads widened automatically by
the planner. After the loop the script runs `smelt test` so all inline
invariants are checked against the final cumulative state.

Per-iteration output:

```
[datagen] 0.2s
[setup] 0.1s
[day  1/60] 2026-03-19  smelt run [start=2026-03-19 end=2026-03-20]  0.2s
[day  2/60] 2026-03-20  smelt run [start=2026-03-20 end=2026-03-21]  0.2s
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
  produce the expected session_id assignments on a mocked event sequence for
  `silver/sessions`, plus the clock-anchored deadline (early-root sessions cut
  at their own day's end; late-root sessions crossing one midnight cut at
  the second). Session campaign attribution and the explicit clock-anchored
  cap are covered end-to-end by
  `crates/smelt-cli/tests/e2e/per_partition_equivalence.rs::web_analytics_session_attribution_matches_full_rebuild`,
  `crates/smelt-cli/tests/e2e/cross_midnight_rebase.rs::never_idle_device_yields_one_session_per_day`,
  and by `verify_incremental_equivalence.py`'s session-attribution assertion.
- [`tests/session_boundary_chained_invariants.test.sql`](tests/session_boundary_chained_invariants.test.sql) —
  mirrors the same gap/platform fixtures against `silver/sessions_chained`
  (identical outcomes — the two tables share the gap rule) plus a fixture
  pinning where the root-anchored cut diverges from the clock-anchored one.
  The self-referential model's `Ordered` planner verdict, its from-scratch
  bootstrap, and the never-idle root-anchored cadence are covered end-to-end
  by `crates/smelt-cli/tests/e2e/cross_midnight_rebase.rs
  ::chained_run_is_refused_or_ordered_never_parallel` and
  `::chained_never_idle_device_yields_one_session_per_two_days`.
- [`tests/enrichment_dual_session_invariants.test.sql`](tests/enrichment_dual_session_invariants.test.sql) —
  asserts `silver/events_enriched` carries both session id/campaign pairs,
  that they agree whenever neither table's cap fires, and diverge only on
  the fixture where the two caps disagree.
- [`tests/device_user_edges_per_day_invariants.test.sql`](tests/device_user_edges_per_day_invariants.test.sql) —
  asserts the cumulative aggregation shape (one row per `(device, user)`,
  `event_count` = SUM across days, `first_seen` = MIN, `last_seen` = MAX) and
  that anonymous events are excluded.
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
- `silver/events_enriched`'s event-grain enrichment (join correctness,
  per-partition equivalence, and the model-upstream creation-cell narrow
  update) is covered end-to-end by
  `crates/smelt-cli/tests/e2e/per_partition_equivalence.rs
  ::web_analytics_events_enriched_matches_full_rebuild` and
  `::web_analytics_events_enriched_narrow_update`,
  `crates/smelt-cli/tests/explain_model.rs
  ::events_enriched_shows_creation_cells_for_both_model_upstreams`, and by
  `verify_incremental_equivalence.py`'s events_enriched assertion.

## Known divergences

### What makes the `sessionize` function work as an incremental dependency

`silver/sessions` calls the reusable `smelt.functions.sessionize` function, which
encapsulates the windowing. Three framework behaviors make that safe inside an
incremental model:

- **Bound derivation reads the expanded SQL.** The `RANGE BETWEEN INTERVAL`
  frames live in the function body; the run pipeline expands the function before
  deriving source bounds (`SqlCompiler::expand_function_calls`), so the 2-day
  `max_lookback` is honored rather than silently defaulting to zero.
- **Bounded-frame windows are admitted regardless of `PARTITION BY`.** The window
  partitions by `device_id`, which does not include the model's
  `partition_column` (`session_start_date`); it is admitted because each frame is
  a bounded `RANGE BETWEEN INTERVAL` (see
  `docs/specs/incremental_shapes.md` § "Batch safety classification").
- **The write window covers the lookback partition.** The outer Form B filter
  (`WHERE event_date BETWEEN session_start_date AND session_start_date + INTERVAL '1 day'`)
  references `session_start_date`, a column produced by the function — resolvable
  because column references through a `TableExpr` function are inferred, and a
  typed literal like `INTERVAL '1 day'` is no longer mistaken for a column.

## Generated tutorial page

The lateness/redelivery/attribution/enrichment slice of this pipeline (the
maintenance-plan machinery in particular) has a generated walkthrough at
[`docs-site/docs/examples/web-analytics-maintenance.md`](../../docs-site/docs/examples/web-analytics-maintenance.md),
rendered by [`generate_tutorial.py`](generate_tutorial.py) from real
`smelt explain --show-sql` / `smelt rebuild --dry-run` output — the
embedded SQL is never hand-pasted. Regenerate it after changing any of the
models it names:

```bash
python3 examples/web_analytics/generate_tutorial.py           # regenerate
python3 examples/web_analytics/generate_tutorial.py --check   # verify freshness (no write)
```

`crates/smelt-cli/tests/tutorial_freshness.rs` is the CI-enforced drift
gate: it re-derives the same SQL in-process (no datagen, no backend) and
fails if the committed page has drifted from the emitters' current output.

## How this example was built

Multi-session implementation tracked in
[`docs/plans/20260517-web-analytics-example.md`](../../docs/plans/20260517-web-analytics-example.md).
The redelivery/lateness dedup, session campaign attribution, and event-grain
enrichment extensions are tracked in
[`docs/plans/20260710-web-analytics-maintenance-demo.md`](../../docs/plans/20260710-web-analytics-maintenance-demo.md).

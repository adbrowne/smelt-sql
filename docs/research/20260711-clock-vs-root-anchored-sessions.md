# Clock-anchored vs root-anchored sessionization — redesigning the web-analytics sessions example

**Date:** 2026-07-11
**Status:** Design approved (interactive session with Andrew); implementation plan to follow.
**Replaces:** the current `silver.sessions` frame-cap design in `examples/web_analytics/`.

## Problem

The current `silver.sessions` enforces its 1-day max-session-length cap via
`RANGE BETWEEN INTERVAL '1 day' PRECEDING` window frames plus a
`COALESCE(MAX(_boundary_ts) OVER (...), event_ts)` fallback. Under a
never-idle input (one device firing an event every 29 minutes for two
months), the fallback degenerates: the first day forms one capped session,
and **every subsequent event becomes its own single-event session** — the
trailing frame never again contains a boundary row, so each row falls back
to itself as a root. Session/visit counts are a headline web-analytics
metric; a design that inflates them ~50×/day in the presence of one weird
user is unacceptable, even in an example. (The docs-site's "session that
spans two midnights" section documents this behaviour in miniature: 58
events in the capped session, then orphan single-event sessions.)

The deeper issue is worth teaching rather than hiding: **any bounded
(incrementally maintainable) sessionizer must cut somewhere**, and *where
the cut's phase comes from* — the session's own history vs the clock —
decides the model's execution properties.

## Design

Two session tables replace the current one. Both keep the 30-minute
inactivity + platform-boundary rule and the 5-minute first-touch campaign
attribution. Both cap sessions at roughly two days. They differ in one
decision: where the cap's phase comes from.

### `silver.sessions` — clock-anchored cut (window-independent)

**Rule:** a session is cut at the midnight ending day D **iff it contains
an event in the first 30 minutes of day D** (`[D 00:00, D 00:30)`).

Consequences (all proven in the design discussion):

- A session rooted before 00:30 dies at its own day's end.
- A session rooted at or after 00:30 may cross one midnight — but a
  crossing session *always* has an event before 00:30 of the new day (the
  crossing gap is ≤ 30 min), so it always dies at the **second** midnight.
- Therefore every session spans ≤ 2 calendar days and < 48h.
- Closed form: root r's deadline is `end_of_day(date(r))` if
  `time(r) < 00:30`, else `end_of_day(date(r) + 1 day)`. Forced roots (first
  chain event past a deadline) always land before 00:30 of their day, so
  the cascade self-stabilizes at one cut per day — no confetti.
- Never-idle user: ~1 session per day after the initial one, deterministic.

**Why it's window-independent:** the cut's phase is midnight — computable
from the event timestamp alone, no memory. Every row's session assignment
is a pure function of a bounded (2-day) trailing window of *source* events.
Partitions build in any order, in parallel. Form B relation:
`event_date BETWEEN session_start_date AND session_start_date + INTERVAL '1 day'`
(the partition column skews earlier by at most 1 day).

Implementation: rewrite the `sessionize` transparent function around the
closed-form deadline rule (two-level windowing over a 2-day frame; the
forced-root cascade needs care — see Risks).

### `silver.sessions_chained` — root-anchored cut (self-referential, ordered)

**Rule:** a day-D event continues an open session only if that session
**rooted less than 2 days ago**; otherwise it strikes a new root.

The cutoff's phase is inherited from the session's own root — for the
never-idle user, sessions roll at ~2pm on alternating days, and that phase
was decided months back. No bounded read of the source can recover it: the
model must read **its own prior output** (which open session am I in; when
did it root?). Self-lookback: 2 days
(`smelt.silver.sessions_chained` partitions D−2, D−1).

- One row per session, partition grain on `session_start_date`, inline SQL
  (self-reference + state inheritance doesn't factor into a reusable
  function naturally).
- The self-read is backward-bounded → the window-independence proof
  returns `Ordered`; the runtime builds windows sequentially in temporal
  order (`smelt-runtime/src/windowing.rs` sequential single-partition
  batches). Backfills cannot be parallelised — that is the point.
- A run for day D rewrites partitions D−2..D (same ±-day Form B rebase
  shape as the clock table).
- Never-idle user: ~1 session per 2 days, phase-locked to the original
  root.

### Enrichment — `silver.events_enriched` carries both

- Keeps `session_id` / `utm_campaign` from `silver.sessions` — the
  **primary**; the gold identity models continue to consume these, keeping
  the parallel table on the hot path.
- Adds `session_id_chained` / `utm_campaign_chained` from
  `silver.sessions_chained`.
- Per-event divergence between the two id columns is directly queryable —
  the comparison surface the docs narrate.

### The lesson (docs narrative)

Same gap rule, three fates for the never-idle user:

| Design | Sessions emitted | Execution |
|---|---|---|
| Root-anchored cap (`sessions_chained`) | ~1 per 2 days | ordered, sequential |
| Clock-anchored cap (`sessions`) | ~1 per day | window-independent, parallel |
| Old frame-cap + COALESCE fallback | ~50 per day (confetti) | "parallel", wrong |

Two near-identical business rules; one design decision (cap phase from
history vs from the clock) flips the execution property. The old design is
demoted to a narrated anti-example — what goes wrong when the cap lives in
a window frame with a permissive fallback.

## Tests

- Update `examples/web_analytics/tests/session_boundary_invariants.test.sql`
  for the new rules.
- New invariant test: both tables count every event exactly once, sessions
  never overlap, and the tables agree on every event except where one cap
  fired differently.
- Update the synthetic-chain e2e tests
  (`crates/smelt-cli/tests/e2e/cross_midnight_rebase.rs`,
  `per_partition_equivalence.rs`) to the new semantics; add the two-month
  never-idle fixture asserting chained ≈ 1/2 days, clock ≈ 1/day, no
  single-event confetti anywhere, and day-by-day replay ≡ full rebuild for
  both tables (for the chained table, replay in order).
- Docs: rewrite the sessions + enrichment sections of
  `docs-site/docs/examples/web-analytics-maintenance.md` around the
  two-table arc; regenerate all `smelt-generate` blocks; cross-link the
  ordered-execution section of the incremental-models guide.

## Risks (validate early, in this order)

1. **Ordered path × Form B rebase** — the runtime's sequential
   single-partition batches composing with the derived output window's
   write rebase. This example is the first real consumer; a gap here
   becomes a small framework fix first.
2. **Planner admission of the 2-day backward self-read** — the proof
   requires no *forward* read (`after == Seconds::ZERO`); a 2-day backward
   bound should pass, but is untested by a real example.
3. **Clock-rule window SQL** — the forced-root cascade needs a careful
   two-level window formulation; get it red-green against the never-idle
   fixture before wiring it into the example.

## Rejected alternatives

- **Session fragments (one row per session-day, uncapped exact semantics).**
  Exact, but still admits unbounded sessions; rejected in favour of a
  root-anchored cap because bounding *both* tables makes the lesson about
  where the bound's phase comes from, not about whether to bound.
- **Full-recompute exact table** — trivially exact but doesn't need its own
  history, so it teaches delegation cost, not ordered execution.
- **Keyed-grain running aggregate** — real, but drags a third refresh mode
  into an example about the partition grain.
- **Hard cut of all sessions at midnight** — fully local (zero lookback)
  but splits every genuine cross-midnight session; count inflation of the
  common case rather than the weird one.

# Phase 2 — `examples/github_activity/` green on DuckDB

**Outcome:** `docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md`
**Serves criteria:** 4 (DuckDB leg, in CI), and sets up 6/7 by fixing the model set
**Driver:** Claude-executable end to end — no warehouse, no credentials
**Spec delta:** none. Every construct used is already specced (`sources.md`,
`timeseries.md`, `incremental_models.md`); if a model needs surface that is not, that is a
finding for the punch-list, not a spec change made here.

## Objective

The four spine models exist, compile, and run to completion against DuckDB over the
committed Parquet sample, with zero LSP diagnostics, wired into per-PR CI. Nothing about
this phase touches BigQuery.

The point is not that four models exist. It is that by the end of the phase the model set
is **frozen**, so phases 5–8 compare two targets over a fixed thing rather than a moving
one — and that the DuckDB leg genuinely exercises dedup, which needs deliberate work
(§"Redelivery is not free").

## What the sample forces

Phase 1's findings change three things the research doc assumed:

- **`payload` is not in the sample**, so `bronze.events` is a typed passthrough *without*
  the JSON payload. The research doc's silver fan-out (`push_events`, `pr_events`, …) is
  already out of scope, and this is why: nothing downstream can extract from a payload
  that was never landed. Widening the projection is a `sample.sql` edit plus a fixture
  regeneration, and it is not done here.
- **`id` is STRING.** The dedup key is textual. Worth stating because a `BIGINT` guess
  would type-check against nothing until the first live run.
- **The feed has no duplicate ids of its own.** See below.

## Redelivery is not free

`silver.events_deduped` exists because the *loader* is at-least-once. GitHub Archive is
not: 64,313 rows, 64,313 distinct ids. If the DuckDB leg loads the fixture once and runs,
the dedup is a no-op that would pass every gate while being completely untested — and it
would keep passing right up until the live BigQuery leg replayed a window in phase 6.

So the DuckDB leg reproduces the loader's redelivery **by construction**, in the shape the
real loader will have: the loader re-runs over a day range that overlaps the previous
run's, so events near a boundary land twice. `setup_sources.sql` therefore loads day `D`
as `[D - overlap, D + 1 day)` rather than `[D, D + 1 day)`, with `overlap` a named
constant, and the duplicate count is asserted non-zero before the models run. A test that
finds zero duplicates is a failing test, not a lucky one.

The overlap must be a **declared** number that the phase-4 loader then matches, for the
same reason `sample.sql` is pinned: if the two legs redeliver differently they are no
longer comparable.

## The four models

| Model | Shape | Why it is in the spine |
|---|---|---|
| `bronze.events` | typed passthrough of the source, clocked on `created_at` | the source contract made concrete; the first thing a compile refusal will hit |
| `silver.events_deduped` | dedup on `id` over the derived late-arrival window | the at-least-once shape; the one model whose correctness the redelivery above exists to test |
| `silver.actor_sessions` | gap-based sessionization per `actor_id`, 30-minute gap | the web-analytics session analogue in a different domain; the shape that crosses day boundaries and so stresses window-forward reads |
| `marts.daily_active_contributors` | per day: distinct actors, sessions, events | makes the silver output visible as a product rather than only as a technique |

Three choices inside that table are judgement calls rather than derivations, and are the
cheapest things for a reviewer to overrule:

- **The 30-minute gap** is borrowed from `examples/web_analytics/` so the two examples can
  be compared directly, which the research doc asked for. Nothing about GitHub activity
  argues for 30 minutes specifically.
- **The lateness window** is derived from how the *shards* behave, not declared from
  taste: GitHub Archive publishes hourly and an event near an hour boundary lands in the
  next file. Task 2 measures the actual observed lag in the fixture and the declaration
  follows the measurement.
- **`daily_active_contributors` over `repo_leaderboard`.** The sample skews hard to
  newly-created bot repos (92% `PushEvent`, median repo has one event), so a leaderboard
  would be a mart whose output is mostly noise. A daily-actives count degrades gracefully
  under that skew.

## Tasks

1. `smelt.yml` — one `duckdb` target, `paths: [models]`, following
   `examples/web_analytics/smelt.yml`. Materializations named per model.
2. Measure the fixture's real late-arrival lag (`created_at` vs the shard day it landed
   in) and pin `mutation_profile.lateness` to it. Record the measurement in the phase
   summary — a declared window nobody measured is exactly the drift
   `feedback_derive_dont_declare` warns about.
3. `models/sources/raw/github_events.yml` — `columns:` from the fixture's real Parquet
   schema (`id` VARCHAR, `created_at` TIMESTAMP, `actor_id`/`repo_id` BIGINT, …),
   `timeseries:` on `created_at`, `mutation_profile: {kind: append_only, lateness: <task
   2>, redelivery: at_least_once}`, `unique_key: [id]`, `retention:` matching the loader's
   N-day trim.
4. `setup_sources.sql` + a day-by-day replay driver modelled on
   `examples/web_analytics/run_incremental.py`, implementing the overlapping load above.
5. The four models.
6. Assert the fixture and the models: duplicates exist after replay and are gone after
   `silver.events_deduped`; sessions do not span the gap; a session that crosses midnight
   stays one session.
7. Wire the example into per-PR CI beside `web_analytics`.

## Tests (red first)

- `cargo test -p smelt-cli --test example_diagnostics` — zero diagnostics for the new
  workspace. Red before the models exist.
- `cargo test -p smelt-lsp --test example_workspaces` — the same via the real LSP backend,
  which catches the asymmetric-discovery bugs the Salsa-direct test misses.
- A replay test that asserts **non-zero duplicates land** and that
  `silver.events_deduped` emits exactly `count(DISTINCT id)` rows. This is the test that
  fails if the overlap is ever quietly dropped.
- A sessionization test over a hand-picked actor spanning midnight: one session, not two.
- Full-refresh equivalence over the replayed days (`verify_incremental_equivalence.py` is
  the precedent). This is criterion 7's DuckDB half, banked early because it costs nothing
  here.

## Verification gate

- `bash .claude/scripts/verify-phase.sh` green.
- `smelt build` and `smelt test` clean in `examples/github_activity/` from a wiped target.
- The replay driver runs the full 30 days and the equivalence check passes at every step.
- The duplicate assertion is observed non-zero — reported as a number in the summary, not
  as "passed".

## Commit message

```
feat(examples): github_activity runs end to end on DuckDB

The four spine models — bronze.events, silver.events_deduped,
silver.actor_sessions and marts.daily_active_contributors — build and test
against the committed Parquet sample with no warehouse, wired into per-PR
CI beside web_analytics.

The DuckDB leg reproduces the loader's at-least-once redelivery on purpose:
GitHub Archive has no duplicate event ids of its own, so a single load
would leave silver.events_deduped a no-op that passes every gate until the
live leg first replays a window. setup_sources.sql loads each day over a
range overlapping the previous one, exactly as the phase-4 loader will, and
a test asserts the duplicates are really there before asserting they are
gone.

The source's lateness is measured from the fixture's own shard lag rather
than declared from taste.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
```

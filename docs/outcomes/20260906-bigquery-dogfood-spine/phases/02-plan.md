# Phase 2 — `examples/github_activity/` green on DuckDB

**Outcome:** `docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md`
**Serves criteria:** 4 (DuckDB leg, in CI), and sets up criteria 6 and 7 by putting the workspace, the source contract and the replay driver in place
**Driver:** Claude-executable end to end — no warehouse, no credentials
**Spec delta:** none. Every construct used is already specced (`sources.md`,
`timeseries.md`, `incremental_models.md`); if a model needs surface that is not, that is a
finding for the punch-list, not a spec change made here.

## Objective

The four spine models exist, compile, and run to completion against DuckDB over the
committed Parquet sample, with zero LSP diagnostics, wired into per-PR CI. Nothing about
this phase touches BigQuery.

The point is not that four models exist. It is that the workspace, the source contract and
the replay driver are in place, so phases 3 and 4 add models to a pipeline that already
runs rather than to a design — and that the DuckDB leg genuinely exercises dedup, which
needs deliberate work (§"Redelivery is not free"). The model set is not frozen here;
phase 5 freezes it, once succession and the fan-out have landed.

## What the sample forces

Phase 1's findings change three things the research doc assumed:

- **`payload` is not in the sample**, so `bronze.events` is a typed passthrough *without*
  the JSON payload. The silver fan-out (`push_events`, `pr_events`, …) cannot be built on
  it: nothing downstream can extract from a payload that was never landed. Widening the
  projection is a `sample.sql` edit plus a fixture regeneration, and **phase 4 does it** —
  not this phase. Nothing built here may assume `payload` exists, and nothing here may
  make adding it a breaking change to the source declaration.
- **`id` is STRING.** The dedup key is textual. Worth stating because a `BIGINT` guess
  would type-check against nothing until the first live run.
- **The feed has no duplicate ids of its own.** See below.

## Redelivery is not free

`silver.events_deduped` exists because the *loader* is at-least-once. GitHub Archive is
not: 64,313 rows, 64,313 distinct ids. If the DuckDB leg loads the fixture once and runs,
the dedup is a no-op that would pass every gate while being completely untested — and it
would keep passing right up until the live BigQuery leg replayed a window in phase 8.

So the loader **deliberately redelivers**: each day's load re-appends a fixed slice of the
*previous* day's rows (human decision of 2026-09-07). The black box is declared
at-least-once, the upstream feed supplies no redelivery of its own, so the black box
supplies it — honestly and by declaration, rather than by hoping for an incidental
duplicate that the measurements say does not exist.

**Why the previous day rather than an overlapping window.** An earlier draft of this plan
had the loader re-run over a range overlapping the previous run's, so events near a
boundary landed twice. That shape has an escape hatch: both copies arrive in the same load
and the same day partition, so a partition-local `QUALIFY ROW_NUMBER()` removes them with
**zero lookback**, and the dedup is exercised only in its most trivial case. Redelivering
*yesterday's* rows closes it. The redelivered row's `created_at` sits outside the current
window, so `silver.events_deduped` sees both copies only if it carries a real lookback in
its SQL that the planner derives a widened read from — and a lookback wrong in either
direction leaves duplicates alive.

Note what does **not** do this work: `mutation_profile.lateness` is orchestration-only
(`sources.md` §Semantics — "never widens a scan, never gates a probe, never changes
emitted SQL"), and lookback is derived from the model's SQL, never declared in frontmatter
(`incremental_shapes.md` §"Derive lookback from the model's SQL"). The lateness
declaration in task 3 is a true statement about the feed; it is not the mechanism.

Three properties the redelivery rule must have:

- **Deterministic, not sampled.** Both legs must redeliver the *same* rows or criterion
  6's parity check diffs noise instead of behaviour. So no `RAND()`, and no
  `FARM_FINGERPRINT` (which DuckDB has no equivalent of). Event ids are numeric strings,
  so `MOD(CAST(id AS BIGINT), 50) = 0` evaluates identically on DuckDB and GoogleSQL. The
  rule joins `sample.sql` as pinned contract — phase 7's loader reproduces it verbatim.
- **2%, not 0.1%.** The fixture averages ~2,144 rows/day, so 0.1% is ~2 redelivered rows
  per day and ~64 across the run — thin enough that a lookback wrong by one day could pass
  on luck. 2% is ~43/day and ~1,290 total, and matches the ~2% duplicate rate
  `examples/web_analytics/` carries, so the two examples stay comparable. A deliberate
  duplicate rate is a test knob, not a property of a real failure mode, so there is no
  realism to trade away.
- **Exactly the previous day, not the last N days.** The required lookback is then exactly
  two days and an error in *either* direction fails. A spread over several days lets a
  too-wide lookback pass.

A consequence to state rather than let someone rediscover: `raw.github_events` now
permanently contains synthetic duplicates, and `bronze.events`, being a passthrough,
carries them. That is correct — it is what makes the bronze→silver boundary mean anything
— but it goes in the README beside the bot-repo skew note so it is never diagnosed as a
bug.

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
4. Pin the redelivery rule beside `sample.sql` — `MOD(CAST(id AS BIGINT), 50) = 0` over the
   previous day — as contract, not as a detail of the driver.
5. `setup_sources.sql` + a day-by-day replay driver modelled on
   `examples/web_analytics/run_incremental.py`, loading day `D` as day `D` plus the
   redelivered slice of `D - 1`.
6. The four models. `silver.events_deduped` carries its two-day lookback **in its SQL**,
   since that is the only thing that widens the read.
7. Assert the fixture and the models: the redelivered rows are present and counted before
   dedup, and `silver.events_deduped` emits exactly `count(DISTINCT id)`; sessions do not
   span the gap; a session that crosses midnight stays one session.
8. Correct `examples/github_activity/README.md`: its dedup note still describes the
   superseded overlapping-window shape ("the *loader* replays overlapping windows"). State
   the previous-day redelivery rule instead, and add the synthetic-duplicates consequence
   beside the existing bot-repo skew note.
9. Wire the example into per-PR CI beside `web_analytics`.

## Tests (red first)

- `cargo test -p smelt-cli --test example_diagnostics` — zero diagnostics for the new
  workspace. Red before the models exist.
- `cargo test -p smelt-lsp --test example_workspaces` — the same via the real LSP backend,
  which catches the asymmetric-discovery bugs the Salsa-direct test misses.
- A replay test that asserts the redelivered rows **land and are counted** before dedup,
  and that `silver.events_deduped` emits exactly `count(DISTINCT id)` rows. This is the
  test that fails if the redelivery rule is ever quietly dropped, and — because the
  duplicates are a day old — the one that fails if the derived lookback is wrong.
- A lookback-boundary test: narrow `silver.events_deduped`'s lookback to one day and
  confirm duplicates **survive**. A dedup test that cannot be made to fail on demand is
  not evidence that the lookback is doing anything.
- A sessionization test over a hand-picked actor spanning midnight: one session, not two.
- Full-refresh equivalence over the replayed days (`verify_incremental_equivalence.py` is
  the precedent). This is criterion 7's DuckDB half, banked early because it costs nothing
  here.

## Verification gate

- `bash .claude/scripts/verify-phase.sh` green.
- `smelt build` and `smelt test` clean in `examples/github_activity/` from a wiped target.
- The replay driver runs the full 30 days and the equivalence check passes at every step.
- The duplicate assertion is observed non-zero — reported as a number in the summary, not
  as "passed" — and the deliberately-narrowed-lookback case is observed to fail.

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
range overlapping the previous one, exactly as the phase-7 loader will, and
a test asserts the duplicates are really there before asserting they are
gone.

The source's lateness is measured from the fixture's own shard lag rather
than declared from taste.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
```

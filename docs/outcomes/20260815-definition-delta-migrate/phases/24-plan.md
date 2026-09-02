# Phase 24 — Resolve open-ended propagated windows so `examples/web_analytics` runs end to end

## Objective

`smelt run --since-upstream` over the whole unfiltered `examples/web_analytics` workspace now
propagates cleanly (phases 21–23) but dies at execution: phase 22's day-unrolled self-edge
schedules `silver.sessions_chained` and `silver.events_enriched` with `start: Some(_), end: None`,
and `smelt-runtime`'s `parse_run_window` rejects that as "Both start and end must be provided
together (or neither)". This phase resolves an open-ended propagated interval to a finite run
window at the scheduling boundary, so the whole-workspace `--since-upstream` run completes —
directly advancing the outcome's end-to-end `examples/web_analytics` success criterion.

Verified live before planning (`smelt run --since-upstream --source sources.raw.events --landed
2026-08-01..2026-08-05 --dry-run` from `examples/web_analytics`): the dirty-set report and all 7
`RUN` lines print, and the *only* hard error is the guard above, on `silver.sessions_chained`.

## Spec delta (do this first)

`docs/specs/incremental_models.md` §"Time-unrolled self-edges" — after the **Forward (dirt)**
bullet, add a short paragraph: the dirty set stays open-ended (`[a, →)`) because that is the
honest statement of what is dirty, but a *run* needs a closed region, so at scheduling time an
open-ended interval is resolved to `[a, today + 1 day)` — today's partition inclusive, against
the same `now` the propagation planner already takes. An open-ended interval whose start is
itself after today is a fail-loud refusal naming the model (nothing to run, and a silently empty
window would be wrong-and-quiet). Point out that the printed dirty-set report still renders
`[a, →)`, and the per-run log line reports the resolved window.
`docs-site/docs/reference/cli.md` §"Forward propagation with `--since-upstream`" gets one
sentence mirroring the resolution rule.

## Tests (red first)

Pure unit tests — `crates/smelt-runtime/tests/since_upstream_propagation.rs`:
1. `open_ended_run_resolves_end_to_the_day_after_now` — `(Some("2026-08-01"), None)` with
   `now = "2026-09-03T…"` resolves to `("2026-08-01", "2026-09-04")`.
2. `closed_run_is_returned_unchanged_by_resolution` — a `(Some, Some)` run is identity.
3. `whole_table_run_is_returned_unchanged_by_resolution` — a `(None, None)` run is identity.
4. `open_ended_run_starting_after_now_is_refused` — `start` = today+2 → `Err`, message names the
   model and both dates.
5. `open_ended_run_resolution_accepts_a_bare_date_now` — `now = "2026-09-03"` (no time part)
   parses identically to the timestamp form the CLI passes.

CLI end-to-end — `crates/smelt-cli/tests/since_upstream.rs`:
6. `web_analytics_whole_workspace_since_upstream_dry_run_completes` — real
   `examples/web_analytics`, `--source sources.raw.events --landed <w> --dry-run`, whole
   workspace (no `--select`): exits 0, stdout carries the `[2026-…, →)` open-ended RUN line, and
   stderr/stdout carry **no** "Both start and end" text. This is the phase's flagship gate.
7. `web_analytics_open_ended_run_logs_the_resolved_window` — same invocation with `--verbose`:
   the `[--since-upstream] running silver.sessions_chained` log line names the resolved
   `[<start>, <resolved-end>)` alongside the `→` form.

## Tasks

1. Write the spec delta above (spec-first) plus the docs-site sentence.
2. Add `pub fn resolve_run_window(run: &PropagatedRun, now: &str) -> Result<PropagatedRun>` to
   `crates/smelt-runtime/src/propagation.rs`: parses the leading `YYYY-MM-DD` of `now`, returns
   the run unchanged unless `start.is_some() && end.is_none()`, else sets
   `end = now_date + 1 day`; `bail!` naming the model when `start >= end`. Doc comment cites the
   spec paragraph and states why the plan itself keeps `end: None` (the dirty set is genuinely
   open-ended; resolution is a *scheduling* act).
3. Red: add tests 1–5, watch them fail, implement, watch them pass.
4. Wire `crates/smelt-cli/src/commands/run.rs::run_since_upstream`: inside the `for run in
   &plan.runs` loop, call `resolve_run_window(run, &now)?` (hoist `now` — it is currently a local
   built just before `plan_since_upstream_with_observed_deltas`) and build `ExecuteRequest` from
   the resolved run. Extend the `[--since-upstream] running …` log so the `(Some, None)` arm
   prints `[s, →) → [s, e)`.
5. Red: add tests 6–7, watch 6 fail with the "Both start and end" error, then pass.
6. Update the phase-22 test `self_referential_model_schedules_an_open_ended_run` only if the
   plan-level contract changed — it should NOT: `PropagatedRun.end` stays `None` in the plan.
   Confirm it still passes untouched.
7. Re-run the live repro from §Objective by hand and paste the tail into the phase summary.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test since_upstream_propagation`
- `cargo test -p smelt-cli --features duckdb --test since_upstream`
- `cargo test -p smelt-cli --test example_diagnostics`
- If a new `eprintln!`/`unwrap` lands, re-run `hardening_budget` and update
  `.claude/hardening-baseline.txt` (noting the delta in the commit body).

## Commit message

`feat(propagation): resolve an open-ended --since-upstream window to a finite run region`

# Phase 14 plan — the numbers are trustworthy on both targets

**Advances:** criterion 7 ("after each incremental window, each model's state equals a full
refresh over the inputs seen so far, on **both** targets"), completing it. The DuckDB half
of the criterion is already met over the 30-day committed fixture (phase 8,
`every_window_matches_the_full_refresh_oracle`). Since phase 17 widened the BigQuery source
to exactly that fixture, the DuckDB half now covers *the same population BigQuery runs on*
and needs nothing added. **What is missing is the BigQuery half, and only that.**

**Human-gated.** Live BigQuery, same credential path as phase 13. A headless iteration with
no credential must emit `<<PHASE_BLOCKED>>`.

**Depends on phase 13.** Read `phases/13-summary.md` before planning any command: it fixes
the thirty-window schedule, the exclusion set, the comparator and the divergence registry
this phase reuses. Do not re-derive them, and do not build a second
comparator — a claim from phase 13 and a claim from phase 14 must be the same kind of claim,
made by the same primitive (`EXCEPT ALL` both ways inside DuckDB over locally-landed rows).

## Spec delta

None. `docs/specs/incremental_models.md` §"The equivalence invariant" already states the
promise; this phase checks it on a real pipeline against a second engine.

## What "the oracle" means here

For window *k*, the oracle is a **full refresh over the inputs seen so far** — every source
row with `created_at` up to window *k*'s end — materialised into a *separate* set of
relations, then compared relation-by-relation against the incrementally-maintained state at
the same point. That is the DuckDB oracle's existing definition
(`github_activity_oracle.rs:574` `full_replay_pair`), and it carries over unchanged.

**Seven comparison points, matching the set phase 13 measured and declared**: windows 1, 2,
3, 5, 10, 20 and 30 of the 2026-08-05 … 2026-09-03 schedule. Phase 13 measured the landing
path at ~1,481 rows/s on the widest relation and found thirty full checkpoints would roughly
double the leg; the same arithmetic applies here, and the oracle leg additionally re-derives
from a growing prefix, so its per-point cost climbs. Reuse the declared set rather than
inventing a second one — a phase-13 claim and a phase-14 claim at the same window must be
about the same state. `PARITY_CHECKPOINTS` makes the full thirty a one-flag change if the
measurement supports it; if you widen the set, say so and show the measurement.

Record the set and its justification again in this phase's summary as an explicit criterion-7
caveat: the invariant is checked at seven of thirty windows on BigQuery, not all thirty.

Note what is *not* a variable here. Phase 13's one divergence (`bronze_events`) was arrival
order — BigQuery's source static, DuckDB's growing — and it vanished under a preloaded
replay. This phase compares one engine against itself over the same source, so arrival order
cannot explain anything it finds. A divergence here is a genuine equivalence-invariant
finding.

## D1 — the BigQuery oracle leg writes to its own dataset, declared in the committed project

Give the project a third target rather than mutating one at run time:

```yaml
  bigquery_oracle:
    type: bigquery
    project: smelt-bq-test-20260816
    dataset: smelt_dogfood_oracle
    location: US
    schema: smelt_dogfood_oracle
```

and extend the two source declarations' target-aware `name:` maps so the oracle leg reads
**the same physical source tables** rather than a copy:

```yaml
name:
  bigquery: smelt_dogfood.github_events
  bigquery_oracle: smelt_dogfood.github_events
```

(`models/sources/raw/github_events.yml` and `github_events_arrival.yml`.) Without the second
entry the default mapping resolves the source to `smelt_dogfood_oracle.sources_raw_github_events`
and the oracle silently reads an empty table — a vacuous pass, which is the failure mode this
phase exists to rule out. Assert it explicitly: the oracle leg's first run must materialise a
non-zero `bronze_events`, and a test must fail if the oracle's source resolution ever falls
back to the default mapping.

`target: dev` stays pinned. Adding a third target must not change the no-`--target` default;
confirm with `example_diagnostics` and `example_workspaces`, which read the committed project.

The oracle dataset is **created without a default table expiration** (the dogfood guard in
`scripts/bq-dogfood-env.sh` already unsets `SMELT_BQ_DEFAULT_TABLE_EXPIRATION_MS`) and is
**dropped at the end of the phase** — it is scaffolding, not history. Record its creation and
its drop in the summary.

## D2 — the DuckDB half is already met; do not rebuild it

Phase 8's `every_window_matches_the_full_refresh_oracle` runs the incremental replay and a
full-refresh oracle over all thirty windows of the committed fixture and compares every
materialised relation. After phase 17 that fixture *is* the BigQuery population, so the
DuckDB half of criterion 7 is met over the shared rows with no new work. Re-run it to confirm
it is still green, cite it, and move on — do not fork a second DuckDB oracle.

The one thing to check rather than assume: the DuckDB oracle's relation set and this phase's
must be the same fourteen. Phase 8's leg includes `silver.actor_sessions` and
`marts.daily_active_contributors`, which BigQuery refuses at compile time; state plainly that
criterion 7's BigQuery half covers fourteen of sixteen models and why, rather than implying
parity of coverage.

## D3 — what is compared, and what is excluded

Same relation discovery and exclusions as phase 13: model relations only; `sources_*`,
`_smelt_*` and `*__tombstones` excluded, for the reason the DuckDB oracle already gives —
they record *how* a run happened, and an incremental run's bookkeeping legitimately differs
from a `--full-refresh` run's. Note in the summary that on BigQuery these tables now exist
(`_smelt_ledger`, `_smelt_observed_delta`, the two `__tombstones`), which was not true when
phase 12 ran, and that excluding them is a decision rather than an absence.

The two compile-refused models (`silver.actor_sessions`, `marts.daily_active_contributors`)
are excluded on both legs, as in phase 13.

## D4 — a divergence here is a different animal from phase 13's

Phase 13's registry answers "do two engines agree with each other". This phase answers "does
one engine's incremental state equal its own full refresh". A non-zero diff here is a
**violation of the equivalence invariant** on a real pipeline, not an engine-compatibility
note, and the bar for registering rather than fixing it is correspondingly higher: an entry
must name the maintenance technique, the model, and the mechanism by which the incremental
plan legitimately lags — the shape `Bound::MonotoneDivergence` already encodes
(`github_activity_oracle.rs:148`). Anything that cannot be stated in those terms is a defect
for `20260906-bigquery-correctness`, recorded as a finding, not registered away.

If the sweep is empty, prove it fails closed on an empty registry, exactly as phase 13 does.

## Tests

Offline, per-PR, no credential:

1. `the_oracle_target_resolves_sources_to_the_shared_tables` — parses the committed
   `github_events.yml` / `github_events_arrival.yml` and asserts both carry a
   `bigquery_oracle:` entry pointing at `smelt_dogfood.*`; RED before the yml edit. This is
   the anti-vacuity gate for D1.
2. `adding_the_oracle_target_does_not_move_the_default` — the project's resolved default
   target is still `dev`.
3. The DuckDB half is `github_activity_oracle::every_window_matches_the_full_refresh_oracle`,
   unchanged — assert it is green, add no substitute for it.
4. `an_unregistered_equivalence_violation_fails` + `the_sweep_fails_closed_on_an_empty_registry`
   — synthetic pairs, mirroring phase 13's negative controls.
5. The existing suites stay green unchanged: `github_activity_oracle` (18),
   `github_activity_replay` (21), `github_activity_loader` (11), `example_diagnostics` (128),
   `example_workspaces`, and phase 13's new target.

Live, credential-gated:

6. `bigquery_incremental_matches_its_oracle_at_every_window` — the whole sweep. Must **fail**,
   not skip, when `SMELT_BQ_DOGFOOD_LIVE=1` is set without a usable credential.

## Tasks

1. Read `phases/13-summary.md`; restate nothing, reuse everything.
2. Add the `bigquery_oracle` target and the two `name:` map entries; write test 1 RED first,
   then the yml. Confirm tests 2 and 5.
3. Extend phase 13's driver (do not fork it) with an oracle leg: for each declared checkpoint *k*,
   run `--full-refresh --start 2026-08-05 --end <window k end>` on `bigquery_oracle`, snapshot,
   and compare against the incremental snapshot phase 13's driver already takes at *k*.
4. Confirm the DuckDB half (D2) is green and cite it; add nothing.
5. Run it live. Record per-run wall time, exit code, report id and billed bytes; the whole
   phase should stay well under a cent.
6. Register or escalate every divergence per D4.
7. Drop `smelt_dogfood_oracle` and confirm it is gone; `smelt_dogfood` and both source tables
   untouched.
8. `bash .claude/scripts/verify-phase.sh`; commit; push.
9. Write `phases/14-summary.md` and flip the phase-14 row in `outcome.md` to `done`, with a
   2026-09-12 decision-log entry stating what was measured and what criterion 7 now rests on.

## Cost and blast radius

Reads are bounded by the 64k-row source, multiplied by up to thirty full refreshes — measure
per-run billed bytes and stop and report if the running total passes US$2. Guards as phase 13: dry-run each new statement
shape; never touch `githubarchive`; never drop, truncate or reload `github_events` /
`github_events_arrival`; write only inside `smelt_dogfood` and `smelt_dogfood_oracle`; never
unpin `target: dev`.

## What this phase does not do

- It does not fix an equivalence violation it finds — that is `20260906-bigquery-correctness`.
- It does not extend the oracle to `silver.actor_sessions` or `marts.daily_active_contributors`.
- It does not promote the live leg into CI (standing decision: no BigQuery CI tier).

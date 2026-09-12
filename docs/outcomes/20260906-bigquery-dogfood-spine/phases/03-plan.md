# Phase 3 plan — succession on the real rename stream

## Objective

Add `silver.repo_naming`, `silver.actor_naming` and `marts.naming_history` to
`examples/github_activity/`, green on DuckDB with no warehouse. Advances criterion 9
(succession recognised from SQL shape alone, over the fixture's 34 renamed repos and 4
renamed actors, in **both** partition postures, with the redelivery folding once) and
extends criterion 4's "whole pipeline, not a spine" to the succession models.

## Spec delta

None. This phase builds an example project against the shipped succession grain
(`docs/specs/incremental_shapes.md` §"The succession grain"); no user-visible behaviour
changes. Any refusal or acceptance that surprises is recorded in
`docs/outcomes/20260906-scd2-keyed-succession`'s decision log — no grammar change here.

## Design calls the implementer must not re-litigate

- **The history is per-event, not per-rename.** "Only the rows where the name changed" is
  not a row-local predicate (it needs `LAG`), and the classifier admits exactly one
  *row-local* pre-window filter. So `silver.repo_naming` is one row per `(repo_id,
  created_at)` carrying the name in force, and `marts.naming_history` derives the actual
  renames downstream with `LAG`. Do not try to pre-filter the stream.
- **Two physical relations, one per posture.** `raw.github_events` keeps
  `partition_column: created_at` (event-time posture) and drives `repo_naming`; a second
  source `raw.github_events_arrival` over its own relation
  (`main.sources_raw_github_events_arrival`), same columns plus a loader-stamped
  `ingested_date DATE NOT NULL`, `partition_column: ingested_date`, `event_time_column:
  created_at`, drives `actor_naming`. Two source declarations over *one* relation was
  rejected: it would key two differently-partitioned fingerprint sidecars onto the same
  table, which is untested and is not what this phase is here to test.
- **The redelivery lands differently in each posture, deliberately.** In the event-time
  source the redelivered previous-day rows land in a **closed** partition, exercising the
  append-only probe's late-arrival classification (a count increase is a late append, never
  `SourceMutationProfileViolated`). In the arrival source the same rows are stamped
  `ingested_date = D` and land in the **open** partition. Same rule, both postures.
- **No delete/tombstone leg.** GitHub's `DeleteEvent` is a branch/tag delete, not a repo or
  actor deletion, so `QUALIFY NOT <flag>` here would be a lie. `examples/scd2_succession`
  already covers deletes; this phase covers the rename stream.
- Neither model declares `grain:`, `unique_key:` or `timeseries:` — recognition is the
  point. `refresh: incremental`, `materialization: table` only.

## Tests (red first) — `crates/smelt-cli/tests/github_activity_replay.rs`

1. `repo_naming_is_recognised_as_the_succession_grain` — `smelt explain repo_naming` reports
   `grain: succession`, `identity: (repo_id, created_at)`, `technique: succession-patch`,
   and `run axis: created_at (event-time-partitioned)` / `clock: created_at`.
2. `actor_naming_is_arrival_partitioned` — same model shape on the arrival source reports
   `run axis: ingested_date (arrival-partitioned)` and `clock: created_at`.
3. `redelivery_folds_once_in_both_succession_models` — after the day-by-day replay,
   `repo_naming` has exactly `count(DISTINCT (repo_id, created_at))` rows and `actor_naming`
   exactly `count(DISTINCT (actor_id, created_at))` over the replayed days; the run does not
   fail with `SuccessionClockTie` or `SourceMutationProfileViolated`.
4. `same_second_events_fold_once_within_a_key` — the fixture's own 139 repo / 145 actor
   `(key, created_at)` ties are all identical on the projected name (measured), so they fold
   like a redelivery; asserts row counts stay collapsed and no tie failure is raised.
5. `naming_history_surfaces_the_real_renames` — `marts.naming_history` has 34 distinct
   renamed `repo_id`s and 4 renamed `actor_id`s; contains the owner-change row
   (`mikiKG45/noob-devops-project` → `guslariR45/noob-devops-project`, same `repo_id`); and
   the 2 repo *names* reused across different `repo_id`s each appear under both ids.
6. `succession_full_refresh_matches_incremental_replay` — extend the existing full-refresh
   equivalence test to cover `repo_naming`, `actor_naming` and `naming_history` over the
   full 30-day fixture (criterion 7, DuckDB half).

## Tasks

1. Extend `setup_sources.sql` to create the empty `main.sources_raw_github_events_arrival`
   (sample columns + `ingested_date DATE`).
2. Add `models/sources/raw/github_events_arrival.yml`: `name:
   main.sources_raw_github_events_arrival`, `mutation_profile` copied from
   `github_events.yml` (append_only, `redelivery: at_least_once`, same `key_recurrence`),
   `timeseries: {event_time_column: created_at, partition_column: ingested_date,
   granularity: day}`, columns + `ingested_date` (`nullable: false`).
3. `run_incremental.py`: `load_day` also appends day D's real rows and the same
   deterministic 2% previous-day redelivery into the arrival relation, stamped
   `ingested_date = D` for both. Mirror in the Rust harness's `load_day`.
4. `models/silver/repo_naming.sql` — `repo_id`, `repo_name`, `created_at AS valid_from`,
   `LEAD(created_at) OVER (PARTITION BY repo_id ORDER BY created_at) AS valid_to`, the
   `IS NULL` currency flag; `FROM smelt.sources.raw.github_events`.
5. `models/silver/actor_naming.sql` — same shape on `actor_id`/`actor_login` from
   `smelt.sources.raw.github_events_arrival`.
6. `models/marts/naming_history.sql` — full-refresh table: union both histories into
   `(entity_kind, entity_id, name, valid_from)`, `LAG(name)` within the entity, keep rows
   where the previous name exists and differs; project `from_name`, `to_name`, `renamed_at`.
7. Write tests 1–6; make them pass. Confirm `smelt explain` output for both models is what
   test 1/2 assert rather than adjusting the assertion to whatever prints.
8. Update `examples/github_activity/README.md` with the two postures, the two relations, and
   why the history is per-event.
9. Write `phases/03-summary.md`: shipped, decisions, anything that surprised (destined for
   `scd2-keyed-succession`'s log and the criterion-8 findings handoff), gates.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test github_activity_replay`
- `cargo test -p smelt-lsp --test example_workspaces github_activity`
- `cargo test -p smelt-cli --test example_diagnostics`
- `python3 examples/github_activity/run_incremental.py` (manual, full 30-day replay)

## Commit message

`feat(examples): succession over the github_activity rename stream, both postures`

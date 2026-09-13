# Phase 11b plan — make the scheduled job self-driving and serverless-safe (offline)

## Objective

Phase 11a's summary surfaced three defects that make "three consecutive **scheduled** runs
complete" (criterion 11) unreachable as the bundle currently stands, all of them provable and
fixable with no workspace: the loader task is driven by the trigger's *real calendar date* while
the fixture holds a fixed historical range; the loader's DuckDB access shells out to a `duckdb`
CLI binary that a serverless Python environment will not have; and nothing declares or seeds the
Unity Catalog Volume the `smelt_run` task points `--project-dir` at. This phase closes all three
offline so 11c is purely "deploy, schedule, observe". It advances criterion 11 only.

## Spec delta

None. Nothing here changes user-visible smelt behaviour: the loader and the bundle are dogfood
scaffolding external to smelt (criterion 3 already documents the loader as external). If the
Volume-seeding contract turns out to need a docs statement, it belongs in
`docs-site/docs/guide/targets.md` §"Deployment: Databricks Asset Bundle" as a prose addition, not
a spec rule — fold it into the same commit if written.

## Tests

Red-green, in this order.

`crates/smelt-cli/tests/dbx_dogfood_loader.rs` (extend):
1. `loader_next_day_picks_the_earliest_unloaded_fixture_day` — with an empty dry-run store,
   `--next-day --dry-run-store <dir>` resolves to the fixture's **first** day, not today's date.
2. `loader_next_day_advances_across_consecutive_invocations` — three successive `--next-day` runs
   against the same store load the fixture's first three days in order.
3. `loader_next_day_is_a_no_op_once_the_fixture_is_exhausted` — a store recording every fixture
   day makes `--next-day` exit 0 with an explicit "nothing left to load" message, never an error
   and never a fabricated day (a scheduled run after the fixture runs out must not fail the job).
4. `loader_arrow_slices_are_identical_across_duckdb_access_paths` — the Python-module path and
   the CLI path produce byte-identical Arrow for every fixture day; skipped with a printed
   reason (not silently) when the `duckdb` Python module is absent locally.
5. `loader_runs_without_the_duckdb_cli_on_path` — the loader's dry-run path succeeds with `PATH`
   scrubbed of `duckdb`, proving the serverless-shaped environment works; same explicit skip when
   the Python module is absent.

`crates/smelt-cli/tests/databricks_bundle.rs` (extend):
6. `bundle_declares_the_volume_the_smelt_run_task_points_at` — a `resources.volumes` entry exists
   whose catalog/schema/name are the *same* `${var.…}` references the `smelt_run` task's
   `--project-dir` parameter composes; no literal path drift between the two.
7. `bundle_loader_environment_declares_every_dependency_the_loader_imports` — `loader_env`'s
   `dependencies:` covers the loader's imports (`duckdb` now included), derived from the loader
   file rather than restated.
8. `bundle_seed_never_overwrites_run_state` — the seed stage's copy list (parsed out of
   `scripts/dbx-bundle.sh`, not restated) names `smelt.yml` and `models/` and excludes `.smelt/`,
   so re-seeding a deployed project cannot destroy the ledger that makes each run incremental.

## Tasks

1. Add `--next-day` to `scripts/dbx-dogfood-loader.py`: enumerate the fixture's distinct
   `CAST(created_at AS DATE)` days (one query, reusing the existing SQL builders' fixture path),
   subtract the days the ledger already records (the live `_loader_days` table, or the dry-run
   store's ledger file — one predicate, both backends), and execute the earliest remainder. Exit
   0 with a message when the remainder is empty.
2. Replace `duckdb_query_arrow`/`duckdb_scalar`'s hard CLI shell-out with a single resolver that
   prefers the `duckdb` Python module and falls back to the CLI, keeping *one* SQL definition per
   query. Do not duplicate the SELECT builders.
3. Add `duckdb` to `scripts/dbx-dogfood-requirements.txt` and to `loader_env`'s `dependencies:`
   in `examples/github_activity/resources/github_activity_job.yml`.
4. Change the `load_next_day` task's parameter from `{{job.trigger.time.iso_date}}` to
   `--next-day` (and update `examples/github_activity/dbx_job/load_next_day.py`'s argument
   handling and docstring accordingly), so a scheduled run advances the fixture by its own
   ledger rather than by wall-clock.
5. Declare the Volume as a bundle resource (`resources/` — a `volumes:` entry in the job file or
   its own file, whichever keeps the var references single-owned) matching the
   `${var.catalog}/${var.schema}/${var.volume_name}` the `smelt_run` task already composes.
6. Add a `seed` subcommand to `scripts/dbx-bundle.sh` that copies `examples/github_activity`'s
   `smelt.yml` and `models/` to the Volume project path via the Databricks CLI's file commands,
   explicitly excluding `.smelt/`; add it to the wrapper's subcommand allow-list, its usage
   string, and the matching `.claude/settings.json` entry.
7. Update `docs-site/docs/guide/targets.md` §"Deployment: Databricks Asset Bundle" for the
   self-driving loader and the seed step, and `CLAUDE.md`'s `scripts/dbx-bundle.sh` mention if
   the subcommand list is spelled there.

## Verification

- `bash .claude/scripts/verify-phase.sh` — must be all green.
- `cargo test -p smelt-cli --test dbx_dogfood_loader --test databricks_bundle`
- `bash scripts/dbx-bundle.sh validate` from `env -i` (the stub path) — exit 0 with the new
  Volume resource present.
- `bash .claude/scripts/shellcheck-gate.sh` — zero findings on the edited `dbx-bundle.sh`.
- `cargo test --workspace --no-fail-fast` before declaring done (11a learned that
  `verify-phase.sh` fails fast and hides later binaries' failures).

## Commit message

`outcome(databricks-dogfood-spine): phase 11b makes the scheduled loader self-driving, drops the duckdb CLI dependency and declares/seeds the Volume`

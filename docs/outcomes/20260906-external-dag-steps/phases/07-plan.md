# Phase 7 plan — `examples/github_activity/` dogfoods its loader as an external step

## Objective

Turn the fixture's day-loader from three drifting copies (a Python function, a Rust test
helper, and a BigQuery shell script) into **one external program smelt itself invokes** as a
declared black-box step producing both raw sources. This is criterion 6's fixture half and
the only end-to-end evidence for criteria 3 and 4 on a real pipeline rather than a synthetic
one: `smelt run` orders the loader ahead of every consumer, invokes it, and fails the run
when it fails. The docs-site page is phase 8.

## Spec delta

**None.** No user-visible behaviour changes here — the declaration surface, invocation
semantics and diagnostics all landed in phases 1-6. `examples/github_activity/README.md` is
updated as fixture documentation, not as spec.

## Design calls settled here

- **The loader is `examples/github_activity/load_day.sh`, bash + the `duckdb` CLI.** The
  step's `command:` must run in CI, and CI installs only `libduckdb.so` — no `duckdb` Python
  module, no CLI. Bash is universal and the CLI is one `curl` away, so this phase provisions
  the CLI (`.github/actions/setup-duckdb`, `mise run setup-duckdb`) rather than gating the
  fixture's tests off. A Rust loader binary was rejected: it would put fixture code in a
  shipped crate and inside the hardening ratchet's "production" derivation.
- **The loader carries its own day ledger and is idempotent per day** (`main._loader_days`;
  a day already recorded is a no-op exit 0). This is load-bearing, not cosmetic: the oracle
  leg must stage N days *without* running smelt N times and then run `--full-refresh`, whose
  step invocation would otherwise re-append an already-loaded day; and `run.rs`'s propagated-
  region loop calls `execute_project` (and therefore the step) once per region, so a run may
  legitimately invoke the loader more than once. smelt guarantees no idempotence
  (`sources.md` §"what smelt does not guarantee") — this is the fixture loader's own
  bookkeeping, opaque to smelt, exactly as a real at-least-once day loader would carry.
- **Redelivery semantics move verbatim, not re-derived**: the 2% `MOD(CAST(id AS BIGINT), 50)
  = 0` slice of D-1, appended to the event-time relation and stamped `ingested_date = D` in
  the arrival relation, is now stated once in the script and pinned by a test.

## Tests

1. `load_day_script_is_idempotent` (`github_activity_loader.rs`) — running `load_day.sh
   --date D` twice against one database yields the same row counts in both relations as
   running it once.
2. `load_day_script_redelivers_the_previous_day` — after days D and D+1, the event-time
   relation carries the 2% D-slice a second time under its original `created_at`, and the
   arrival relation carries those same ids stamped `ingested_date = D+1`.
3. `load_day_script_creates_the_raw_tables_when_absent` — a fresh database needs no
   `setup_sources.sql` pre-pass for the step to succeed.
4. `smelt_run_invokes_the_loader_step` (`github_activity_replay.rs`) — a staged workspace
   with **no** pre-loaded rows, run for day 0, ends with day 0's rows in both sources and
   `bronze.events` built: smelt drove the load.
5. `loader_step_failure_leaves_downstream_unbuilt` — replacing `load_day.sh` with an
   `exit 3` stub makes `smelt run` fail non-zero, naming the step, with no model relation
   created (criterion 4 on the real fixture).
6. `github_activity_declares_its_loader_step` (`list_external_step.rs`) — `smelt list
   --json` over `examples/github_activity` shows one external step producing both
   `smelt.sources.raw.github_events` and `..._arrival` (criterion 3 on the real fixture).
7. `full_refresh_matches_incremental_replay` (existing, `github_activity_replay.rs`) — green
   unchanged with the loader now driven by the step.
8. `every_window_matches_the_full_refresh_oracle` (existing, `github_activity_oracle.rs`) —
   green unchanged; the oracle's staged days are loaded by direct script calls and the
   `--full-refresh` run's step invocation no-ops on the day ledger.
9. `github_activity` (existing, `crates/smelt-lsp/tests/example_workspaces.rs`) and
   `example_diagnostics` — zero diagnostics with the step declared (criterion 6).

## Tasks

1. Write `examples/github_activity/load_day.sh`: `--date D [--database PATH]` (default
   `target/dev.duckdb`, resolved relative to the script's own directory so the step's
   `current_dir` = project dir works); creates both raw tables if absent; creates and
   consults `main._loader_days`; appends day D plus the D-1 redelivery to both relations;
   records D. Comment the ledger's purpose and the redelivery rule at the top.
2. Add `examples/github_activity/models/sources/raw/github_loader.yml` — `external_step:`
   with `description`, `produces:` both source addresses, `command: ["bash",
   "load_day.sh", "--date", "{run_date}"]`, `cadence: '1 day'`.
3. Replace `run_incremental.py::load_day`'s inline SQL with a call to `load_day.sh`, and drop
   its per-day pre-load entirely where `smelt run` now covers it — the driver becomes
   setup + `smelt run` per day + `smelt test`.
4. Replace `github_activity_support::load_day`'s inline SQL with a `load_day.sh` invocation
   (same signature, so call sites are unchanged), and panic with a clear message if `duckdb`
   is not on `PATH` rather than skipping.
5. Rewire the two replay/oracle incremental legs to stop pre-loading the day under test (the
   step loads it); leave the oracle's multi-day staging calls in place. Add `_loader_` to
   `EXCLUDED_PREFIXES` in `github_activity_oracle.rs` with a one-line reason.
6. Provision the `duckdb` CLI in `.github/actions/setup-duckdb/action.yml` (cached alongside
   the library) and in `mise run setup-duckdb`.
7. Update `examples/github_activity/README.md`: the loader is a node in smelt's DAG, one
   implementation, and the day ledger is why replay and oracle can share it.
8. Re-run the ratchets; bump `.claude/hardening-baseline.txt` / large-file baseline only if
   they actually move, each with an inline sign-off note.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test github_activity_loader --test github_activity_replay --test github_activity_oracle --test list_external_step --test explain_external_step`
- `cargo test -p smelt-lsp --test example_workspaces github_activity`
- `cargo test -p smelt-runtime --test execute_parity`
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`feat(examples): github_activity drives its day loader as a declared external step`

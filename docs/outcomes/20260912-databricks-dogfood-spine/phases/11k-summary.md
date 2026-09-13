# Phase 11k summary — Live: seed the job's frontier (blocked one layer deeper)

**Shipped:**
- Real ingestion frontier discovered from `workspace.smelt_dogfood._loader_days`: 21 distinct
  days, `min=2026-08-05`, `max=2026-08-25`.
- All 16 models' `databricks_job` intervals seeded (`smelt state seed-interval`, one call per
  model, in a scratch checkout) with `--start 2026-08-05 --end 2026-08-26` — end is **exclusive**
  and becomes `--auto`'s next start verbatim (`IntervalStore::latest_date`,
  `crates/smelt-state/src/intervals.rs:96`), so the correct value is one day *past* the last
  ingested day, not the last ingested day itself (caught before upload; an initial `--end
  2026-08-25` run was discarded and redone).
- Uploaded exactly one file (`databricks fs mkdir` + single-file `fs cp --overwrite`) to
  `dbfs:/Volumes/workspace/smelt_dogfood/smelt_project/project/.smelt/targets/databricks_job/intervals.json`;
  verified byte-identical via `fs cat`; confirmed `.smelt/lock` and no other subtree were touched.
- Root-caused and fixed a genuine, unrelated deploy bug: `examples/github_activity/.gitignore`'s
  `dist/` entry made `databricks bundle deploy`'s default `sync.paths` (which honours
  `.gitignore`) silently drop the wheels the `artifacts:` build step mirrors into
  `examples/github_activity/dist/`, even though the deploy log claimed success — the
  `smelt_env.dependencies` path never existed in the workspace. Fixed with `sync.include:
  [dist/*.whl]` in `databricks.yml` (confirmed via `databricks bundle schema`'s documented
  `config.Sync.include` override); confirmed via `databricks workspace list` that both wheels now
  land at the referenced path. `verify-phase.sh` and the 24-case `databricks_bundle` structural
  gate are green with this change.
- Redeployed twice (wheel + Volume seed both current) and ran two manual smoke runs.

**Decisions:**
- Treated the sync/`dist/` bug as fixable-in-pass (small, mechanical, root-caused with certainty)
  and committed it; treated the external-step gap below as a design question and did not
  improvise a fix — see Blocked.

**For the next planner — the real blocker, one layer past what 11j closed:**

The seed genuinely worked: after the sync fix, the smoke run's failure moved from
`ExternalStepNotInvocable` (no window — the bug 11j/11k's seed targets) to `ExternalStepFailed:
step 'sources.raw.github_loader' exited with code 127` (`bash: load_day.sh: No such file or
directory`) — proof `--auto` now derives a real, non-empty window from the seeded frontier.

But this step *cannot* succeed against `databricks_job` even once `load_day.sh` is reachable:
- `load_day.sh` is the **DuckDB-CLI dev-target loader** — it shells out to a local `duckdb`
  binary, reads a local `seeds/github_events_sample.parquet`, and writes a local
  `target/dev.duckdb`. None of that exists or makes sense on Databricks serverless compute.
- smelt has exactly **one** `external_step:` declaration per source
  (`models/sources/raw/github_loader.yml`), shared across every target — there is no per-target
  command override in `docs/specs/sources.md`.
- `invoke_external_steps` is hardcoded `true` in every CLI call site (`crates/smelt-cli/src/
  commands/run.rs:256,573`, `build.rs:241,446`, `rebuild.rs:72`, `bakeoff/run.rs:324`) — there is
  no flag or target config to make a run skip re-invoking an already-satisfied step.
- Even if there were, the current refusal semantics (`docs/specs/sources.md` §Semantics 12,
  `ExternalStepNotInvocable`) are deliberately fail-loud: "reading the produced sources'
  possibly-stale existing content instead is never the fallback." So today, declining invocation
  means refusing the whole run, not trusting data a separate task already landed.
- `dbx_job/load_next_day.py`'s own doc comment explains why the loader **cannot** simply be
  pointed at the Databricks path either: it must run **in-process** inside the job's own
  notebook-style REPL to see the ambient Databricks Connect session — a `tokio::process::Command`
  child (which is what `invoke_required_steps` always spawns) has no share of that session at
  all.

This is the design decision no phase can make unreviewed — same shape as the outcome-level
`## Blocked` entry the 2026-09-14 decision log already resolved once (seed vs. generalize
`--auto`), but one layer deeper: this one is about whether/how external steps participate in
frontier tracking at all. Candidate options, none improvised here:

1. **Give external steps their own interval/frontier tracking**, parallel to models', so
   `graph.steps_required_by` skips a step whose produced sources already cover the requested
   window. Closest to `run_smelt.py`'s original (now falsified) comment. Needs a spec change to
   `docs/specs/sources.md` and extending `smelt state seed-interval` (or a sibling tool) to seed
   step coverage too.
2. **A real "declines invocation, trusts already-produced sources" mode**, gated on an explicit
   freshness *proof* (e.g. checking the produced table's own max/count against the claimed
   window) rather than blind trust — a narrower, spec'd carve-out of the fail-loud rule above.
3. **Target-aware dispatch inside the step's own `command:`** that, on `databricks_job`, does a
   fast idempotent freshness check instead of a load — but a check needs its own live connection,
   and the step runs as a plain subprocess with no ambient session, which conflicts with 11d's
   ambient-only (no host/token) design commitment for this target. That tension needs an explicit
   ruling before this option is viable.

Everything from task 6 onward in the 11k plan (compressed cadence, three scheduled runs, report
pull, quota recording, `volume_probe` writeup, cadence restore) is **not started** — blocked on
the above. The daily cadence was never compressed this pass, so nothing needs restoring.

**Gates:**
- `cargo test -p smelt-cli --test databricks_bundle --quiet` — 24 passed.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full workspace test, example_diagnostics).
- No ratchet lowered.

# Phase 11c summary — deploy attempt, blocked on a missing Unity Catalog grant

**Status: blocked.** Tasks 1–4 landed and are green; task 5 (`deploy`) failed on a permission
gap that needs a human. Tasks 6–13 were not reached.

## Shipped

- `examples/github_activity/databricks.yml` — `schedule_cron` bundle variable (default
  `0 0 6 * * ?`, the compressed cadence is a `--var` override only).
- `examples/github_activity/resources/github_activity_job.yml` — the daily job's schedule now
  reads `${var.schedule_cron}` and declares `pause_status: UNPAUSED`; a new
  `github_activity_volume_probe` job resource (task `volume_probe`, one dependency-free
  serverless environment).
- `examples/github_activity/dbx_job/volume_probe.py` — measures `flock` advisory locking,
  `os.replace()` rename atomicity, and `fsync` against the Volume path, printing one JSON
  verdict.
- `crates/smelt-cli/tests/databricks_bundle.rs` — `the_one_job()`/`named_job()` split (two jobs
  now exist), `bundle_declares_one_daily_scheduled_serverless_job` amended to check the
  variable reference plus its committed default, two new tests
  (`bundle_schedule_is_explicitly_unpaused`, `bundle_volume_probe_targets_the_declared_volume`).
  12/12 green; `databricks bundle validate` green.
- `scripts/dbx-bundle.sh` — bug fix: `deploy`/`run`/`seed` now export `DATABRICKS_TOKEN`
  alongside `DATABRICKS_HOST` (previously token-less, so every live subcommand failed unified
  auth outright), plus a `SMELT_DBX_TOKEN` presence check.
- `scripts/dbx-provision.sh` — the oauth-m2m grant now includes `CREATE VOLUME` on
  `${CATALOG}.${SCHEMA}` (not the oracle schema, which has no Volume). Not yet applied to the
  live workspace — needs a human to re-run it or issue the single `GRANT` by hand.
- `smelt_sql.data/scripts/` created locally (gitignored) — `pyproject.toml`'s
  `data = "smelt_sql.data"` requires it for `maturin build`; this worktree never had it.
- `.smelt-dbx-venv` rebuilt via `dbx-dogfood-venv.sh` so `duckdb` (a requirements.txt entry
  since 11b) is actually installed.

## Decisions

- Did not attempt to self-grant `CREATE VOLUME` using the scoped service-principal credential,
  even though `dbx-query.sh` could technically issue arbitrary SQL. `dbx-query.sh` is
  deliberately read-only by design (phase 4a), and the credential is deliberately scoped down;
  bypassing that boundary to attempt a privilege grant it doesn't hold would defeat the point.
  Treated as human-gated, same as phase 4b.
- Left the two job resources that deploy did create (`github_activity_daily`,
  `github_activity_volume_probe`) in the workspace rather than trying to tear them down —
  they're harmless (the daily job's schedule is at the committed default and will just fail its
  `smelt_run` task if it ever fires, since the Volume doesn't exist), and the next `deploy` will
  reconcile them once the grant lands.

## For the next planner

- Resume 11c from task 5 once a human applies one of the two grant routes in outcome.md's
  `## Blocked` entry (re-run `dbx-provision.sh`, or hand-issue the single `GRANT CREATE VOLUME`
  statement).
- Unconfirmed: whether a plain schema-level `GRANT CREATE VOLUME` is sufficient for Databricks
  Asset Bundles to create a **managed** Volume, or whether Free Edition additionally needs
  broader Unity Catalog admin capability. If the grant alone isn't enough, resuming 11c may need
  `databricks bundle deployment bind` against a human-pre-created Volume instead of bundle-owned
  creation — new work, not scoped by this phase's plan.
- The `dbx-bundle.sh` token bug would have blocked every future `deploy`/`run`/`seed` call, not
  just this one — worth a regression test (a structural test asserting the script exports both
  `DATABRICKS_HOST` and `DATABRICKS_TOKEN` on those branches) if a future phase touches this
  script again.
- Token TTL is ~1 hour (docs/outcomes' phase-9f blocked entry already flagged this); this phase
  needed one `dbx-auth.sh` remint mid-session. Once 11c resumes and runs the ~40-minute
  three-scheduled-run wait, budget at least one more remint during that wait.

## Gates

- `cargo test -p smelt-cli --test databricks_bundle` — 12/12 green.
- `bash scripts/dbx-bundle.sh validate` — green.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full `cargo test`, `example_diagnostics`).
- `bash scripts/dbx-verify.sh` — green (workspace reachable, both schemas visible, out-of-scope
  `CREATE SCHEMA` correctly refused).
- `databricks bundle deploy` (live) — **failed** at the Volume-resource step; this is the
  phase's own blocked leg, not a pre-flight failure elsewhere.

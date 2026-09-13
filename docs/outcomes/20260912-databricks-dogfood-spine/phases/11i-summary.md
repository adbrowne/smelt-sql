# Phase 11i summary — blocked on a genuine `--auto`/target-aliasing gap (sixth attempt)

**Shipped:** five real, first-time-discovered infra bugs fixed and verified live, all
committed with `verify-phase.sh` ALL GREEN:
- Two-artifact wheel split (`smelt_wheel_x86_64`/`smelt_wheel_aarch64` in `databricks.yml`) —
  works around `databricks/cli#2969`'s dual-wheel-artifact cleanup bug.
- `smelt_env.dependencies` now references `${workspace.file_path}/examples/github_activity/
  dist/${var.x86_64_wheel_name}` (and aarch64) instead of a local relative glob, which the
  bundle CLI never resolved correctly regardless of path depth or mirroring.
- `scripts/dbx-bundle.sh deploy` pre-builds and passes the two exact wheel filenames as bundle
  variables (pip does not glob-expand `/Workspace/...` requirement specs).
- `run_smelt.py` resolves the `smelt` binary via `shutil.which`/`sys.exec_prefix` fallback, and
  sets `LD_LIBRARY_PATH` from `smelt_sql`'s real install location (the binary's own baked-in
  RPATH `$ORIGIN/../smelt_sql.libs` doesn't resolve on this environment's site-packages layout).
- `run_smelt.py` supplies placeholder `SMELT_DBX_HOSTNAME`/`SMELT_DBX_TOKEN` values so
  `smelt.yml`'s eager per-target env-var interpolation doesn't fail for targets `databricks_job`
  never uses.
- `run_smelt.py` passes `--auto` to `smelt run` (needed regardless of the blocker below).
- Updated `crates/smelt-cli/tests/databricks_bundle.rs` for the two-artifact shape.
- Bundle redeployed at the committed daily cadence (`0 0 6 * * ?`, UNPAUSED) — clean baseline,
  nothing left compressed or mid-deploy.

**Decisions:**
- Reverted `crates/smelt-cli/tests/github_activity_dbx_scheduled.rs`'s hard-gate flip (this
  plan's task 1) back to skip-on-missing, since the evidence files never landed this pass —
  landing them prematurely would have left the suite red. Also logged in outcome.md.
- Left `${SMELT_DBX_HOSTNAME}`/`${SMELT_DBX_TOKEN}` placeholder injection in `run_smelt.py`
  rather than changing `smelt.yml`'s config-loading semantics to skip unselected targets —
  the latter is a real, broader product change (spec implications) out of this phase's scope.

**For the next planner:**
- The blocking discovery is architectural: `smelt run --auto`'s frontier detection
  (`compute_auto_time_range`) is local-`.smelt/`-store-based and ends its window at real
  wall-clock "today" — both wrong for `databricks_job`, a separately-named target from
  `databricks` that shares the same physical schema (12+ days already loaded there) but starts
  with an empty local interval store. See outcome.md's Blocked entry for the three candidate
  fixes (backend-resident-state reconciliation, shared interval store keyed by physical
  location, or an explicit `--start`/`--end` handed across via `dbutils.jobs.taskValues`). This
  needs a design decision, not another live attempt — the next planner should decide the
  direction before dispatching another implement pass on this row.
- Worth checking whether BigQuery's dogfood spine has the identical gap and simply never
  surfaced it, before assuming this is Databricks-specific.
- The Databricks CLI dual-wheel bug (databricks/cli#2969) and the `environments.dependencies`
  local-glob resolution failure are both worth a short write-up somewhere durable (docs-site or
  a comment) if another bundle in this repo ever needs multi-arch wheels — the workarounds are
  non-obvious and took significant live-loop iteration to find.
- `.smelt/` on the Volume held only a `lock` file with no manifest before this pass — worth
  confirming (once the `--auto` blocker clears) that run-state on Databricks is genuinely
  Delta/catalog-resident rather than file-resident, matching expectation.

**Gates:** `bash .claude/scripts/verify-phase.sh` — ALL GREEN. `bash scripts/dbx-bundle.sh
validate` — clean. Live: `bash scripts/dbx-verify.sh` — clean (reachability + refusal), deploy
succeeded, manual smoke run reached `smelt run` and failed with `ExternalStepNotInvocable`
(the blocker above) — no scheduled runs were attempted since the smoke run (task 5, a
prerequisite) never completed cleanly.

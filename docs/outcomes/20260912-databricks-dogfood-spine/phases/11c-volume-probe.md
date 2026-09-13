# Phase 11c — Volume FUSE probe result

Ran `github_activity_volume_probe` (task `volume_probe`, `dbx_job/volume_probe.py`) against the
deployed Unity Catalog Volume `workspace.smelt_dogfood.smelt_project` at
`/Volumes/workspace/smelt_dogfood/smelt_project/project`.

Run: https://dbc-466c2133-56f4.cloud.databricks.com/jobs/42210763587936/runs/780053178034229
(TERMINATED / SUCCESS, ~24s).

```json
{"flock_advisory": {"honoured": true, "second_holder_result": "blocked"}, "rename_atomicity": {"honoured": true}, "fsync": {"honoured": true, "error": null}}
```

## Verdict

All three `.smelt/` filesystem guarantees hold on this Unity Catalog Volume's FUSE layer:

- **`flock` advisory locking**: honoured. A second process attempting `LOCK_EX | LOCK_NB` while
  the first holds the lock is correctly blocked (`EACCES`/`EAGAIN`), not silently granted.
- **`os.replace()` rename atomicity**: honoured. The renamed-over manifest lands its full new
  content; no reader would observe a truncated or missing file mid-rename.
- **`fsync`**: honoured, no error.

Per the plan's conditional spec delta: since the Volume **does** honour both guarantees, the
positive result is recorded in `docs-site/docs/guide/targets.md` only — no Known Divergence is
added to `docs/specs/run_state.md`, since nothing here diverges from the guarantees `.smelt/`
already assumes.

## A load-bearing finding surfaced by this probe

This was the **first bundle-job task ever executed on this workspace** (every prior live phase
drove `smelt run --target databricks` directly from this machine via Databricks Connect, never
through a Databricks Job). The first run attempt, with every job environment declaring
`client: "1"` (the value the 11a/11b plans committed), failed outright before reaching probe
logic at all:

```
Cannot launch the cluster. Cause: Invalid platform channel Client-1. INVALID_PARAMETER_VALUE:
INVALID_PARAMETER_VALUE: Workspace doesn't support Client-1 channel for REPL.
```

This reproduced identically via `databricks jobs run-now` directly (bypassing `bundle run`
entirely), so it is a genuine workspace-level constraint on the serverless environment version,
not a CLI quirk. `client: "2"` launches cleanly. Fixed in
`resources/github_activity_job.yml` for **all three** job environments (`loader_env`,
`smelt_env`, `probe_env`) — not just the probe — since the daily job's tasks would have hit the
identical failure on their first scheduled run otherwise. See outcome.md's decision log for
2026-09-13 (phase 11c implement).

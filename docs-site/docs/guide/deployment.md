# Production Deployment

This page walks through taking a smelt project from a laptop to a scheduled production run: installing the binary, configuring targets and secrets, understanding what `.smelt/` stores, and how to back it up and upgrade it safely.

## Installing for servers

Install the `smelt` CLI the same way you would locally -- see [Installation](../getting-started/installation.md) for the available methods (pip wheel, Homebrew, Docker image). For a CI or production host, pin an exact version rather than tracking latest, so a run behaves identically across deploys:

```bash
pip install "smelt-sql==<version>"
```

Rebuilding the pin is a deliberate upgrade step (see [Upgrades](#upgrades) below), not something that happens implicitly on every deploy.

## Project layout in CI

A typical production setup checks out the project repository and runs `smelt build` (seed + run) or `smelt run` against a named target:

```bash
git clone <your-repo> && cd <your-repo>
smelt run --target prod
```

`smelt.yml` is checked into version control alongside your models. Nothing about a production run requires a different project layout than local development -- the only difference is which `--target` you point at and where its `.smelt/` state lives (normally the checkout's working directory, so state persists across invocations only if the working directory itself persists between runs).

## Configuring targets

Targets are named execution environments under the `targets:` key in `smelt.yml` -- see [Targets and Backends](targets.md) for the full field reference. A production deploy typically adds a `prod` target alongside the `dev` target used locally:

```yaml
targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main

  prod:
    type: duckdb
    database: /var/lib/smelt/prod.duckdb
    schema: main
    settings:
      memory_limit: "16GB"
      threads: "8"
      temp_directory: /var/lib/smelt/spill
```

```bash
smelt run --target prod
```

### Resource limits

DuckDB targets accept a `settings:` map applied as `SET key = value` on connection open (unknown keys are rejected at startup). The keys that matter most for a production host are `memory_limit`, `threads`, and `temp_directory` -- see [Targets and Backends](targets.md#duckdb-settings) for the full table. If `settings:` omits `memory_limit` or `temp_directory`, smelt applies host-aware defaults (roughly `min(50% of RAM, RAM - 20GB)` for memory, `.smelt-duckdb-tmp` next to the database file for spill) rather than DuckDB's own unbounded default -- see [Project Configuration](../reference/smelt-yml.md#memory-limits-and-spilling) for the exact defaulting rule. On a shared or memory-constrained host, set both explicitly instead of relying on the default.

## Secrets

Never write a `connect_url`, credential, or other secret directly into `smelt.yml` -- the file is normally committed to version control. Reference an environment variable instead with `${VAR}` interpolation, resolved once against the process environment when the config loads, before any other validation runs:

```yaml
targets:
  prod:
    type: spark
    connect_url: ${SMELT_PROD_CONNECT_URL}
    catalog: spark_catalog
    schema: main
```

A missing environment variable is a fail-loud configuration error naming the variable and the YAML key path -- smelt never silently substitutes an empty string and lets a blank credential fail later inside a connection attempt. A literal `$` that is not the start of a `${VAR}` reference is written `$$` to escape it. Set the referenced variable through your CI secrets store or the host's environment (e.g. a systemd unit's `Environment=` / `EnvironmentFile=`, or an orchestrator's secret injection) -- never in a `.env` file checked into the repository.

## `.smelt/` state layout

Any target with `state.mode` other than the default `stateless` persists run state under a project-local `.smelt/` directory (gitignore this directory -- it is regenerable host-local state, not something to commit):

```
.smelt/
  meta.json                       # { "state_version": 1 } -- layout version marker
  lock                             # advisory single-writer lock, held for a run's duration
  targets/<target>/
    runs/<run_id>.json              # one run manifest per execution
    intervals.json                  # cumulative interval coverage across runs
    reconciliation.json             # reconciliation ledger for grain: key models
    landed_deltas.json              # per-source landed-delta intervals
    snapshots.json                  # fingerprint snapshots (state.mode: environments)
    schemas/<model>.json            # deployed schema snapshot per model
    reports/<run_id>.json           # run-report artifact for the run
```

Every run-scoped artifact is nested under `.smelt/targets/<target>/`, keyed by the target the run executed against -- a `dev` run's interval ledger or reconciliation state never answers a question about `prod`, and vice versa. Only `meta.json` and `lock` live at the project root, shared across every target.

Set `state.mode` in `smelt.yml` to opt into persistence -- `stateless` (the default) writes nothing and requires no `.smelt/` directory at all:

```yaml
state:
  mode: intervals   # stateless | intervals | environments
```

## Locking

A run acquires an exclusive advisory lock on `.smelt/lock` for its entire duration and releases it on completion or error. A second smelt process that tries to acquire the lock while it is held fails loudly, naming the PID recorded in the lock file, rather than interleaving writes with the first process. This means a production scheduler must never fire overlapping invocations against the same target's state directory.

## Backup and restore

`.smelt/` is plain JSON files, safe to copy with any file-level backup tool while no run is in progress (or, for a live backup, after acquiring the same advisory lock your ops tooling uses for other maintenance). Every write under `.smelt/` is atomic (temp file + rename), so a snapshot taken between runs is always internally consistent -- there is no "flush" step to coordinate. Restoring is copying the backed-up `.smelt/` directory back into place; because state is opt-in and regenerable (a `stateless` project needs none, and an `intervals`/`environments` project can rebuild its ledgers from a full re-run), a lost `.smelt/` is a correctness/performance regression to full recomputation, never data loss of your warehouse tables.

## Upgrades

`.smelt/meta.json` records the on-disk layout's `state_version`. Upgrading the smelt binary is safe to do in place -- a newer binary understands older layout versions and migrates a legacy (pre-versioning) store automatically, under the lock, on first open. A `state_version` newer than the binary understands is a hard error naming both versions, so a downgrade of the smelt binary against state written by a newer version fails loudly instead of misreading it. Roll out a binary upgrade the same way as any other deploy: update the pin, redeploy, let the next scheduled run perform any needed migration.

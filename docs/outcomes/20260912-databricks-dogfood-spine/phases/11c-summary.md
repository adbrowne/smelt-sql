# Phase 11c summary (resumed attempt) — deploy landed, probe clean, blocked on ambient host

**Status: blocked.** Tasks 1–5 done and committed; task 6 (compressed-cadence redeploy) done;
task 7 (three consecutive scheduled runs) blocked on a real, load-bearing gap in the
`databricks_job` target's ambient-host design — every scheduled run's `load_next_day` task fails
before it can advance the fixture. Tasks 8–13 not reached. The workspace has been left at the
committed daily cadence (`0 0 6 * * ?`, UNPAUSED), matching what task 11 would otherwise restore.

## Shipped

- `scripts/dbx-bundle.sh` — new read-only `runs` subcommand (`runs list <job-name>` →
  `jobs list-runs`, `runs get <run-id>` → `jobs get-run`), both exporting
  `DATABRICKS_HOST`/`DATABRICKS_TOKEN`. `seed`'s `VOLUME_PATH` now carries the `dbfs:` scheme
  Unity Catalog Volume paths need (measured: `databricks fs cp`/`ls` reported "no such
  directory" against `/Volumes/...` without it, even though the volume existed) and `seed` now
  `mkdir`s the destination `project/` directory first (`fs cp` does not create it implicitly).
- `crates/smelt-cli/tests/databricks_bundle.rs` — `every_live_subcommand_exports_both_credentials`
  and `runs_subcommand_is_read_only` (plan tests 1–2), 14/14 green.
- `examples/github_activity/databricks.yml` — the `smelt_wheel` artifact's `build:` command now
  resolves `DUCKDB_LIB_DIR`/`LD_LIBRARY_PATH` itself and clears the stale `libduckdb-sys`/`smelt`
  build cache before invoking `maturin build`. Root cause: `libduckdb-sys`'s build script falls
  back to **downloading** a hash-named prebuilt `libduckdb-<hash>.so` release asset whenever it
  cannot resolve `DUCKDB_LIB_DIR` at compile time, and once that fallback artifact is cached
  under its own Cargo fingerprint bucket, later builds with `DUCKDB_LIB_DIR` correctly set can
  still resolve back to the stale bucket depending on build-directory state — forcing a clean
  slate before every `deploy` build side-steps the whole class of staleness rather than chasing
  the exact caching rule.
- `examples/github_activity/resources/github_activity_job.yml` — **all three** job environments
  (`loader_env`, `smelt_env`, `probe_env`) moved from `client: "1"` to `client: "2"`. `client:
  "1"` failed every job-task launch on this workspace outright (`Invalid platform channel
  Client-1 ... Workspace doesn't support Client-1 channel for REPL`), reproduced identically via
  raw `databricks jobs run-now` (not a `bundle run` quirk) — a genuine workspace-level
  constraint on the serverless environment version, discovered because this was the **first
  bundle-job task ever executed on this workspace** (every prior live phase drove
  `smelt run --target databricks` directly via Databricks Connect, never through a Job).
- `examples/github_activity/dbx_job/load_next_day.py` — `__file__` fallback to `sys.argv[0]`.
  Under `client: "2"` the launcher runs job files via `exec(compile(f.read(), filename, 'exec'))`
  inside a notebook-style REPL rather than a real `python <file>` process, so `__file__` is
  never injected into globals (`NameError: name '__file__' is not defined`).
- `docs/outcomes/.../phases/11c-volume-probe.md` — the `volume_probe` job's verdict, run clean
  after the `client: "2"` fix: `flock` advisory locking, `os.replace()` rename atomicity, and
  `fsync` are **all honoured** by the Unity Catalog Volume's FUSE layer. Per the plan's
  conditional spec delta, this positive result belongs in `docs-site/docs/guide/targets.md`
  (task 13, not reached) rather than a Known Divergence in `docs/specs/run_state.md`.
- `databricks bundle deploy` succeeded against the live workspace: the managed Volume
  (`workspace.smelt_dogfood.smelt_project`) was created, seeded (`smelt.yml`, `models/`), and
  the daily job's schedule was redeployed at the compressed cadence
  (`0 0/20 * * * ?`, UNPAUSED) and confirmed live, then restored to the committed default before
  this phase ended.

## Decisions

- Did not chase the exact Cargo-fingerprint-caching rule behind the intermittent
  `libduckdb-<hash>.so` bundled-fallback failures once the pattern was confirmed (three
  reproductions, same signature, cleared by wiping the build cache each time) — a
  belt-and-braces `rm -rf` in the build command costs a slower `deploy` but is unambiguously
  correct, and this outcome's licence is "fix what's needed for a run to complete," not audit
  Cargo's fingerprinting.
- Fixed `client: "2"` on **all** job environments, not just `probe_env` — the daily job's
  `loader_env`/`smelt_env` would have hit the identical launch failure on their very first
  scheduled run otherwise (confirmed: the daily job's own first PERIODIC run at the old cadence,
  captured mid-phase via `runs list`, failed with the same `Client-1` error before this fix
  landed).
- Reverted an initial guess (map `DATABRICKS_HOST` onto the loader's `SMELT_DBX_HOST` env var)
  once a full `os.environ` dump from inside a live job task proved `DATABRICKS_HOST` is **not**
  exported into a `spark_python_task`'s process at all — the ambient ADR/spec's assumption
  ("a Databricks job automatically exports `DATABRICKS_HOST`") does not hold for this workspace's
  serverless job runtime. See `## Blocked` for the actual ambient mechanism observed and the
  candidate routes.

## For the next planner

- **The load-bearing finding.** `docs/specs/smelt_yml.md` §"Target shape" and
  `docs/specs/multi_backend.md` §"Connection security" describe the `databricks_job` target's
  ambient form as reading a job-exported `DATABRICKS_HOST` env var with no token. A full
  `os.environ` dump captured from inside a live `spark_python_task` on this workspace shows
  **no `DATABRICKS_HOST` (or any `DATABRICKS_*`/`DBX_*` host var) at all** — the actual ambient
  channel already established by the job launcher is `SPARK_REMOTE` (a full Spark Connect
  URL/token string the Spark Connect client honours automatically when no explicit
  `.remote()`/`.host()` is given). This means:
  - `scripts/dbx-dogfood-loader.py`'s host resolution (`os.environ["SMELT_DBX_HOST"]`, hard
    error if absent) cannot work inside a job task as designed, and every scheduled run's
    `load_next_day` task fails before it reaches loader logic.
  - The `databricks_job` smelt target itself (`host: ${DATABRICKS_HOST}`) rests on the same
    false premise and was never actually exercised by a live job run this session — task 7's
    dependency chain means `smelt_run` never got a chance to prove or disprove it, but it reads
    the identical (missing) env var, so it is very likely to fail identically once `load_next_day`
    is unblocked.
  - This is a design decision, not a patch: either (a) teach both the loader and the Rust
    `SparkBackend`/`DatabricksAdapter` path to build an ambient session with **no explicit host**
    (bare `DatabricksSession.builder.getOrCreate()`, relying on `SPARK_REMOTE` the way plain
    `pyspark.sql.SparkSession.builder.getOrCreate()` already would), which changes
    `crates/smelt-backend-spark/src/lib.rs`'s `SparkFlavor::Databricks` contract (currently
    always `Some(host)`) and `python/smelt/databricks_adapter.py`'s `DatabricksSession.builder
    .host(host)` call (currently unconditional even when ambient); or (b) find and export a real
    host value into the job's environment via a bundle-level mechanism (an `spark_conf`/
    environment variable block referencing `${workspace.host}` or similar) so the existing
    `${DATABRICKS_HOST}`-reading code paths keep working unmodified. Route (a) matches what
    "ambient" is supposed to mean and generalizes to any workspace; route (b) is a narrower
    patch but keeps the existing Rust/Python contract. Recommend spec-first: revisit
    `docs/specs/multi_backend.md` §"Connection security"'s ambient-form claim against this
    measured fact before choosing.
- Every other blocker this phase hit was fixed and is now closed: the `dbfs:` seed path, the
  `client: "1"`→`"2"` launch failure, the `__file__` REPL incompatibility, and the maturin/duckdb
  build staleness. Only the ambient-host gap remains between here and three consecutive
  scheduled runs.
- The Volume FUSE-layer measurement (criterion 11's other open question) is **answered and
  positive** — no further live measurement needed there; only the docs-site write-up (task 13)
  remains, appropriately deferred alongside the rest of tasks 8–13 since they all sit downstream
  of task 7.
- Token TTL is still ~45 min (gpg cache); this phase needed one remint mid-session, as the 11a/11b
  notes predicted.

## Gates

- `cargo test -p smelt-cli --test databricks_bundle` — 14/14 green.
- `bash scripts/dbx-bundle.sh validate` — green.
- `bash .claude/scripts/shellcheck-gate.sh` — green.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full `cargo test`, `example_diagnostics`).
- `bash scripts/dbx-verify.sh` — green (workspace reachable after one `dbx-auth.sh` remint, both
  schemas visible, out-of-scope `CREATE SCHEMA` correctly refused).
- `databricks bundle deploy` (live) — green, including at the compressed cadence and restored to
  the committed default.
- `github_activity_volume_probe` (live) — green: `flock`, `os.replace()`, and `fsync` all
  honoured on the Volume.
- Three consecutive scheduled runs — **not reached**; every scheduled `load_next_day` task fails
  on the ambient-host gap above.

# Phase 11m summary — blocked on the ambient Spark Connect channel shape

**Status: blocked.** Real forward progress landed (four genuine infra defects found and fixed,
each verified live), but the smoke run (task 4) still cannot complete: the deployed `smelt`
binary's subprocess cannot use the serverless job's ambient Spark Connect channel at all.

## Shipped

- `scripts/dbx-bundle.sh`: `seed`'s `SEED_ITEMS` now includes `functions` — it previously synced
  only `smelt.yml` and `models`, so `silver.actor_sessions`'s `smelt.functions.sessionize` call
  was `UnknownSmeltFn` on the deployed Volume even though the project builds cleanly locally.
- `scripts/dbx-wheel-build.sh`: every `maturin build` now passes `--features databricks` —
  `smelt-cli`'s default feature set has no Databricks backend at all; every earlier *live* phase
  used `cargo build --features databricks` directly and never exercised the wheel path, so this
  gap was invisible until now. The aarch64 cross-build additionally needed a real target-arch
  `libpython` to satisfy PyO3's `bindings = "bin"` embedding (abi3-py39's cross-compile fallback
  derives a `-lpython3.9` link name from the ABI floor, not the actual interpreter version) — new
  `ensure_aarch64_python_lib()` downloads a python-build-standalone CPython 3.11
  `aarch64-unknown-linux-gnu` build, symlinks `libpython3.9.so` to the real `.so.1.0` (safe under
  abi3: a newer CPython's export table is a superset), and sets `PYO3_CROSS_LIB_DIR`. Verified
  with `readelf -d`: the resulting binary's `DT_NEEDED` correctly names `libpython3.11.so.1.0`,
  not the placeholder. manylinux's own repair step already excludes `libpython*` from bundling
  (the wheel consumer's own Python provides it), so nothing extra ships.
- `examples/github_activity/resources/github_activity_job.yml`: `smelt_env.dependencies` gained
  `databricks-connect==15.4.5` and `pyarrow` — needed once the databricks feature actually let the
  embedded PyO3 interpreter reach `smelt.databricks_adapter`.
- `examples/github_activity/dbx_job/run_smelt.py`: now sets `PYTHONPATH` to the bundle's synced
  `python/` directory (computed from the running frame's `co_filename`, since `__file__` is
  unavailable under Databricks' `exec(compile(...))` task launcher) so the deployed binary's
  embedded interpreter can `import smelt.databricks_adapter`.
- `crates/smelt-cli/tests/databricks_bundle.rs`: `smelt_env_dependencies_are_arch_scoped` updated
  for the two new dependency entries.

## Decisions

- Fixed all four defects in-pass rather than deferring, since each was small and root-caused
  (functions/ omission, missing feature flag, missing PYTHONPATH, missing deps) — 11k's
  `sync.include` precedent. 2026-09-14.
- Did NOT attempt to fix the fifth, newly-discovered defect (below) in-pass: it is a genuine
  architectural gap, not a one-line omission, and the credential's one-hour window does not give
  room to explore it safely. 2026-09-14.

## Blocked (new, 2026-09-14)

**The deployed binary's subprocess cannot construct a working `DatabricksSession` at all**, even
with `--skip-external-steps` and every dependency now present. `SPARK_REMOTE` reaches the
subprocess's inherited environment (confirmed via a temporary debug print, since removed) but its
value is `unix:///da...` — a Unix domain socket path, Databricks' own internal IPC channel to the
notebook kernel's already-established Spark Connect session — not an `sc://host:port` URL.
`databricks-connect`'s public `DatabricksSession.builder.getOrCreate()` ambient ladder
(`_try_get_notebook_session()` then bare `SPARK_REMOTE`) fails `_try_get_notebook_session()`
(no in-process Databricks kernel objects reachable from a separate OS process) and then chokes on
the `unix://` scheme it cannot parse as `sc://`.

This means the entire "ambient, no host/token" design (`docs/outcomes/
20260912-databricks-dogfood-spine/phases/11d-plan.md`) works fine for `load_next_day.py` — which
runs *inside* the task's own Python kernel process, where `_try_get_notebook_session()` or
in-process binding succeeds — but cannot work for `run_smelt.py`'s pattern of shelling out to a
separately-compiled binary, because that binary's subprocess never gets the internal kernel
context, only inherited environment variables, and the one environment variable that is set is in
a scheme only Databricks' own patched client understands.

Candidate routes for the next planner, none attempted:

1. Run `smelt run` *in-process* inside `run_smelt.py`'s own Python kernel instead of shelling out
   to the compiled binary — e.g. a Python-callable entry point into `smelt-runtime` (would need a
   PyO3 extension-module build, a materially different distribution shape from today's
   `bindings = "bin"` executable).
2. Find and use whatever internal Databricks Python API `_try_get_notebook_session()` itself calls
   (likely `dbruntime`-namespaced), and thread its result through to the `databricks_adapter.py`
   ambient constructor via an env var or file the subprocess *can* read — fragile, version-coupled
   to the exact serverless runtime image.
3. Revisit whether "ambient, no host/token" is really the right posture for a job-task-boundary
   subprocess specifically (as opposed to a notebook-embedded execution model) — the spec's
   measured-false claim it replaced (11d) may need a *second* correction: not "job exports
   DATABRICKS_HOST" but "job exports a raw Spark Connect endpoint," which turns out to also be
   false for a spawned subprocess.

None of these is small. Recommend a dedicated planning pass (not a resume) once picked up.

## Gates

- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, shellcheck, full
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test databricks_bundle --quiet` — 24/24 pass (updated for the new
  `smelt_env.dependencies` entries).
- `cargo test -p smelt-cli --test github_activity_dbx_scheduled --quiet` — 5/5 pass, still
  skip-when-missing (no `11g-runs.json`/`11g-equivalence.json` produced this phase — the smoke run
  never reached a completed pipeline execution).
- No ratchet lowered.
- The deployed bundle (job resource + both wheels + `python/`/`scripts/` sync) is left in sync
  with this commit's code. The committed daily cadence was never compressed this phase (task 4
  never succeeded, so task 5 — compress cadence — was never reached), so no cadence restore is
  needed.

## For the next planner

- Criterion 11 is NOT closed. Rows 5-9 (of this phase's own tasks) never ran.
- Every fix in this phase is real, tested-live progress and should stay landed regardless of how
  the SPARK_REMOTE blocker is eventually resolved.
- The credential was re-minted three times during this phase (gpg-agent cache held throughout);
  budget for the same cadence on the next live attempt.

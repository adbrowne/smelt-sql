"""Job-native wrapper for the Databricks Asset Bundle's `smelt_run` task
(docs/outcomes/20260912-databricks-dogfood-spine/outcome.md criterion 11).

The job's `smelt_env` environment installs `smelt` from the locally-built
wheel declared in `databricks.yml`'s `artifacts:` block. Measured phase 11i,
two layered issues before the wheel actually runs here:

1. The installed console-script binary is NOT on `PATH` for a bare
   `subprocess.run` in this serverless notebook environment (exit 127) even
   though the install itself succeeds — `maturin`'s `bindings = "bin"` places
   it alongside the environment's own Python interpreter
   (`sys.exec_prefix/bin/smelt`), which the notebook's inherited `PATH` does
   not include. `shutil.which` first, so a future environment that does put
   it on PATH keeps working unchanged.
2. Once located and exec'd, the binary itself fails with "error while loading
   shared libraries: libduckdb-<hash>.so: cannot open shared object file" —
   its baked-in RPATH is `$ORIGIN/../smelt_sql.libs` (`readelf -d`), which
   assumes the vendored DuckDB lib installs as a **sibling of `bin/`**
   (`<prefix>/smelt_sql.libs`). This environment's actual site-packages
   layout nests it under `<prefix>/lib/python3.11/site-packages/` instead, so
   the RPATH math is off by the `lib/pythonX.Y/site-packages` prefix and
   never finds it. Importing `smelt_sql` gives its real installed location
   directly (pip always installs a wheel's top-level `smelt_sql.libs/` as a
   sibling of the `smelt_sql/` package dir it repairs, regardless of where
   site-packages itself sits), so `LD_LIBRARY_PATH` is set from that instead
   of trusting the RPATH.

This wrapper only supplies the Volume-resident project directory (so
`.smelt/` persists between scheduled runs) and the ambient `databricks_job`
target (`examples/github_activity/smelt.yml`), which carries no token: the
job's own environment supplies Databricks credentials.

3. Once the smelt binary actually reaches `create_backend`'s Databricks arm,
   its embedded PyO3 interpreter imports `smelt.databricks_adapter`
   (`python/smelt/databricks_adapter.py`) exactly like the local dev
   environment does via `scripts/dbx-dogfood-env.sh`'s PYTHONPATH — but
   nothing set PYTHONPATH for this deployed binary's own child process,
   so the import failed with `ModuleNotFoundError: No module named 'smelt'`
   (measured phase 11m). `databricks.yml`'s `sync.paths` already syncs the
   repo's `python/` directory alongside this bundle root (for
   `load_next_day.py`'s own import of the same module) — computed relative
   to this file's own deployed location rather than hardcoded, since
   `${workspace.file_path}` is a bundle YAML variable with no equivalent at
   Python runtime.
"""

import os
import shutil
import subprocess
import sys


def resolve_smelt_binary() -> str:
    found = shutil.which("smelt")
    if found:
        return found
    candidate = os.path.join(os.path.dirname(sys.executable), "smelt")
    if os.path.isfile(candidate):
        return candidate
    raise SystemExit(
        "smelt binary not found on PATH or alongside the interpreter "
        f"({sys.executable}) — wheel install may have failed silently"
    )


def resolve_duckdb_libs_dir() -> str:
    import smelt_sql

    site_packages = os.path.dirname(os.path.dirname(os.path.abspath(smelt_sql.__file__)))
    libs_dir = os.path.join(site_packages, "smelt_sql.libs")
    if not os.path.isdir(libs_dir):
        raise SystemExit(
            f"expected {libs_dir} (sibling of the smelt_sql package) to hold "
            "the vendored libduckdb — wheel install may have changed layout"
        )
    return libs_dir


def main():
    if len(sys.argv) < 2 or not sys.argv[1]:
        raise SystemExit(
            "run_smelt.py requires the Volume-resident project directory as its "
            "first argument"
        )
    project_dir = sys.argv[1]
    smelt_binary = resolve_smelt_binary()
    libs_dir = resolve_duckdb_libs_dir()

    env = os.environ.copy()
    existing = env.get("LD_LIBRARY_PATH", "")
    env["LD_LIBRARY_PATH"] = f"{libs_dir}:{existing}" if existing else libs_dir

    # This file deploys to `<sync-root>/examples/github_activity/dbx_job/`;
    # `<sync-root>/python` is the repo's `python/` directory, synced by
    # `databricks.yml`'s `sync.paths` alongside the bundle root. Four
    # `dirname` calls walk dbx_job -> github_activity -> examples -> the sync
    # root itself (measured phase 11m: three landed one level short, at
    # `.../files/examples`, not `.../files`). `__file__` is unavailable — the
    # `spark_python_task` launcher runs this script via
    # `exec(compile(f.read(), filename, "exec"))` rather than a normal module
    # import (`NameError: name '__file__' is not defined`), so the deployed
    # absolute path is recovered from the current frame's own `co_filename`
    # instead, which `compile()` stamped from that same `filename`.
    this_file = sys._getframe().f_code.co_filename
    sync_root = os.path.dirname(
        os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(this_file))))
    )
    python_dir = os.path.join(sync_root, "python")
    if not os.path.isdir(python_dir):
        raise SystemExit(
            f"expected {python_dir} (the synced python/ directory) to exist — "
            "databricks.yml's sync.paths may have changed"
        )
    existing_pythonpath = env.get("PYTHONPATH", "")
    env["PYTHONPATH"] = f"{python_dir}:{existing_pythonpath}" if existing_pythonpath else python_dir

    # `smelt.yml` interpolates every target's env-var references eagerly at
    # load time, not just the selected one (fail-loud discipline: an
    # unresolved reference is always a diagnostic, never a silent skip) — so
    # the `databricks`/`databricks_oracle` targets' `${SMELT_DBX_HOSTNAME}`/
    # `${SMELT_DBX_TOKEN}` must resolve to *something* even though
    # `databricks_job` (the target this task actually selects) never reads
    # them. This job's environment carries neither var (11d's ambient-auth
    # design), so supply harmless placeholders rather than widening
    # `smelt.yml` loading to skip unselected targets — a real per-target-scope
    # change with its own spec/CLAUDE.md implications, out of this phase's
    # scope.
    env.setdefault("SMELT_DBX_HOSTNAME", "unused-by-databricks_job-target")
    env.setdefault("SMELT_DBX_TOKEN", "unused-by-databricks_job-target")

    subprocess.run(
        [
            smelt_binary,
            "run",
            "--project-dir",
            project_dir,
            "--target",
            "databricks_job",
            # `--auto` ("process only uncovered intervals since last run")
            # derives the window from the seeded `databricks_job` interval
            # history (11j/11k), which is exactly a scheduled run's job.
            "--auto",
            # `sources.raw.github_loader`'s `command:` is the DuckDB-CLI
            # dev-target loader (`load_day.sh`) — it cannot run against
            # Databricks serverless compute at all (measured phase 11k:
            # `ExternalStepFailed: ... exit code 127`). The separate
            # `load_next_day` task (this job's first task) already loaded
            # the window this run derives above, through the ambient
            # Databricks Connect session `load_next_day.py` requires — the
            # job's own task ordering (`depends_on` in `databricks.yml`) is
            # the freshness guarantee this flag trusts rather than proves
            # (`docs/specs/sources.md` §Semantics 12's named carve-out;
            # human decision recorded in
            # `docs/outcomes/20260912-databricks-dogfood-spine/outcome.md`
            # 2026-09-14).
            "--skip-external-steps",
        ],
        check=True,
        env=env,
    )


if __name__ == "__main__":
    main()

"""Job-native wrapper for the Databricks Asset Bundle's `load_next_day` task
(docs/outcomes/20260912-databricks-dogfood-spine/outcome.md criterion 11).

Databricks Jobs has no shell-command task type, so this thin
`spark_python_task` shells out to the already-tested
scripts/dbx-dogfood-loader.py rather than reimplementing its logic — reuse,
not a second loader. `databricks.yml`'s `sync.paths` puts both
`scripts/` and `python/` alongside this bundle's synced files even though
neither lives under the bundle root, so both are reachable at fixed relative
offsets from this file once deployed.

Passes `--next-day` rather than a date: the fixture holds a fixed historical
range with no relationship to `{{job.trigger.time.iso_date}}`'s real
calendar date, so a scheduled run must advance the fixture by its own
ledger (the live `_loader_days` table) instead of trusting wall-clock time.
"""

import os
import subprocess
import sys

# Databricks' serverless job-environment launcher (`client: "2"`) runs this
# file via `exec(compile(...))` inside a notebook-style REPL rather than a
# real `python <file>` invocation, so `__file__` is not injected into
# globals (measured phase 11c: `NameError: name '__file__' is not defined`
# under client "2", though it worked under the "Invalid platform channel
# Client-1"-rejected client "1"). `sys.argv[0]` still carries the script's
# own deployed path in both launch modes.
_HERE = os.path.dirname(os.path.abspath(globals().get("__file__") or sys.argv[0]))
_BUNDLE_ROOT = os.path.dirname(_HERE)  # examples/github_activity
_SYNC_ROOT = os.path.dirname(os.path.dirname(_BUNDLE_ROOT))  # repo root, once synced
_LOADER = os.path.join(_SYNC_ROOT, "scripts", "dbx-dogfood-loader.py")
_PYTHON_DIR = os.path.join(_SYNC_ROOT, "python")


def main():
    env = dict(os.environ)
    existing = env.get("PYTHONPATH")
    env["PYTHONPATH"] = f"{_PYTHON_DIR}{os.pathsep}{existing}" if existing else _PYTHON_DIR

    subprocess.run([sys.executable, _LOADER, "--next-day"], check=True, env=env)


if __name__ == "__main__":
    main()

"""Job-native wrapper for the Databricks Asset Bundle's `load_next_day` task
(docs/outcomes/20260912-databricks-dogfood-spine/outcome.md criterion 11).

Databricks Jobs has no shell-command task type, so this thin
`spark_python_task` shells out to the already-tested
scripts/dbx-dogfood-loader.py rather than reimplementing its logic — reuse,
not a second loader. `databricks.yml`'s `sync.paths` puts both
`scripts/` and `python/` alongside this bundle's synced files even though
neither lives under the bundle root, so both are reachable at fixed relative
offsets from this file once deployed.
"""

import os
import subprocess
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
_BUNDLE_ROOT = os.path.dirname(_HERE)  # examples/github_activity
_SYNC_ROOT = os.path.dirname(os.path.dirname(_BUNDLE_ROOT))  # repo root, once synced
_LOADER = os.path.join(_SYNC_ROOT, "scripts", "dbx-dogfood-loader.py")
_PYTHON_DIR = os.path.join(_SYNC_ROOT, "python")


def main():
    if len(sys.argv) < 2 or not sys.argv[1]:
        raise SystemExit(
            "load_next_day.py requires the day to load as its first argument "
            "(the job passes {{job.trigger.time.iso_date}})"
        )
    date = sys.argv[1]

    env = dict(os.environ)
    existing = env.get("PYTHONPATH")
    env["PYTHONPATH"] = f"{_PYTHON_DIR}{os.pathsep}{existing}" if existing else _PYTHON_DIR

    subprocess.run([sys.executable, _LOADER, "--date", date], check=True, env=env)


if __name__ == "__main__":
    main()

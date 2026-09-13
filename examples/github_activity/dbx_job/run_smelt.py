"""Job-native wrapper for the Databricks Asset Bundle's `smelt_run` task
(docs/outcomes/20260912-databricks-dogfood-spine/outcome.md criterion 11).

The job's `smelt_env` environment installs `smelt` from the locally-built
wheel declared in `databricks.yml`'s `artifacts:` block, so the binary is
already on PATH here — this wrapper only supplies the Volume-resident project
directory (so `.smelt/` persists between scheduled runs) and the ambient
`databricks_job` target (`examples/github_activity/smelt.yml`), which carries
no token: the job's own environment supplies Databricks credentials.
"""

import subprocess
import sys


def main():
    if len(sys.argv) < 2 or not sys.argv[1]:
        raise SystemExit(
            "run_smelt.py requires the Volume-resident project directory as its "
            "first argument"
        )
    project_dir = sys.argv[1]

    subprocess.run(
        ["smelt", "run", "--project-dir", project_dir, "--target", "databricks_job"],
        check=True,
    )


if __name__ == "__main__":
    main()

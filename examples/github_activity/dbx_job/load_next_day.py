"""Job-native wrapper for the Databricks Asset Bundle's `load_next_day` task
(docs/outcomes/20260912-databricks-dogfood-spine/outcome.md criterion 11).

Databricks Jobs has no shell-command task type, so this thin
`spark_python_task` imports and calls the already-tested
scripts/dbx-dogfood-loader.py's `cmd_next_day()` directly, **in this same
process**, rather than reimplementing its logic — reuse, not a second
loader. `databricks.yml`'s `sync.paths` puts both `scripts/` and `python/`
alongside this bundle's synced files even though neither lives under the
bundle root, so both are reachable at fixed relative offsets from this file
once deployed.

In-process, not `subprocess.run`, is load-bearing (measured live, phase
11e): a serverless job task's ambient Databricks Connect session
(`docs/specs/multi_backend.md` §"Connection security") is bound to the
notebook-style REPL process Databricks itself launches this file's code
in — `databricks.connect`'s ambient ladder first looks for a `spark`/`sc`
object already bound in *this* process's IPython namespace, and its
fallback (`SPARK_REMOTE`) names a local `unix://` domain socket private to
that same process, which plain PySpark's Spark Connect client refuses
outright (`[INVALID_CONNECT_URL] ... must start with 'sc://'`). A child
`subprocess.run` process has neither: it can inherit the `SPARK_REMOTE` env
var by value, but not the process-bound channel it names, and it has no
share of the parent's IPython kernel namespace either. Calling
`cmd_next_day()` in-process instead lets the ambient `DatabricksAdapter()`
construction inside it see the same namespace this file's own code runs in.

Passes no date: the fixture holds a fixed historical range with no
relationship to `{{job.trigger.time.iso_date}}`'s real calendar date, so a
scheduled run must advance the fixture by its own ledger (the live
`_loader_days` table) instead of trusting wall-clock time.
"""

import importlib.util
import os
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
    if _PYTHON_DIR not in sys.path:
        sys.path.insert(0, _PYTHON_DIR)

    # `dbx-dogfood-loader.py`'s filename is not a valid module name (hyphens),
    # so it is loaded by path — the same technique
    # `crates/smelt-cli/tests/dbx_dogfood_loader.rs` already uses to drive it
    # without a real install.
    spec = importlib.util.spec_from_file_location("dbx_dogfood_loader", _LOADER)
    loader_mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loader_mod)
    loader_mod.cmd_next_day()


if __name__ == "__main__":
    main()

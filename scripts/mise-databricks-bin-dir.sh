#!/usr/bin/env bash
#
# Prints the Databricks CLI bin directory to use, preferring an install
# already on PATH, then the location scripts/mise-setup-databricks.sh knows
# about. Prints nothing-but-the-not-yet-existent-location if none is present
# yet (e.g. before `mise run setup-databricks`) — mirrors
# scripts/mise-gcloud-bin-dir.sh's reasoning: an empty PATH entry means the
# current directory, a genuine hazard, so a non-existent directory is the
# safer default every shell simply skips.
set -euo pipefail

if command -v databricks >/dev/null 2>&1; then
  dirname "$(command -v databricks)"
  exit 0
fi

if [ -x "${HOME}/.local/databricks-cli/databricks" ]; then
  echo "${HOME}/.local/databricks-cli"
  exit 0
fi

echo "${HOME}/.local/databricks-cli"

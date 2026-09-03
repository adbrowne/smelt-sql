#!/usr/bin/env bash
#
# Prints the DuckDB shared-library directory to use, checking the same two
# locations (and in the same order) as CLAUDE.md's manual setup snippet.
# Prints nothing if neither is present yet (e.g. before `mise run
# setup-duckdb` has been run) — mise's env template tolerates an empty value.
set -euo pipefail

for d in /usr/local/lib "${HOME}/.local/lib/duckdb"; do
  if [ -e "${d}/libduckdb.so" ]; then
    echo "${d}"
    exit 0
  fi
done

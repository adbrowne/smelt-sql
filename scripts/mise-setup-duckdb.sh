#!/usr/bin/env bash
#
# mise task: setup-duckdb — install the system DuckDB shared library
# (v1.5.4) into ~/.local/lib/duckdb, and the `duckdb` CLI into
# ~/.local/bin, idempotently.
#
# CLAUDE.md's manual setup snippet checks /usr/local/lib first (where CI
# installs it, system-wide) then ~/.local/lib/duckdb (user-local). This task
# only ever writes user-local paths — it never touches /usr/local/lib or
# /usr/local/bin, which need sudo and are CI's job, not a dev machine's.
set -euo pipefail

VERSION="1.5.4"
LIB_DEST_DIR="${HOME}/.local/lib/duckdb"
BIN_DEST_DIR="${HOME}/.local/bin"

if [ -e /usr/local/lib/libduckdb.so ]; then
  echo "libduckdb.so already present system-wide at /usr/local/lib — nothing to do"
elif [ -e "${LIB_DEST_DIR}/libduckdb.so" ]; then
  echo "libduckdb.so already present at ${LIB_DEST_DIR} — nothing to do"
else
  mkdir -p "${LIB_DEST_DIR}"
  TMP_ZIP="$(mktemp -d)/libduckdb.zip"
  curl -sL "https://github.com/duckdb/duckdb/releases/download/v${VERSION}/libduckdb-linux-amd64.zip" -o "${TMP_ZIP}"
  unzip -o "${TMP_ZIP}" libduckdb.so -d "${LIB_DEST_DIR}"
  rm -rf "$(dirname "${TMP_ZIP}")"
  echo "Installed libduckdb.so v${VERSION} to ${LIB_DEST_DIR}"
fi

# The CLI (distinct from the shared library above) is needed by
# examples/github_activity/load_day.sh, the external step
# crates/smelt-cli/tests/github_activity_{loader,replay,oracle}.rs drive
# `smelt run` through.
if command -v duckdb >/dev/null 2>&1; then
  echo "duckdb CLI already on PATH ($(command -v duckdb)) — nothing to do"
elif [ -e "${BIN_DEST_DIR}/duckdb" ]; then
  echo "duckdb CLI already present at ${BIN_DEST_DIR} — nothing to do"
else
  mkdir -p "${BIN_DEST_DIR}"
  TMP_ZIP="$(mktemp -d)/duckdb_cli.zip"
  curl -sL "https://github.com/duckdb/duckdb/releases/download/v${VERSION}/duckdb_cli-linux-amd64.zip" -o "${TMP_ZIP}"
  unzip -o "${TMP_ZIP}" duckdb -d "${BIN_DEST_DIR}"
  chmod +x "${BIN_DEST_DIR}/duckdb"
  rm -rf "$(dirname "${TMP_ZIP}")"
  echo "Installed duckdb CLI v${VERSION} to ${BIN_DEST_DIR}"
fi

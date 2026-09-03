#!/usr/bin/env bash
#
# mise task: setup-duckdb — install the system DuckDB shared library
# (v1.5.4) into ~/.local/lib/duckdb, idempotently.
#
# CLAUDE.md's manual setup snippet checks /usr/local/lib first (where CI
# installs it, system-wide) then ~/.local/lib/duckdb (user-local). This task
# only ever writes the user-local path — it never touches /usr/local/lib,
# which needs sudo and is CI's job, not a dev machine's.
set -euo pipefail

VERSION="1.5.4"
DEST_DIR="${HOME}/.local/lib/duckdb"

if [ -e /usr/local/lib/libduckdb.so ]; then
  echo "libduckdb.so already present system-wide at /usr/local/lib — nothing to do"
  exit 0
fi

if [ -e "${DEST_DIR}/libduckdb.so" ]; then
  echo "libduckdb.so already present at ${DEST_DIR} — nothing to do"
  exit 0
fi

mkdir -p "${DEST_DIR}"
TMP_ZIP="$(mktemp -d)/libduckdb.zip"
curl -sL "https://github.com/duckdb/duckdb/releases/download/v${VERSION}/libduckdb-linux-amd64.zip" -o "${TMP_ZIP}"
unzip -o "${TMP_ZIP}" libduckdb.so -d "${DEST_DIR}"
rm -rf "$(dirname "${TMP_ZIP}")"

echo "Installed libduckdb.so v${VERSION} to ${DEST_DIR}"

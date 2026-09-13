#!/usr/bin/env bash
#
# mise task: setup-databricks — install the Databricks CLI (needed for
# `databricks bundle validate`/`deploy`/`run`, see scripts/dbx-bundle.sh) into
# ~/.local/databricks-cli, idempotently.
#
# Mirrors scripts/mise-setup-gcloud.sh: user-local paths only, never
# /usr/local, and a no-op when a usable install already exists. A TASK rather
# than a [tools] entry for the same reason gcloud is one — every CI job
# installs everything in [tools] via jdx/mise-action@v2, and only the bundle
# job needs this ~15MB download.
set -euo pipefail

VERSION="1.16.1"
DEST_PARENT="${HOME}/.local"
DEST="${DEST_PARENT}/databricks-cli"

if command -v databricks >/dev/null 2>&1; then
  installed="$(databricks --version 2>/dev/null | grep -o '[0-9]\+\.[0-9]\+\.[0-9]\+' | head -n1 || true)"
  if [ "${installed}" = "${VERSION}" ]; then
    echo "databricks CLI ${VERSION} already on PATH ($(command -v databricks)) — nothing to do"
    exit 0
  fi
  echo "databricks CLI on PATH is version ${installed:-unknown}, pin wants ${VERSION} — installing the pinned version to ${DEST}"
fi
if [ -x "${DEST}/databricks" ]; then
  installed="$("${DEST}/databricks" --version 2>/dev/null | grep -o '[0-9]\+\.[0-9]\+\.[0-9]\+' | head -n1 || true)"
  if [ "${installed}" = "${VERSION}" ]; then
    echo "databricks CLI ${VERSION} already present at ${DEST} — nothing to do"
    exit 0
  fi
fi

mkdir -p "${DEST}"
TMP_DIR="$(mktemp -d)"
TARBALL="${TMP_DIR}/databricks-cli.tar.gz"
URL="https://github.com/databricks/cli/releases/download/v${VERSION}/databricks_cli_${VERSION}_linux_amd64.tar.gz"

echo "Downloading Databricks CLI ${VERSION} ..."
curl -sL "${URL}" -o "${TARBALL}"
tar -xzf "${TARBALL}" -C "${DEST}" databricks
chmod +x "${DEST}/databricks"
rm -rf "${TMP_DIR}"

echo "Installed Databricks CLI ${VERSION} to ${DEST}"

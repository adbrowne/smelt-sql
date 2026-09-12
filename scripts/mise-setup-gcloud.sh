#!/usr/bin/env bash
#
# mise task: setup-gcloud — install the Google Cloud SDK (gcloud, bq) into
# ~/.local/google-cloud-sdk, idempotently.
#
# Mirrors scripts/mise-setup-duckdb.sh: user-local paths only, never
# /usr/local, and a no-op when a usable install already exists.
#
# It is a TASK rather than a [tools] entry on purpose. Every workflow runs
# `jdx/mise-action@v2` with no arguments, which installs everything in
# [tools] — and this is a ~200MB download that none of the seven CI jobs
# needs. shellcheck, by contrast, is ~1MB and gates every job, so that one
# IS a [tools] pin. Size and universality are the two questions.
set -euo pipefail

VERSION="580.0.0"
DEST_PARENT="${HOME}/.local"
DEST="${DEST_PARENT}/google-cloud-sdk"

# An existing install anywhere on PATH wins — adopt it rather than duplicate
# it. This is the case on the machine this was written for, where the SDK
# already lived at ~/google-cloud-sdk.
if command -v gcloud >/dev/null 2>&1; then
  echo "gcloud already on PATH ($(command -v gcloud)) — nothing to do"
  exit 0
fi
for d in "${HOME}/google-cloud-sdk" "${DEST}"; do
  if [ -x "${d}/bin/gcloud" ]; then
    echo "Cloud SDK already present at ${d} — nothing to do"
    exit 0
  fi
done

mkdir -p "${DEST_PARENT}"
TMP_DIR="$(mktemp -d)"
TARBALL="${TMP_DIR}/google-cloud-cli.tar.gz"
URL="https://dl.google.com/dl/cloudsdk/channels/rapid/downloads/google-cloud-cli-${VERSION}-linux-x86_64.tar.gz"

echo "Downloading Cloud SDK ${VERSION} ..."
curl -sL "${URL}" -o "${TARBALL}"
tar -xzf "${TARBALL}" -C "${DEST_PARENT}"
rm -rf "${TMP_DIR}"

# --path-update=false: mise's [env] entry resolves the bin directory, so the
# installer must not also edit shell rc files behind the user's back.
"${DEST}/install.sh" --quiet --usage-reporting=false --path-update=false --command-completion=false

echo "Installed Cloud SDK ${VERSION} to ${DEST}"

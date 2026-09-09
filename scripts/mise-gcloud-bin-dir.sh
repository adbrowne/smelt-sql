#!/usr/bin/env bash
#
# Prints the Google Cloud SDK bin directory to use, preferring an install
# already on PATH, then the two locations scripts/mise-setup-gcloud.sh knows
# about. Prints nothing if none is present yet (e.g. before `mise run
# setup-gcloud`) — mise's env template tolerates an empty value.
#
# Exists because `bq` shells out to `gcloud` by NAME, so an absolute path to
# `bq` alone is not enough: the SDK's bin directory has to be on PATH.
set -euo pipefail

if command -v gcloud >/dev/null 2>&1; then
  dirname "$(command -v gcloud)"
  exit 0
fi

for d in "${HOME}/google-cloud-sdk/bin" "${HOME}/.local/google-cloud-sdk/bin"; do
  if [ -x "${d}/gcloud" ]; then
    echo "${d}"
    exit 0
  fi
done

# Nothing installed yet. Print where `mise run setup-gcloud` WILL put it rather
# than printing nothing: this value feeds mise's `_.path`, and an empty entry
# on PATH means the current directory — a genuine hazard, and a much worse
# outcome than a non-existent directory, which every shell simply skips.
echo "${HOME}/.local/google-cloud-sdk/bin"

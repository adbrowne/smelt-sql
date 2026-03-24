#!/usr/bin/env bash
set -euo pipefail

PLAN="docs/plans/20260322-incremental-model-support.md"
BRANCH="incremental-model-support"
TOTAL_PHASES=8
START_PHASE=${1:-1}
END_PHASE=${2:-$TOTAL_PHASES}

echo "Running phases $START_PHASE to $END_PHASE"

for phase in $(seq "$START_PHASE" "$END_PHASE"); do
  echo "=== Starting Phase $phase ==="

  claude -p "Implement Phase $phase of the incremental model support plan.
Master plan: $PLAN (Section 3, Phase $phase)
Branch: keep working on the $BRANCH branch and push to github when done" \
    --dangerously-skip-permissions \
    --output-format stream-json \
    --verbose \
    | tee "phase-${phase}-output.json"
#    | jq -rj 'select(.type == "stream_event" and .event.delta.type? == "text_delta") | .event.delta.text'

  if [ "${PIPESTATUS[0]}" -ne 0 ]; then
    echo "Phase $phase failed, stopping."
    exit 1
  fi

  echo "=== Phase $phase complete ==="
done

echo "All phases complete."

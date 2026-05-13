#!/usr/bin/env bash
#
# Reads .claude/usage-log.jsonl (written by log-tool-call.sh, log-session-end.sh,
# and autonomy-loop.sh) and prints a summary: total tokens by session, biggest
# tool-result outliers, headless-iteration cost (when available).
#
# Usage:
#   bash .claude/scripts/usage-summary.sh            # whole log
#   bash .claude/scripts/usage-summary.sh --since YYYY-MM-DD

set -u

LOG="${CLAUDE_PROJECT_DIR:-$PWD}/.claude/usage-log.jsonl"
if [ ! -f "$LOG" ]; then
  echo "no log at $LOG"; exit 0
fi

SINCE="${1:-}"
if [ "$SINCE" = "--since" ]; then
  SINCE="$2"
fi

filter='.'
if [ -n "$SINCE" ]; then
  filter="select(.ts >= \"$SINCE\")"
fi

echo "=== Sessions (assistant-message usage; missing for headless --no-session-persistence) ==="
jq -c "$filter | select(.event == \"session-end\")" "$LOG" \
  | jq -s 'sort_by(.ts) | .[] | "\(.ts)  \(.session // "?" | .[0:8])  in=\(.input_tokens)  out=\(.output_tokens)  cache_create=\(.cache_creation)  cache_read=\(.cache_read)  msgs=\(.assistant_msgs)"' -r

echo
echo "=== Headless autonomy-loop iterations (token + cost from --output-format json) ==="
jq -c "$filter | select(.event == \"headless-iter\")" "$LOG" \
  | jq -s 'sort_by(.ts) | .[] | "\(.ts)  iter=\(.iter)  cost=$\(.total_cost_usd)  in=\(.input)  out=\(.output)  cache_create=\(.cache_create)  cache_read=\(.cache_read)  dur=\((.duration_ms // 0)/1000|floor)s"' -r

echo
echo "=== Top-10 largest tool results (likely token-spend outliers) ==="
jq -c "$filter | select(.event == \"tool\")" "$LOG" \
  | jq -s 'sort_by(-(.resp_bytes // 0)) | .[:10] | .[] | "\(.resp_bytes) bytes  \(.tool)  \(.cmd // "" | tostring | .[0:80])"' -r

echo
echo "=== Edit responses exceeding threshold (EDIT_SIZE_KB=${EDIT_SIZE_KB:-50}) ==="
# Edit returns the full post-edit file. On large files this dominates cache-read
# cost because the response lingers in transcript and is re-read every turn.
# Aggregate by file to surface "this file is the hot spot — split it or delegate
# its edits to a subagent so the big response stays out of the orchestrator's
# context window".
threshold_bytes=$(( ${EDIT_SIZE_KB:-50} * 1024 ))
jq -c "$filter | select(.event == \"tool\" and .tool == \"Edit\" and (.resp_bytes // 0) >= $threshold_bytes)" "$LOG" \
  | jq -s '
      if length == 0 then "<no Edit responses over threshold>"
      else
        . as $entries
        | ($entries
           | group_by(.cmd)
           | map({
               file: (.[0].cmd // "?"),
               count: length,
               max_kb: ((map(.resp_bytes) | max) / 1024 | floor),
               total_mb: ((map(.resp_bytes) | add) / 1024 / 1024 * 100 | floor / 100)
             })
           | sort_by(-.total_mb)
           | (.[] | "\(.count)x  max=\(.max_kb)KB  total=\(.total_mb)MB  \(.file)")),
          "",
          "SUMMARY: \($entries | length) Edit calls over threshold; \(($entries | map(.resp_bytes) | add) / 1024 / 1024 * 100 | floor / 100) MB total returned"
      end
    ' -r

echo
echo "=== Tool-call counts by tool ==="
jq -c "$filter | select(.event == \"tool\") | .tool" "$LOG" \
  | sort | uniq -c | sort -rn

echo
echo "=== Headless cost totals ==="
jq -c "$filter | select(.event == \"headless-iter\")" "$LOG" \
  | jq -s '{iterations: length, total_cost_usd: ([.[].total_cost_usd // 0] | add), total_input: ([.[].input // 0] | add), total_output: ([.[].output // 0] | add), total_cache_create: ([.[].cache_create // 0] | add), total_cache_read: ([.[].cache_read // 0] | add)}'

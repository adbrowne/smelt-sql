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
echo "=== Tool-call counts by tool ==="
jq -c "$filter | select(.event == \"tool\") | .tool" "$LOG" \
  | sort | uniq -c | sort -rn

echo
echo "=== Headless cost totals ==="
jq -c "$filter | select(.event == \"headless-iter\")" "$LOG" \
  | jq -s '{iterations: length, total_cost_usd: ([.[].total_cost_usd // 0] | add), total_input: ([.[].input // 0] | add), total_output: ([.[].output // 0] | add), total_cache_create: ([.[].cache_create // 0] | add), total_cache_read: ([.[].cache_read // 0] | add)}'

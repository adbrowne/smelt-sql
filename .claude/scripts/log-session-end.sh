#!/usr/bin/env bash
#
# Hook: SessionEnd — appends a per-session token-usage summary to
# .claude/usage-log.jsonl by scanning the transcript file.
#
# In headless runs that use --no-session-persistence, the transcript path
# may not exist on disk by the time this fires; in that case we record a
# "no transcript" marker so the gap is visible. Per-iteration usage for
# headless runs is captured separately by autonomy-loop.sh from the
# --output-format json envelope.

set -u

LOG_DIR="${CLAUDE_PROJECT_DIR:-$PWD}/.claude"
mkdir -p "$LOG_DIR"
LOG="$LOG_DIR/usage-log.jsonl"

INPUT="$(cat)"
TRANSCRIPT="$(printf '%s' "$INPUT" | jq -r '.transcript_path // empty')"
SESSION="$(printf '%s' "$INPUT" | jq -r '.session_id // empty')"
REASON="$(printf '%s' "$INPUT" | jq -r '.reason // empty')"

if [ -z "$TRANSCRIPT" ] || [ ! -f "$TRANSCRIPT" ]; then
  jq -nc \
    --arg s "$SESSION" \
    --arg r "$REASON" \
    '{ts: (now | todate), event: "session-end", session: $s, reason: $r, note: "no-transcript"}' \
    >> "$LOG" 2>/dev/null || true
  exit 0
fi

# Sum usage across all assistant messages in the transcript.
jq -sc \
  --arg s "$SESSION" \
  --arg r "$REASON" \
  '
    [ .[] | select(.type == "assistant") | .message.usage // empty ] as $u
    | {
        ts: (now | todate),
        event: "session-end",
        session: $s,
        reason: $r,
        assistant_msgs: ($u | length),
        input_tokens: ([$u[] | .input_tokens // 0] | add // 0),
        output_tokens: ([$u[] | .output_tokens // 0] | add // 0),
        cache_creation: ([$u[] | .cache_creation_input_tokens // 0] | add // 0),
        cache_read: ([$u[] | .cache_read_input_tokens // 0] | add // 0)
      }
  ' "$TRANSCRIPT" >> "$LOG" 2>/dev/null || true

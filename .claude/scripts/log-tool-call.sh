#!/usr/bin/env bash
#
# Hook: PostToolUse — appends one JSON line per tool call to .claude/usage-log.jsonl.
#
# Captures tool name, a brief identifier of what the call did (bash command,
# file path, agent description, etc.), and the size of the response. The size
# field is the dominant signal for "this tool call burned a lot of tokens".
#
# Reads the hook input JSON from stdin (schema: { session_id, tool_name,
# tool_input, tool_response, ... }) and is fire-and-forget — failures here
# must not block the tool return.

set -u

LOG_DIR="${CLAUDE_PROJECT_DIR:-$PWD}/.claude"
mkdir -p "$LOG_DIR"
LOG="$LOG_DIR/usage-log.jsonl"

jq -c '{
  ts: (now | todate),
  event: "tool",
  session: .session_id,
  tool: .tool_name,
  cmd: (
    .tool_input.command
    // .tool_input.file_path
    // .tool_input.description
    // .tool_input.prompt
    // .tool_input.pattern
    // null
  ),
  resp_bytes: (.tool_response | tostring | length),
  input_bytes: (.tool_input | tostring | length)
}' >> "$LOG" 2>/dev/null || true

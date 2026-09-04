#!/usr/bin/env bash
#
# Pretty-print a `claude --print --output-format stream-json --verbose` JSONL
# stream for interactive/tmux readability. Reads JSONL on stdin, writes
# human-readable lines to stdout. Used by outcome-loop.sh (and reusable by
# autonomy-loop.sh) as a tee'd formatting stage — the raw JSONL still goes to
# the iteration log untouched, this is display-only.
#
# Unrecognized/non-JSON lines pass through as-is so stderr text (crashes,
# 429s) stays visible instead of being silently swallowed.

set -uo pipefail

truncate() {
  # $1 = string, $2 = max length
  local s="$1" max="$2"
  if [ "${#s}" -gt "${max}" ]; then
    printf '%s…' "${s:0:${max}}"
  else
    printf '%s' "${s}"
  fi
}

while IFS= read -r line; do
  type="$(jq -r '.type // empty' <<<"${line}" 2>/dev/null)"

  case "${type}" in
    system)
      subtype="$(jq -r '.subtype // empty' <<<"${line}")"
      if [ "${subtype}" = "init" ]; then
        model="$(jq -r '.model // "?"' <<<"${line}")"
        echo "── session start (model=${model}) ──"
      fi
      ;;
    assistant)
      while IFS= read -r block; do
        [ -z "${block}" ] && continue
        echo "${block}"
      done < <(jq -r '.message.content[]? |
        if .type == "text" then .text
        elif .type == "tool_use" then "→ " + .name + " " + (.input | tostring)
        else empty end' <<<"${line}" 2>/dev/null)
      ;;
    user)
      while IFS= read -r block; do
        [ -z "${block}" ] && continue
        echo "  ⇢ $(truncate "${block}" 300)"
      done < <(jq -r '.message.content[]? | select(.type=="tool_result") |
        (.content // "" | if type=="array" then (map(.text? // "") | join(" ")) else tostring end)
        | gsub("\n"; " ")' <<<"${line}" 2>/dev/null)
      ;;
    result)
      cost="$(jq -r '.total_cost_usd // "?"' <<<"${line}")"
      turns="$(jq -r '.num_turns // "?"' <<<"${line}")"
      dur="$(jq -r '.duration_ms // "?"' <<<"${line}")"
      echo "── result: cost=\$${cost} turns=${turns} duration=${dur}ms ──"
      ;;
    "")
      # Not a JSON line (stray stderr, progress text, etc.) — show as-is.
      echo "${line}"
      ;;
    *)
      # Other event types (rate_limit_event, thinking_tokens, hook_*, …) are
      # noise for this purpose — dropped from the readable view (still in
      # the raw log).
      ;;
  esac
done

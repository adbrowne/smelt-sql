#!/usr/bin/env bash
# Extracts every "§Known Divergences / Open Questions" bullet from the four
# incremental-program anchor specs, as they stood at the 2026-08-15 program
# baseline commit. Emits one TSV line per bullet:
#   spec<TAB>subsection<TAB>bold lead-in<TAB>has_open_question(yes/no)<TAB>baseline line no
#
# Usage: bash extract-baseline.sh
set -euo pipefail

BASELINE_COMMIT="03a431f3"
SPECS=(definition_deltas incremental_models incremental_shapes model_properties)

for spec in "${SPECS[@]}"; do
  git show "${BASELINE_COMMIT}:docs/specs/${spec}.md" | awk -v spec="$spec" '
    function flush() {
      if (bullet_active) {
        # Find the bold lead-in: text between the first and second "**".
        # Collapse whitespace first so a bold span wrapped across a line
        # break (e.g. "...(Open\n  Question)...") still reads as one phrase.
        text = bullet_text
        gsub(/[ \t]+/, " ", text)
        first = index(text, "**")
        lead = ""
        if (first > 0) {
          rest = substr(text, first + 2)
          second = index(rest, "**")
          if (second > 0) {
            lead = substr(rest, 1, second - 1)
          } else {
            lead = rest
          }
        }
        has_oq = (text ~ /[Oo]pen [Qq]uestion/) ? "yes" : "no"
        gsub(/\t/, " ", lead)
        print spec "\t" subsection "\t" lead "\t" has_oq "\t" bullet_line
      }
      bullet_active = 0
      bullet_text = ""
    }
    BEGIN { in_section = 0; subsection = "-"; bullet_active = 0 }
    /^## / {
      if ($0 ~ /^## Known Divergences/) {
        flush()
        in_section = 1
        subsection = "-"
        next
      } else if (in_section) {
        flush()
        in_section = 0
        next
      }
    }
    in_section && /^### / {
      flush()
      subsection = $0
      sub(/^### /, "", subsection)
      next
    }
    in_section && /^- \*\*/ {
      flush()
      bullet_active = 1
      bullet_line = NR
      line = $0
      sub(/^- /, "", line)
      bullet_text = line
      next
    }
    in_section && bullet_active && /^  / {
      bullet_text = bullet_text " " $0
      next
    }
    in_section && bullet_active && /^$/ {
      next
    }
    END { flush() }
  '
done

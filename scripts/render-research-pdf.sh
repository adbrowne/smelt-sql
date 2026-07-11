#!/usr/bin/env bash
# Render a research/spec markdown doc to an academic-styled PDF.
#
# Usage:
#   bash scripts/render-research-pdf.sh [input.md] [output.pdf]
#
# Defaults to docs/research/20260703-model-updates.md, writing the PDF
# alongside the input. Requires pandoc + xelatex (texlive) with the
# Latin Modern OpenType fonts (shipped with texlive) and DejaVu Sans Mono.
#
# Styling notes:
# - Body is Latin Modern Roman (the OpenType Computer Modern successor —
#   the classic academic-paper look); code is DejaVu Sans Mono, chosen for
#   its coverage of box-drawing and math glyphs inside code blocks.
# - Unicode math symbols not present in Latin Modern text fonts are mapped
#   to proper LaTeX glyphs. The mappings are wrapped in \AtBeginDocument
#   because pandoc's template loads unicode-math, which re-registers some
#   of these characters at begin-document and would otherwise clobber the
#   mappings (the prime U+2032 then crashes inside \texttt).
# - yaml_metadata_block is disabled: research docs use bare `---` horizontal
#   rules that pandoc would otherwise try to parse as YAML metadata.
# - Long tables are set at \scriptsize so the wide comparison matrices fit.
# - Headings must be preceded by a blank line: CommonMark accepts a heading
#   directly after a `---` rule, but pandoc parses it as literal text.

set -euo pipefail

IN="${1:-docs/research/20260703-model-updates.md}"
OUT="${2:-${IN%.md}.pdf}"

if [[ ! -f "$IN" ]]; then
  echo "error: input not found: $IN" >&2
  exit 1
fi

# Title from the doc's H1; author/date from git / today unless overridden.
AUTHOR="${AUTHOR:-$(git config user.name 2>/dev/null || echo "")}"
DATE="${DATE:-$(date +%Y-%m-%d)}"

PREAMBLE="$(mktemp --suffix=.tex)"
trap 'rm -f "$PREAMBLE"' EXIT

cat > "$PREAMBLE" <<'EOF'
\usepackage{newunicodechar}
\usepackage{amssymb}
\usepackage{pifont}
\usepackage{etoolbox}
% shrink tables so the wide comparison matrices fit
\AtBeginEnvironment{longtable}{\scriptsize}
% keep code blocks from running off the page edge too aggressively
\usepackage{fvextra}
\fvset{breaklines=true, breakanywhere=true}
% unicode math symbols not present in Latin Modern text fonts;
% deferred to begin-document so unicode-math cannot clobber them
\AtBeginDocument{\newunicodechar{±}{\ensuremath{\pm}}}
\AtBeginDocument{\newunicodechar{·}{\ensuremath{\cdot}}}
\AtBeginDocument{\newunicodechar{×}{\ensuremath{\times}}}
\AtBeginDocument{\newunicodechar{÷}{\ensuremath{\div}}}
\AtBeginDocument{\newunicodechar{Δ}{\ensuremath{\Delta}}}
\AtBeginDocument{\newunicodechar{π}{\ensuremath{\pi}}}
\AtBeginDocument{\newunicodechar{σ}{\ensuremath{\sigma}}}
\AtBeginDocument{\newunicodechar{ω}{\ensuremath{\omega}}}
\AtBeginDocument{\newunicodechar{′}{\ensuremath{{}^{\prime}}}}
\AtBeginDocument{\newunicodechar{←}{\ensuremath{\leftarrow}}}
\AtBeginDocument{\newunicodechar{→}{\ensuremath{\rightarrow}}}
\AtBeginDocument{\newunicodechar{↔}{\ensuremath{\leftrightarrow}}}
\AtBeginDocument{\newunicodechar{↦}{\ensuremath{\mapsto}}}
\AtBeginDocument{\newunicodechar{∈}{\ensuremath{\in}}}
\AtBeginDocument{\newunicodechar{−}{\ensuremath{-}}}
\AtBeginDocument{\newunicodechar{∖}{\ensuremath{\setminus}}}
\AtBeginDocument{\newunicodechar{∘}{\ensuremath{\circ}}}
\AtBeginDocument{\newunicodechar{∞}{\ensuremath{\infty}}}
\AtBeginDocument{\newunicodechar{∩}{\ensuremath{\cap}}}
\AtBeginDocument{\newunicodechar{∪}{\ensuremath{\cup}}}
\AtBeginDocument{\newunicodechar{≈}{\ensuremath{\approx}}}
\AtBeginDocument{\newunicodechar{≠}{\ensuremath{\neq}}}
\AtBeginDocument{\newunicodechar{≡}{\ensuremath{\equiv}}}
\AtBeginDocument{\newunicodechar{≤}{\ensuremath{\leq}}}
\AtBeginDocument{\newunicodechar{≥}{\ensuremath{\geq}}}
\AtBeginDocument{\newunicodechar{⊂}{\ensuremath{\subset}}}
\AtBeginDocument{\newunicodechar{⊆}{\ensuremath{\subseteq}}}
\AtBeginDocument{\newunicodechar{⊇}{\ensuremath{\supseteq}}}
\AtBeginDocument{\newunicodechar{⊉}{\ensuremath{\nsupseteq}}}
\AtBeginDocument{\newunicodechar{⊎}{\ensuremath{\uplus}}}
\AtBeginDocument{\newunicodechar{⊕}{\ensuremath{\oplus}}}
\AtBeginDocument{\newunicodechar{⊖}{\ensuremath{\ominus}}}
\AtBeginDocument{\newunicodechar{⋈}{\ensuremath{\bowtie}}}
\AtBeginDocument{\newunicodechar{⟹}{\ensuremath{\Longrightarrow}}}
\AtBeginDocument{\newunicodechar{✓}{\ding{51}}}
\AtBeginDocument{\newunicodechar{✗}{\ding{55}}}
\AtBeginDocument{\newunicodechar{✅}{\ding{51}}}
\AtBeginDocument{\newunicodechar{❌}{\ding{55}}}
EOF

pandoc "$IN" \
  -f markdown-yaml_metadata_block \
  -o "$OUT" \
  --pdf-engine=xelatex \
  --shift-heading-level-by=-1 \
  --toc --toc-depth=1 \
  -H "$PREAMBLE" \
  -V mainfont="Latin Modern Roman" \
  -V sansfont="Latin Modern Sans" \
  -V monofont="DejaVu Sans Mono" \
  -V monofontoptions="Scale=0.82" \
  -V fontsize=11pt \
  -V geometry:margin=2.2cm \
  -V colorlinks=true -V linkcolor=NavyBlue -V urlcolor=NavyBlue \
  ${AUTHOR:+-M author="$AUTHOR"} \
  -M date="$DATE"

echo "wrote $OUT ($(pdfinfo "$OUT" 2>/dev/null | awk '/^Pages:/{print $2}') pages)"

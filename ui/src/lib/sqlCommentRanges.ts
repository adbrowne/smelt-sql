/**
 * Client-side heuristic comment-span detector for the model diagnostics
 * SQL viewers (`docs/specs/ui_model_diagnostics.md` §Semantics "Comment
 * folding"). Returns byte offset ranges for `--` line comments and
 * `/* … *\/` block comments so `SqlViewer` can fold them, rather than
 * deleting their text. Adjacent comment ranges separated only by
 * whitespace (consecutive `--` lines, a run of `--` lines after a block
 * comment, etc.) are merged into a single range, so a header made of
 * several `--` lines folds as one unit instead of one fold per line.
 *
 * This is a regex-based approximation, not a lexer: unlike
 * `smelt-parser`'s token stream, it does not track string-literal context,
 * so a `--`/`/* … *\/` sequence *inside* a string literal is (incorrectly)
 * treated as a comment. This is an acceptable, documented divergence for a
 * read-only display convenience — it never feeds any admission or
 * correctness decision (the "advisory heuristic" carve-out in `CLAUDE.md`
 * §"Property composition walk rule" applies by analogy: this function
 * never feeds anything but a UI fold range).
 */
export interface CommentRange {
  from: number
  to: number
}

export function findSqlCommentRanges(sql: string): CommentRange[] {
  const blockRanges: CommentRange[] = []
  const blockRe = /\/\*[\s\S]*?\*\//g
  let match: RegExpExecArray | null
  while ((match = blockRe.exec(sql)) !== null) {
    blockRanges.push({ from: match.index, to: match.index + match[0].length })
  }

  // Line comments are only counted outside any block comment's span, so a
  // `--` sequence embedded in a block comment's text doesn't produce a
  // second, overlapping range.
  const lineRanges: CommentRange[] = []
  const lineRe = /--[^\n]*/g
  while ((match = lineRe.exec(sql)) !== null) {
    const from = match.index
    const insideBlock = blockRanges.some((b) => from >= b.from && from < b.to)
    if (!insideBlock) {
      lineRanges.push({ from, to: from + match[0].length })
    }
  }

  const sorted = [...blockRanges, ...lineRanges].sort((a, b) => a.from - b.from)
  return mergeAdjacentRanges(sql, sorted)
}

/** Merge ranges whose only separation is whitespace (i.e. nothing but a
 * line break, and possibly leading indentation, sits between them) into
 * one contiguous range. */
function mergeAdjacentRanges(sql: string, ranges: CommentRange[]): CommentRange[] {
  if (ranges.length === 0) return ranges

  const merged: CommentRange[] = [ranges[0]]
  for (let i = 1; i < ranges.length; i++) {
    const prev = merged[merged.length - 1]
    const curr = ranges[i]
    const gap = sql.slice(prev.to, curr.from)
    if (/^\s*$/.test(gap)) {
      merged[merged.length - 1] = { from: prev.from, to: curr.to }
    } else {
      merged.push(curr)
    }
  }
  return merged
}

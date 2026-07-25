/**
 * Client-side heuristic mirror of `smelt_parser::strip_sql_comments`
 * (`crates/smelt-parser/src/strip_comments.rs`, `docs/specs/
 * ui_model_diagnostics.md` §Semantics "Comment stripping").
 *
 * The `ModelDiagnostics` API response (`docs/specs/ui_model_diagnostics.md`
 * §Surface "smelt-runtime builder") does not carry a pre-stripped SQL
 * variant — the builder returns exactly one SQL string per statement/model,
 * not a comment-stripped copy alongside it. The page's "Remove comments"
 * toggle (§Surface "UI page") therefore strips client-side against the raw
 * SQL text it already has, rather than requesting a second server-derived
 * variant for a purely cosmetic display toggle.
 *
 * This is a regex-based approximation, not a lexer: unlike the real
 * `smelt-parser` token stream, it does not track string-literal context, so
 * a `--`/`/* … *\/` sequence *inside* a string literal is (incorrectly)
 * treated as a comment. This is an acceptable, documented divergence for a
 * read-only display convenience — it never feeds any admission or
 * correctness decision (the "advisory heuristic" carve-out in `CLAUDE.md`
 * §"Property composition walk rule" applies by analogy: this function never
 * feeds anything but a UI toggle).
 */
export function stripSqlComments(sql: string): string {
  // Strip /* ... */ block comments (non-nested — the CodeMirror display
  // toggle does not need the lexer's nesting-aware handling).
  let out = sql.replace(/\/\*[\s\S]*?\*\//g, '')
  // Strip -- line comments through end of line, preserving the newline.
  out = out.replace(/--[^\n]*/g, '')
  return out
}

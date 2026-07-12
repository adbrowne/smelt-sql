/// Parser implementation with error recovery
use crate::lexer::{tokenize, Token};
use crate::syntax_kind::{SmeltLanguage, SyntaxKind};
use crate::SyntaxKind::*;
use rowan::{GreenNode, GreenNodeBuilder, TextRange};

/// Result of parsing
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parse {
    pub green_node: GreenNode,
    pub errors: Vec<ParseError>,
}

impl Parse {
    pub fn syntax(&self) -> rowan::SyntaxNode<SmeltLanguage> {
        rowan::SyntaxNode::new_root(self.green_node.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub range: TextRange,
}

/// Is `name` one of the recognised SelectItems kind keywords
/// (Scalar / Agg / Window)? Used by `parse_selectitems_tail` to
/// distinguish the single-argument `SelectItems<Kind>` form from the
/// single-argument `SelectItems<ctx>` form.
pub(super) fn is_selectitems_kind_name(name: &str) -> bool {
    matches!(name, "Scalar" | "Agg" | "Window")
}

/// Parse input text into a CST
pub fn parse(input: &str) -> Parse {
    let tokens = tokenize(input);
    let mut parser = Parser::new(input, &tokens);
    parser.parse_file();
    parser.finish()
}

/// Parse a generator file body as a single top-level meta-language expression.
///
/// `stripped_text` is the full source file text with frontmatter replaced by
/// `-- ` comment lines (same byte length as the original), as returned by
/// [`smelt_parser::strip_frontmatter`]. The body starts at `body_offset`
/// bytes into `stripped_text`; tokens before that offset are trivia produced
/// by the comment-replacement and are consumed silently.
///
/// Returns a CST rooted at a `FILE` node whose single non-trivia child is the
/// parsed expression. If the first non-trivia token at or after `body_offset`
/// is `SELECT`, `WITH`, or `VALUES`, the parser wraps it in a `SELECT_STMT`
/// and sets a `bare_sql_at_body` flag on the `Parse` result so that callers
/// can emit `GenerateFileBareSelectForbidden` with the correct span.
///
/// Line/column information in the resulting CST nodes is accurate relative to
/// the full source file because `stripped_text` preserves byte offsets.
pub fn parse_meta_expression_from_offset(stripped_text: &str, body_offset: usize) -> Parse {
    let tokens = tokenize(stripped_text);
    let mut parser = Parser::new(stripped_text, &tokens);
    parser.parse_generator_body(body_offset);
    parser.finish()
}

/// Maximum nesting depth for recursive parse functions.
/// Prevents stack overflow on adversarial or deeply nested input.
const MAX_PARSE_DEPTH: u32 = 256;

struct Parser<'a> {
    pub(super) input: &'a str,
    pub(super) tokens: &'a [Token],
    pub(super) pos: usize,
    pub(super) offset: usize,
    pub(super) builder: GreenNodeBuilder<'static>,
    pub(super) errors: Vec<ParseError>,
    pub(super) depth: u32,
    /// Named row-variable names collected while parsing the current
    /// `smelt.define` param list (Phase 35). Reset at the start of each
    /// `smelt.define`; checked after the param list to enforce the v1
    /// constraint that at most one distinct name may appear.
    pub(super) current_define_row_vars: Vec<String>,
    /// Whether the current argument-list parse is inside a `smelt.<path>(...)`
    /// call. When true, `parse_argument` admits generic type expressions
    /// (`List<T>`, `Map<K, V>`, `{f: T}`) as arguments so loader schema
    /// arguments parse correctly. When false (regular SQL function calls),
    /// `IDENT < IDENT` is parsed as a comparison expression, not as a
    /// generic type.
    pub(super) in_smelt_call_args: bool,
    /// Whether the parser is currently inside a pipe SQL stage body.
    ///
    /// When `true`, `parse_pipe_expr` must NOT fold `|>` into a `PIPE_EXPR`
    /// meta-language expression node.  The `|>` token at this position is the
    /// next pipe-SQL stage delimiter, not an infix meta-language pipe operator.
    pub(super) in_pipe_stage: bool,
    /// Whether the parser is currently parsing the timezone (right-hand)
    /// operand of an `AT TIME ZONE` postfix expression.
    ///
    /// When `true`, `parse_primary_expr`'s own `AT TIME ZONE` postfix loop
    /// must not fire for a subsequent `AT TIME ZONE` sequence — otherwise a
    /// chain like `ts AT TIME ZONE 'UTC' AT TIME ZONE 'EST'` would parse as
    /// `ts AT TIME ZONE ('UTC' AT TIME ZONE 'EST')` (right-nested inside the
    /// timezone operand) instead of the correct left-associative `(ts AT TIME
    /// ZONE 'UTC') AT TIME ZONE 'EST'` (verified via the DuckDB oracle). The
    /// outer postfix `while` loop in `parse_primary_expr` is what actually
    /// consumes the second `AT TIME ZONE`, so the inner (operand) parse must
    /// stop short of it.
    pub(super) in_at_time_zone_operand: bool,
}

impl<'a> Parser<'a> {
    pub(super) fn new(input: &'a str, tokens: &'a [Token]) -> Self {
        Self {
            input,
            tokens,
            pos: 0,
            offset: 0,
            builder: GreenNodeBuilder::new(),
            errors: Vec::new(),
            depth: 0,
            current_define_row_vars: Vec::new(),
            in_smelt_call_args: false,
            in_pipe_stage: false,
            in_at_time_zone_operand: false,
        }
    }

    pub(super) fn finish(self) -> Parse {
        Parse {
            green_node: self.builder.finish(),
            errors: self.errors,
        }
    }

    /// Current token kind
    pub(super) fn current(&self) -> SyntaxKind {
        self.tokens.get(self.pos).map(|t| t.kind).unwrap_or(EOF)
    }

    /// Check if at specific token kind
    pub(super) fn at(&self, kind: SyntaxKind) -> bool {
        self.current() == kind
    }

    /// Check if at any of the given kinds
    pub(super) fn at_any(&self, kinds: &[SyntaxKind]) -> bool {
        kinds.contains(&self.current())
    }

    /// Get the text of the current token
    pub(super) fn current_text(&self) -> &str {
        if let Some(token) = self.tokens.get(self.pos) {
            &self.input[self.offset..self.offset + token.len]
        } else {
            ""
        }
    }

    /// Check if current token is an IDENT with specific text (case-insensitive)
    pub(super) fn at_contextual_keyword(&self, text: &str) -> bool {
        self.at(IDENT) && self.current_text().eq_ignore_ascii_case(text)
    }

    /// Check if current token is a double-quoted identifier lexed as STRING
    /// (`"foo"`), usable anywhere an alias IDENT is expected. DuckDB and
    /// PostgreSQL both treat double-quoted text as a quoted identifier while
    /// single-quoted text is a string literal, but smelt's lexer does not
    /// distinguish the two at the token-kind level (`consume_string` returns
    /// `STRING` for either quote character) — alias sites that want to
    /// accept `AS "alias"` must check the leading quote character
    /// themselves via this helper.
    pub(super) fn at_quoted_ident_alias(&self) -> bool {
        self.at(STRING) && self.current_text().starts_with('"')
    }

    /// Advance to next token, consuming trivia
    pub(super) fn advance(&mut self) {
        if self.pos < self.tokens.len() {
            let token = self.tokens[self.pos];
            let text = &self.input[self.offset..self.offset + token.len];

            // Report lexer errors with the actual invalid character
            if token.kind == ERROR {
                let display_text = if text.len() <= 10 {
                    text.to_string()
                } else {
                    // Truncate at a char boundary at or before byte 10 —
                    // `text` may contain multi-byte UTF-8 characters (an
                    // ERROR token can now span a whole malformed
                    // number+identifier blob), so a naive byte index can
                    // land inside a multi-byte character and panic.
                    let cut = (0..=10)
                        .rev()
                        .find(|&i| text.is_char_boundary(i))
                        .unwrap_or(0);
                    format!("{}...", &text[..cut])
                };
                let message = format!("Unexpected character: '{}'", display_text);
                let start = self.offset as u32;
                let end = (self.offset + token.len) as u32;
                self.errors.push(ParseError {
                    message,
                    range: TextRange::new(start.into(), end.into()),
                });
            }

            self.builder.token(token.kind.into(), text);
            self.offset += token.len;
            self.pos += 1;
        }
    }

    /// Skip trivia (whitespace, comments)
    pub(super) fn skip_trivia(&mut self) {
        while self.current().is_trivia() {
            self.advance();
        }
    }

    /// Expect a specific token kind, report error if not present
    pub(super) fn expect(&mut self, kind: SyntaxKind) -> bool {
        self.skip_trivia();
        if self.at(kind) {
            self.advance();
            true
        } else {
            self.error(format!("Expected {:?}, found {:?}", kind, self.current()));
            false
        }
    }

    /// Start a composite node
    pub(super) fn start_node(&mut self, kind: SyntaxKind) {
        self.builder.start_node(kind.into());
    }

    /// Start a composite node at a checkpoint (for lookahead/backtracking)
    pub(super) fn start_node_at(&mut self, checkpoint: rowan::Checkpoint, kind: SyntaxKind) {
        self.builder.start_node_at(checkpoint, kind.into());
    }

    /// Finish current node
    pub(super) fn finish_node(&mut self) {
        self.builder.finish_node();
    }

    /// Report a parse error
    pub(super) fn error(&mut self, message: String) {
        let start = self.offset as u32;
        let end = (self.offset + self.tokens.get(self.pos).map(|t| t.len).unwrap_or(0)) as u32;
        self.errors.push(ParseError {
            message,
            range: TextRange::new(start.into(), end.into()),
        });
    }

    /// Check if we've exceeded the maximum nesting depth.
    /// Returns true if too deep (caller should bail out).
    pub(super) fn too_deep(&mut self) -> bool {
        if self.depth >= MAX_PARSE_DEPTH {
            self.error("Expression nesting depth exceeds maximum of 256".to_string());
            true
        } else {
            false
        }
    }

    /// Synchronize to one of the given tokens (error recovery)
    pub(super) fn sync_to(&mut self, kinds: &[SyntaxKind]) {
        while !self.at(EOF) && !self.at_any(kinds) {
            self.start_node(ERROR);
            self.advance();
            self.finish_node();
        }
    }

    /// Check if current token is a keyword that would end a table reference
    pub(super) fn at_keyword_that_ends_table_ref(&self) -> bool {
        // Keywords that can follow a table reference in the FROM clause
        self.at_any(&[
            WHERE_KW,
            GROUP_KW,
            HAVING_KW,
            QUALIFY_KW,
            ORDER_KW,
            LIMIT_KW,
            OFFSET_KW,
            // JOIN keywords
            JOIN_KW,
            INNER_KW,
            LEFT_KW,
            RIGHT_KW,
            FULL_KW,
            CROSS_KW,
            // PIVOT/UNPIVOT
            PIVOT_KW,
            UNPIVOT_KW,
            // Set operations
            UNION_KW,
            INTERSECT_KW,
            EXCEPT_KW,
        ]) || self.at_contextual_keyword("FETCH")
            || self.at_contextual_keyword("NATURAL")
    }

    /// Check if current token can start an expression
    pub(super) fn at_expression_start(&self) -> bool {
        // Phase B (meta-language): `fn` is a reserved keyword (FN_KW) and can
        // start a lambda expression when encountered in expression position.
        self.at_any(&[
            IDENT, NUMBER, STRING, LPAREN, NOT_KW, CASE_KW, CAST_KW, EXTRACT_KW, EXISTS_KW,
            ARRAY_KW, ROW_KW, STRUCT_KW, MINUS, LBRACE, FN_KW,
        ])
    }

    // ===== Map method name allowlist =====

    /// Returns true when `name` is a recognised Map<K,V> API method name.
    /// The spec defines a **closed** five-method API: `{entries, keys,
    /// values, get, has}`. Only these names are routed through the
    /// `MAP_METHOD_CALL` node kind; any other dotted-method call falls
    /// through to `FUNCTION_CALL`, which avoids misclassifying
    /// `db.remove(x)` (where `db` is a non-Map identifier) as a Map API
    /// call. Type inference dispatches `MAP_METHOD_CALL` against the
    /// receiver type and reports `MapApiUnknown` only when the receiver
    /// is actually a `Map<K, V>`.
    pub(super) fn is_map_method_name(name: &str) -> bool {
        matches!(name, "entries" | "get" | "has" | "keys" | "values")
    }

    /// Peek ahead from the current position (which must be at a `DOT` token)
    /// to check whether the token stream looks like `.map_method(`.
    ///
    /// Returns `true` iff:
    ///   1. The current token is `DOT`.
    ///   2. The next non-trivia token after the DOT is an `IDENT` whose text
    ///      is a recognised Map API method name (entries, keys, values, get, has).
    ///   3. The non-trivia token after that IDENT is `LPAREN`.
    ///
    /// Used by the postfix MAP_METHOD_CALL loop in `parse_primary_expr` to
    /// decide whether to commit before consuming the DOT.
    pub(super) fn peek_dot_map_method_call(&self) -> bool {
        // Current token must be DOT.
        if !self.at(DOT) {
            return false;
        }
        // Skip trivia after DOT to find the method-name IDENT.
        let mut la = 1usize;
        while let Some(t) = self.tokens.get(self.pos + la) {
            if t.kind.is_trivia() {
                la += 1;
            } else {
                break;
            }
        }
        let Some(ident_tok) = self.tokens.get(self.pos + la) else {
            return false;
        };
        if ident_tok.kind != IDENT {
            return false;
        }
        // Compute the byte offset of the IDENT so we can read its text.
        let mut ident_offset = self.offset;
        for i in 0..la {
            ident_offset += self.tokens[self.pos + i].len;
        }
        let name = &self.input[ident_offset..ident_offset + ident_tok.len];
        if !Self::is_map_method_name(name) {
            return false;
        }
        // Skip trivia after the IDENT to find the LPAREN.
        let mut la2 = la + 1;
        while let Some(t) = self.tokens.get(self.pos + la2) {
            if t.kind.is_trivia() {
                la2 += 1;
            } else {
                break;
            }
        }
        matches!(
            self.tokens.get(self.pos + la2).map(|t| t.kind),
            Some(LPAREN)
        )
    }

    /// Peek ahead from the current position (which must be at an `IDENT`
    /// token) to check whether the token stream looks like the postfix
    /// `AT TIME ZONE` sequence.
    ///
    /// Returns `true` iff the current token and the next two non-trivia
    /// tokens are `IDENT`s with text (case-insensitively) `AT`, `TIME`,
    /// `ZONE` in that order. Does not consume any tokens — `AT`, `TIME`, and
    /// `ZONE` are all contextual (unreserved) keywords, so a bare `AT` not
    /// followed by `TIME ZONE` must be left alone (e.g. to be consumed later
    /// as an implicit select-item alias).
    pub(super) fn peek_at_time_zone(&self) -> bool {
        if !self.at_contextual_keyword("AT") {
            return false;
        }

        // Find the next non-trivia token after the current AT token.
        let mut la = 1usize;
        while let Some(t) = self.tokens.get(self.pos + la) {
            if t.kind.is_trivia() {
                la += 1;
            } else {
                break;
            }
        }
        let Some(time_tok) = self.tokens.get(self.pos + la) else {
            return false;
        };
        if time_tok.kind != IDENT {
            return false;
        }
        let mut time_offset = self.offset;
        for i in 0..la {
            time_offset += self.tokens[self.pos + i].len;
        }
        let time_text = &self.input[time_offset..time_offset + time_tok.len];
        if !time_text.eq_ignore_ascii_case("TIME") {
            return false;
        }

        // Find the next non-trivia token after TIME.
        let mut la2 = la + 1;
        while let Some(t) = self.tokens.get(self.pos + la2) {
            if t.kind.is_trivia() {
                la2 += 1;
            } else {
                break;
            }
        }
        let Some(zone_tok) = self.tokens.get(self.pos + la2) else {
            return false;
        };
        if zone_tok.kind != IDENT {
            return false;
        }
        let mut zone_offset = self.offset;
        for i in 0..la2 {
            zone_offset += self.tokens[self.pos + i].len;
        }
        let zone_text = &self.input[zone_offset..zone_offset + zone_tok.len];
        zone_text.eq_ignore_ascii_case("ZONE")
    }

    /// Look ahead `n` non-trivia tokens from the current position (0 = the
    /// current token itself) and return its text, without consuming
    /// anything. Used by stateless contextual-keyword lookaheads (e.g.
    /// [`peek_grouping_sets_clause`](Self::peek_grouping_sets_clause)) that
    /// need to inspect a fixed sequence of tokens before committing to a
    /// grammar path. Companion to [`peek_nth_non_trivia`](Self::peek_nth_non_trivia)
    /// (`parser/types.rs`), which returns only the token kind.
    pub(super) fn peek_nth_non_trivia_text(&self, n: usize) -> Option<&str> {
        let mut la = 0usize;
        let mut seen = 0usize;
        loop {
            let tok = self.tokens.get(self.pos + la)?;
            if tok.kind.is_trivia() {
                la += 1;
                continue;
            }
            if seen == n {
                let mut offset = self.offset;
                for i in 0..la {
                    offset += self.tokens[self.pos + i].len;
                }
                return Some(&self.input[offset..offset + tok.len]);
            }
            seen += 1;
            la += 1;
        }
    }

    /// Stateless lookahead for `GROUPING SETS (` at a GROUP BY list position.
    /// `GROUPING` and `SETS` are both contextual keywords (lexed as plain
    /// `IDENT`); this only recognises the clause when the exact three-token
    /// sequence `GROUPING SETS (` appears, so `grouping`/`sets` stay usable
    /// as ordinary identifiers everywhere else (`SELECT grouping FROM t`,
    /// `GROUP BY grouping, sets`). Pure lookahead — consumes nothing, holds
    /// no parser-state flag, so it is safe to call from any expression-ladder
    /// entry point without a reset obligation.
    pub(super) fn peek_grouping_sets_clause(&self) -> bool {
        if !self.at_contextual_keyword("GROUPING") {
            return false;
        }
        let Some(sets_text) = self.peek_nth_non_trivia_text(1) else {
            return false;
        };
        if !sets_text.eq_ignore_ascii_case("SETS") {
            return false;
        }
        matches!(self.peek_nth_non_trivia(2), Some(LPAREN))
    }

    // Domain-specific parsing methods live in submodules:
    //   - parser::smelt_ext (smelt.* extensions)
    //   - parser::types     (type refs, records, struct types)
    //   - parser::select    (SELECT, FROM, WHERE, ...)
    //   - parser::expr      (expression precedence, primaries, ...)
    //   - parser::meta      (meta-language lambdas)
}

mod expr;
mod meta;
mod pipe;
mod select;
mod smelt_ext;
mod types;

#[cfg(test)]
mod tests;

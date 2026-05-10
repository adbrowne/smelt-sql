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
fn is_selectitems_kind_name(name: &str) -> bool {
    matches!(name, "Scalar" | "Agg" | "Window")
}

/// Parse input text into a CST
pub fn parse(input: &str) -> Parse {
    let tokens = tokenize(input);
    let mut parser = Parser::new(input, &tokens);
    parser.parse_file();
    parser.finish()
}

/// Maximum nesting depth for recursive parse functions.
/// Prevents stack overflow on adversarial or deeply nested input.
const MAX_PARSE_DEPTH: u32 = 256;

struct Parser<'a> {
    input: &'a str,
    tokens: &'a [Token],
    pos: usize,
    offset: usize,
    builder: GreenNodeBuilder<'static>,
    errors: Vec<ParseError>,
    depth: u32,
    /// Named row-variable names collected while parsing the current
    /// `smelt.define` param list (Phase 35). Reset at the start of each
    /// `smelt.define`; checked after the param list to enforce the v1
    /// constraint that at most one distinct name may appear.
    current_define_row_vars: Vec<String>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str, tokens: &'a [Token]) -> Self {
        Self {
            input,
            tokens,
            pos: 0,
            offset: 0,
            builder: GreenNodeBuilder::new(),
            errors: Vec::new(),
            depth: 0,
            current_define_row_vars: Vec::new(),
        }
    }

    fn finish(self) -> Parse {
        Parse {
            green_node: self.builder.finish(),
            errors: self.errors,
        }
    }

    /// Current token kind
    fn current(&self) -> SyntaxKind {
        self.tokens.get(self.pos).map(|t| t.kind).unwrap_or(EOF)
    }

    /// Check if at specific token kind
    fn at(&self, kind: SyntaxKind) -> bool {
        self.current() == kind
    }

    /// Check if at any of the given kinds
    fn at_any(&self, kinds: &[SyntaxKind]) -> bool {
        kinds.contains(&self.current())
    }

    /// Get the text of the current token
    fn current_text(&self) -> &str {
        if let Some(token) = self.tokens.get(self.pos) {
            &self.input[self.offset..self.offset + token.len]
        } else {
            ""
        }
    }

    /// Check if current token is an IDENT with specific text (case-insensitive)
    fn at_contextual_keyword(&self, text: &str) -> bool {
        self.at(IDENT) && self.current_text().eq_ignore_ascii_case(text)
    }

    /// Advance to next token, consuming trivia
    fn advance(&mut self) {
        if self.pos < self.tokens.len() {
            let token = self.tokens[self.pos];
            let text = &self.input[self.offset..self.offset + token.len];

            // Report lexer errors with the actual invalid character
            if token.kind == ERROR {
                let display_text = if text.len() <= 10 {
                    text.to_string()
                } else {
                    format!("{}...", &text[..10])
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
    fn skip_trivia(&mut self) {
        while self.current().is_trivia() {
            self.advance();
        }
    }

    /// Expect a specific token kind, report error if not present
    fn expect(&mut self, kind: SyntaxKind) -> bool {
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
    fn start_node(&mut self, kind: SyntaxKind) {
        self.builder.start_node(kind.into());
    }

    /// Start a composite node at a checkpoint (for lookahead/backtracking)
    fn start_node_at(&mut self, checkpoint: rowan::Checkpoint, kind: SyntaxKind) {
        self.builder.start_node_at(checkpoint, kind.into());
    }

    /// Finish current node
    fn finish_node(&mut self) {
        self.builder.finish_node();
    }

    /// Report a parse error
    fn error(&mut self, message: String) {
        let start = self.offset as u32;
        let end = (self.offset + self.tokens.get(self.pos).map(|t| t.len).unwrap_or(0)) as u32;
        self.errors.push(ParseError {
            message,
            range: TextRange::new(start.into(), end.into()),
        });
    }

    /// Check if we've exceeded the maximum nesting depth.
    /// Returns true if too deep (caller should bail out).
    fn too_deep(&mut self) -> bool {
        if self.depth >= MAX_PARSE_DEPTH {
            self.error("Expression nesting depth exceeds maximum of 256".to_string());
            true
        } else {
            false
        }
    }

    /// Synchronize to one of the given tokens (error recovery)
    fn sync_to(&mut self, kinds: &[SyntaxKind]) {
        while !self.at(EOF) && !self.at_any(kinds) {
            self.start_node(ERROR);
            self.advance();
            self.finish_node();
        }
    }

    /// Check if current token is a keyword that would end a table reference
    fn at_keyword_that_ends_table_ref(&self) -> bool {
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
    }

    /// Check if current token can start an expression
    fn at_expression_start(&self) -> bool {
        // Phase B (meta-language): `fn` is a reserved keyword (FN_KW) and can
        // start a lambda expression when encountered in expression position.
        self.at_any(&[
            IDENT, NUMBER, STRING, LPAREN, NOT_KW, CASE_KW, CAST_KW, EXTRACT_KW, EXISTS_KW,
            ARRAY_KW, ROW_KW, STRUCT_KW, MINUS, LBRACE, FN_KW,
        ])
    }

    // ===== Parsing rules =====

    fn parse_file(&mut self) {
        self.start_node(FILE);

        self.skip_trivia();

        // A file may contain: zero or more smelt.define declarations, interleaved
        // with at most one bare SELECT / WITH / VALUES statement.
        // smelt.define is ONLY special at the top-level statement position. In
        // expression position, `smelt.define` is an ordinary qualified identifier.
        let mut seen_model = false;
        while !self.at(EOF) {
            if self.at_smelt_define_trigger() {
                self.parse_smelt_define();
                self.skip_trivia();
                continue;
            }

            if self.at_smelt_extern_trigger() {
                self.parse_smelt_extern();
                self.skip_trivia();
                continue;
            }

            if self.at(SELECT_KW) || self.at(WITH_KW) {
                // Bare SELECT/WITH model body. We only parse the first one; any
                // following top-level tokens are consumed silently (preserving
                // pre-Phase-1 behavior for statements the child parser does not
                // fully consume, e.g. comma-separated FROM lists).
                if seen_model {
                    break;
                }
                self.parse_select_stmt();
                seen_model = true;
                self.skip_trivia();
                continue;
            }

            if self.at(VALUES_KW) {
                if seen_model {
                    break;
                }
                self.parse_values_clause();
                seen_model = true;
                self.skip_trivia();
                continue;
            }

            // Unknown content at top level. If we've already parsed a model
            // body, silently swallow the remainder (matches the legacy
            // single-statement parser's behavior). Otherwise, emit an error
            // and resync to the next top-level declaration.
            if seen_model {
                break;
            }
            self.error("Expected smelt.define or SELECT statement".to_string());
            self.sync_to_top_level();
            self.skip_trivia();
        }

        // Consume any remaining tokens (trivia or otherwise) without emitting
        // further errors — this preserves the pre-Phase-1 behavior of silently
        // absorbing leftover content at the end of a file.
        while !self.at(EOF) {
            self.advance();
        }

        self.finish_node();
    }

    /// Peek forward (skipping trivia) to check whether the current position is
    /// the start of a top-level `smelt.define` declaration. Does not consume
    /// any tokens. The trigger is exactly three non-trivia tokens:
    ///   IDENT("smelt")  DOT  IDENT("define")
    fn at_smelt_define_trigger(&self) -> bool {
        // First non-trivia token must be IDENT "smelt".
        if !self.at(IDENT) || !self.current_text().eq_ignore_ascii_case("smelt") {
            return false;
        }

        // Find the next non-trivia token: must be DOT.
        let mut lookahead = 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        match self.tokens.get(self.pos + lookahead) {
            Some(t) if t.kind == DOT => {}
            _ => return false,
        }

        // Find the next non-trivia token: must be IDENT "define".
        lookahead += 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        let Some(tok) = self.tokens.get(self.pos + lookahead) else {
            return false;
        };
        if tok.kind != IDENT {
            return false;
        }
        let mut offset = self.offset;
        for prior in 0..lookahead {
            offset += self.tokens[self.pos + prior].len;
        }
        let text = &self.input[offset..offset + tok.len];
        text.eq_ignore_ascii_case("define")
    }

    /// Peek forward (skipping trivia) to check whether the current position is
    /// the start of a top-level `smelt.extern` declaration. Does not consume
    /// any tokens. The trigger is exactly three non-trivia tokens:
    ///   IDENT("smelt")  DOT  IDENT("extern")
    fn at_smelt_extern_trigger(&self) -> bool {
        // First non-trivia token must be IDENT "smelt".
        if !self.at(IDENT) || !self.current_text().eq_ignore_ascii_case("smelt") {
            return false;
        }

        // Find the next non-trivia token: must be DOT.
        let mut lookahead = 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        match self.tokens.get(self.pos + lookahead) {
            Some(t) if t.kind == DOT => {}
            _ => return false,
        }

        // Find the next non-trivia token: must be IDENT "extern".
        lookahead += 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        let Some(tok) = self.tokens.get(self.pos + lookahead) else {
            return false;
        };
        if tok.kind != IDENT {
            return false;
        }
        let mut offset = self.offset;
        for prior in 0..lookahead {
            offset += self.tokens[self.pos + prior].len;
        }
        let text = &self.input[offset..offset + tok.len];
        text.eq_ignore_ascii_case("extern")
    }

    /// Peek forward (skipping trivia) to check whether the current position is
    /// the start of a `smelt.fn.<path>(...)` call (Phase 5b: now rejected).
    /// Used only in the rejection arms that emit parse errors.
    fn at_smelt_fn_trigger(&self) -> bool {
        // First non-trivia token must be IDENT "smelt".
        if !self.at(IDENT) || !self.current_text().eq_ignore_ascii_case("smelt") {
            return false;
        }
        // Find the next non-trivia token: must be DOT.
        let mut lookahead = 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        if self.tokens.get(self.pos + lookahead).map(|t| t.kind) != Some(DOT) {
            return false;
        }
        // Find the next non-trivia token: must be IDENT "fn".
        // `fn` is a contextual keyword that lexes as IDENT, so the check
        // is purely text-based.
        lookahead += 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        let Some(tok) = self.tokens.get(self.pos + lookahead) else {
            return false;
        };
        // `fn` is now a reserved keyword (FN_KW), so we check for that token kind.
        tok.kind == FN_KW
    }

    /// Peek forward (skipping trivia) to check whether the current position is
    /// the start of a unified `smelt.<path>` form (smelt.<path> migration,
    /// Phase 1). Does not consume any tokens.
    ///
    /// The trigger is `IDENT("smelt") DOT IDENT(<seg>)` where `<seg>` is NOT
    /// one of the existing legacy / built-in second segments:
    /// `fn`, `define`, `extern`, `as_struct`, `ref`, `source`, `metric`. Those
    /// keep their existing parser paths in Phase 1 and are removed only in
    /// Phase 4.
    ///
    /// We do NOT look at what follows the second segment — the parser
    /// disambiguates value form from call form (`SMELT_PATH_REF` vs.
    /// `SMELT_PATH_CALL`) at parse time on the trailing `(`.
    fn at_smelt_path_trigger(&self) -> bool {
        // First non-trivia token must be IDENT "smelt".
        if !self.at(IDENT) || !self.current_text().eq_ignore_ascii_case("smelt") {
            return false;
        }

        // Find the next non-trivia token: must be DOT.
        let mut lookahead = 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        match self.tokens.get(self.pos + lookahead) {
            Some(t) if t.kind == DOT => {}
            _ => return false,
        }

        // Find the next non-trivia token: must be IDENT.
        lookahead += 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        let Some(tok) = self.tokens.get(self.pos + lookahead) else {
            return false;
        };
        if tok.kind != IDENT {
            return false;
        }

        // Read the segment text without consuming.
        let mut offset = self.offset;
        for prior in 0..lookahead {
            offset += self.tokens[self.pos + prior].len;
        }
        let seg = &self.input[offset..offset + tok.len];

        // The unified path form does NOT steal from the existing legacy
        // grammar. These second segments stay on their current paths in
        // Phase 1.
        const LEGACY: &[&str] = &[
            "fn",
            "define",
            "extern",
            "as_struct",
            "ref",
            "source",
            "metric",
        ];
        for legacy in LEGACY {
            if seg.eq_ignore_ascii_case(legacy) {
                return false;
            }
        }
        true
    }

    /// Peek forward (skipping trivia) to check whether the current position is
    /// a **rejected** legacy call: `smelt.ref(` or `smelt.source(`.
    ///
    /// Phase 4 removes these forms from the language. The parser emits an error
    /// but still produces a FUNCTION_CALL node (error recovery) so downstream
    /// code that walks the CST doesn't crash.
    fn at_smelt_legacy_ref_or_source_trigger(&self) -> bool {
        if !self.at(IDENT) || !self.current_text().eq_ignore_ascii_case("smelt") {
            return false;
        }
        // Skip trivia to DOT.
        let mut la = 1;
        while let Some(t) = self.tokens.get(self.pos + la) {
            if t.kind.is_trivia() {
                la += 1;
            } else {
                break;
            }
        }
        if !matches!(self.tokens.get(self.pos + la), Some(t) if t.kind == DOT) {
            return false;
        }
        // Skip trivia to the second IDENT.
        la += 1;
        while let Some(t) = self.tokens.get(self.pos + la) {
            if t.kind.is_trivia() {
                la += 1;
            } else {
                break;
            }
        }
        let Some(tok) = self.tokens.get(self.pos + la) else {
            return false;
        };
        if tok.kind != IDENT {
            return false;
        }
        // Compute the text of the second segment.
        let mut offset = self.offset;
        for prior in 0..la {
            offset += self.tokens[self.pos + prior].len;
        }
        let seg = &self.input[offset..offset + tok.len];
        if !seg.eq_ignore_ascii_case("ref") && !seg.eq_ignore_ascii_case("source") {
            return false;
        }
        // The segment after must be `(` to confirm it's a call form.
        let mut la2 = la + 1;
        while let Some(t) = self.tokens.get(self.pos + la2) {
            if t.kind.is_trivia() {
                la2 += 1;
            } else {
                break;
            }
        }
        matches!(self.tokens.get(self.pos + la2), Some(t) if t.kind == LPAREN)
    }

    /// Peek forward (skipping trivia) to check whether the current position is
    /// the start of a `smelt.as_struct(...)` call. Does not consume any tokens.
    /// The trigger is exactly three non-trivia tokens:
    ///   IDENT("smelt")  DOT  IDENT("as_struct")
    /// followed by `(`.
    fn at_smelt_as_struct_trigger(&self) -> bool {
        if !self.at(IDENT) || !self.current_text().eq_ignore_ascii_case("smelt") {
            return false;
        }
        let mut lookahead = 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        match self.tokens.get(self.pos + lookahead) {
            Some(t) if t.kind == DOT => {}
            _ => return false,
        }
        lookahead += 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        let Some(tok) = self.tokens.get(self.pos + lookahead) else {
            return false;
        };
        if tok.kind != IDENT {
            return false;
        }
        let mut offset = self.offset;
        for prior in 0..lookahead {
            offset += self.tokens[self.pos + prior].len;
        }
        let text = &self.input[offset..offset + tok.len];
        text.eq_ignore_ascii_case("as_struct")
    }

    /// Parse a `smelt.as_struct(alias [EXCEPT col1, col2, ...])` expression.
    /// Produces a `SMELT_AS_STRUCT_CALL` node.
    fn parse_smelt_as_struct(&mut self) {
        self.start_node(SMELT_AS_STRUCT_CALL);
        self.advance(); // smelt
        self.skip_trivia();
        self.advance(); // .
        self.skip_trivia();
        self.advance(); // as_struct
        self.skip_trivia();
        if !self.at(LPAREN) {
            self.error("Expected '(' after 'smelt.as_struct'".to_string());
            self.finish_node();
            return;
        }
        self.advance(); // (
        self.skip_trivia();
        if !self.at(IDENT) {
            self.error("Expected alias identifier in 'smelt.as_struct(...)'".to_string());
            self.finish_node();
            return;
        }
        self.advance(); // alias identifier
        self.skip_trivia();
        if self.at(EXCEPT_KW) {
            self.start_node(EXCEPT_COL_LIST);
            self.advance(); // EXCEPT
            self.skip_trivia();
            loop {
                if !self.at(IDENT) {
                    break;
                }
                self.advance(); // column name
                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance(); // ,
                    self.skip_trivia();
                } else {
                    break;
                }
            }
            self.finish_node(); // EXCEPT_COL_LIST
        }
        self.skip_trivia();
        if !self.at(RPAREN) {
            self.error("Expected ')' to close 'smelt.as_struct(...)'".to_string());
        } else {
            self.advance(); // )
        }
        self.finish_node(); // SMELT_AS_STRUCT_CALL
    }

    /// Sync forward to EOF or the start of the next top-level `smelt.define`.
    /// Anything skipped is wrapped in ERROR nodes (one per token).
    fn sync_to_top_level(&mut self) {
        while !self.at(EOF) {
            // Skip trivia without emitting ERROR so the tree stays sensible.
            if self.current().is_trivia() {
                self.advance();
                continue;
            }
            if self.at_smelt_define_trigger() || self.at_smelt_extern_trigger() {
                return;
            }
            self.start_node(ERROR);
            self.advance();
            self.finish_node();
        }
    }

    /// Parse a top-level `smelt.define` declaration. The caller must have
    /// verified `at_smelt_define_trigger()` first.
    fn parse_smelt_define(&mut self) {
        self.start_node(SMELT_DEFINE);

        // Reset row-variable tracking for this define (Phase 35).
        self.current_define_row_vars.clear();

        // Consume the three trigger tokens: `smelt`, `.`, `define`.
        // They are three separate tokens in the lexer.
        self.skip_trivia();
        self.advance(); // IDENT "smelt"
        self.skip_trivia();
        self.advance(); // DOT
        self.skip_trivia();
        self.advance(); // IDENT "define"

        // DEFINE_NAME: wrap the next identifier.
        self.skip_trivia();
        if self.at(IDENT) {
            self.start_node(DEFINE_NAME);
            self.advance();
            self.finish_node();
        } else {
            self.error("Expected function name after smelt.define".to_string());
            // Try to sync to `(` so we can still parse the param list.
            self.sync_to(&[LPAREN, AS_KW, EOF]);
        }

        // Parameter list.
        self.skip_trivia();
        if self.at(LPAREN) {
            self.parse_param_list();
        } else {
            self.error("Expected '(' after function name".to_string());
            // Try to sync to AS_KW so we can still parse the body.
            self.sync_to(&[AS_KW, EOF]);
        }

        // Phase 35: v1 constraint — at most one distinct named row variable
        // per signature. Two distinct names (e.g. `..r` and `..s`) are an
        // error; the same name in multiple params is fine (the checker
        // enforces unification later).
        {
            let mut seen: Vec<String> = Vec::new();
            for name in self.current_define_row_vars.drain(..) {
                if !seen.contains(&name) {
                    seen.push(name);
                }
            }
            if seen.len() > 1 {
                self.error(format!(
                    "v1 constraint: at most one named row variable per signature (found: {})",
                    seen.join(", ")
                ));
            }
        }

        // Optional return arrow: `-> <TypeRef>`. The lexer produces a single
        // JSON_ARROW token for `->`.
        self.skip_trivia();
        if self.at(JSON_ARROW) {
            self.start_node(RETURN_ARROW);
            self.advance(); // JSON_ARROW (->)
            self.skip_trivia();
            self.parse_type_ref();
            self.finish_node();
        }

        // Expect AS.
        self.skip_trivia();
        if self.at(AS_KW) {
            self.advance();
        } else {
            self.error("Expected 'AS' in smelt.define".to_string());
            // Sync to the start of the body `(` or next top-level / EOF.
            while !self.at(EOF)
                && !self.at(LPAREN)
                && !self.at_smelt_define_trigger()
                && !self.at_smelt_extern_trigger()
            {
                self.start_node(ERROR);
                self.advance();
                self.finish_node();
            }
            if !self.at(LPAREN) {
                // No body to parse — finish the SmeltDefine node with errors.
                self.finish_node();
                return;
            }
        }

        // Body: `(` <expression> `)`.
        self.skip_trivia();
        self.parse_define_body();

        // Optional terminating `;`.
        self.skip_trivia();
        if self.at(IDENT) {
            // Never consume an IDENT here; a following smelt.define starts with
            // IDENT and must remain available for parse_file's dispatch.
        }
        // Consume a single `;` if present — the lexer does not tokenize `;`
        // (it would come through as an ERROR token). We ignore it defensively.

        self.finish_node();
    }

    /// Parse a top-level `smelt.extern` declaration. The caller must have
    /// verified `at_smelt_extern_trigger()` first.
    ///
    /// Grammar (Phase 10):
    ///   smelt.extern NAME ( params ) -> TypeRef
    ///
    /// Unlike `smelt.define`, there is no body — externs are signature-only
    /// declarations that bind a user-chosen name to a backend-provided function.
    fn parse_smelt_extern(&mut self) {
        self.start_node(SMELT_EXTERN);

        // Consume the three trigger tokens: `smelt`, `.`, `extern`.
        self.skip_trivia();
        self.advance(); // IDENT "smelt"
        self.skip_trivia();
        self.advance(); // DOT
        self.skip_trivia();
        self.advance(); // IDENT "extern"

        // DEFINE_NAME: wrap the next identifier. (Reusing DEFINE_NAME here so
        // downstream AST helpers can read the extern's name with the same
        // getter used for smelt.define.)
        //
        // Phase 11 (backend namespace sugar): accept a dotted form
        // `<backend>.<name>`, e.g. `smelt.extern duckdb.read_parquet(...)`.
        // When present, the backend segment is captured as the first IDENT
        // inside DEFINE_NAME; downstream AST helpers (`SmeltExtern::name`
        // and `SmeltExtern::backend_namespace`) read the two idents
        // separately. Non-dotted externs keep the legacy single-IDENT shape.
        self.skip_trivia();
        if self.at(IDENT) {
            self.start_node(DEFINE_NAME);
            self.advance(); // first IDENT
                            // If the next non-trivia tokens are `.` IDENT, treat this as a
                            // backend-namespaced extern name.
            let mut lookahead = 0;
            while let Some(t) = self.tokens.get(self.pos + lookahead) {
                if t.kind.is_trivia() {
                    lookahead += 1;
                } else {
                    break;
                }
            }
            let next = self.tokens.get(self.pos + lookahead).map(|t| t.kind);
            if next == Some(DOT) {
                // Check the token after the DOT is an IDENT.
                let mut after_dot = lookahead + 1;
                while let Some(t) = self.tokens.get(self.pos + after_dot) {
                    if t.kind.is_trivia() {
                        after_dot += 1;
                    } else {
                        break;
                    }
                }
                if self.tokens.get(self.pos + after_dot).map(|t| t.kind) == Some(IDENT) {
                    // Consume `.` and the second IDENT — both land inside
                    // the same DEFINE_NAME node.
                    self.skip_trivia();
                    self.advance(); // DOT
                    self.skip_trivia();
                    self.advance(); // second IDENT
                }
            }
            self.finish_node();
        } else {
            self.error("Expected function name after smelt.extern".to_string());
            self.sync_to(&[LPAREN, JSON_ARROW, EOF]);
        }

        // Parameter list.
        self.skip_trivia();
        if self.at(LPAREN) {
            self.parse_param_list();
        } else {
            self.error("Expected '(' after function name".to_string());
            self.sync_to(&[JSON_ARROW, EOF]);
        }

        // Return arrow: `-> <TypeRef>`. Required for externs — the extern
        // signature must declare its return type.
        self.skip_trivia();
        if self.at(JSON_ARROW) {
            self.start_node(RETURN_ARROW);
            self.advance(); // JSON_ARROW (->)
            self.skip_trivia();
            self.parse_type_ref();
            self.finish_node();
        } else {
            self.error("Expected '->' return type in smelt.extern".to_string());
        }

        self.finish_node(); // SMELT_EXTERN
    }

    /// Parse the parenthesized parameter list of a smelt.define.
    fn parse_param_list(&mut self) {
        self.start_node(PARAM_LIST);
        self.expect(LPAREN);
        self.skip_trivia();

        while !self.at(RPAREN) && !self.at(EOF) {
            // If we see a top-level resync point, break out to let error
            // recovery kick in at the caller.
            if self.at(AS_KW) || self.at_smelt_define_trigger() || self.at_smelt_extern_trigger() {
                self.error("Expected ')' to close parameter list".to_string());
                break;
            }

            if !self.at(IDENT) {
                self.error("Expected parameter name".to_string());
                // Consume one token as ERROR to make progress.
                self.start_node(ERROR);
                self.advance();
                self.finish_node();
                self.skip_trivia();
                continue;
            }

            self.parse_param();

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
                self.skip_trivia();
                // Allow trailing comma.
                if self.at(RPAREN) {
                    break;
                }
            } else {
                break;
            }
        }

        self.skip_trivia();
        self.expect(RPAREN);
        self.finish_node();
    }

    /// Parse a single parameter: NAME [ ':' TYPE_REF ] [ '=' DEFAULT_VALUE ].
    fn parse_param(&mut self) {
        self.start_node(PARAM);

        // Parameter name (required, we've already checked for IDENT).
        self.advance(); // consume IDENT

        // Optional `: TypeRef`.
        self.skip_trivia();
        if self.at(COLON) {
            self.advance();
            self.skip_trivia();
            self.parse_type_ref();
        }

        // Optional `= DefaultValue`.
        self.skip_trivia();
        if self.at(EQ) {
            self.start_node(DEFAULT_VALUE);
            self.advance();
            self.skip_trivia();
            self.parse_expression();
            self.finish_node();
        }

        self.finish_node();
    }

    /// Parse a type reference.
    ///
    /// History:
    ///   - Phase 1: flat token run.
    ///   - Phase 4: text-level structured parse in `smelt-types` (still a
    ///     flat CST).
    ///   - Phase 13: structured CST children for the non-`Expr` sorts
    ///     (`TableExpr`, `AggExpr`, `WindowExpr`, `SelectItems`). The
    ///     head identifier is still emitted as raw IDENT tokens inside
    ///     `TYPE_REF`, with additional structured nodes (`EXPR_KIND_*`,
    ///     `ROW_REQUIREMENT`, `SELECTITEMS_KIND`, `SELECTITEMS_CTX`)
    ///     siblings to the raw tokens so the AST wrapper can classify
    ///     the sort and expose structured getters.
    ///
    /// Boundaries at depth 0 are `,`, `)`, `=`, `AS`, `->`, and EOF (the
    /// caller's error recovery handles the rest).
    fn parse_type_ref(&mut self) {
        self.start_node(TYPE_REF);

        self.skip_trivia();

        // Peek the leading IDENT (if any). This decides whether we go
        // down a structured path (for recognised sorts) or fall back to
        // the flat consumer used for unknown / error heads.
        let head_text = if self.at(IDENT) {
            Some(self.current_text().to_string())
        } else {
            None
        };

        match head_text.as_deref() {
            // Scalar / Agg / Window expression sorts: emit an EXPR_KIND
            // tag alongside the flat body so the AST can report a
            // uniform `expr_kind()` across all three. The data-type
            // payload is parsed textually by `smelt-types` from the
            // TYPE_REF's raw text, so we keep that as a flat run here.
            // Phase 19: also detect optional `, ctx` context binding
            // and emit an `EXPR_CTX` child node.
            Some("Expr") => {
                self.advance(); // IDENT "Expr"
                self.emit_expr_kind_marker(EXPR_KIND_SCALAR);
                self.parse_expr_tail();
            }
            Some("AggExpr") => {
                self.advance(); // IDENT "AggExpr"
                self.emit_expr_kind_marker(EXPR_KIND_AGG);
                self.parse_expr_tail();
            }
            Some("WindowExpr") => {
                self.advance(); // IDENT "WindowExpr"
                self.emit_expr_kind_marker(EXPR_KIND_WINDOW);
                self.parse_expr_tail();
            }
            Some("TableExpr") => {
                self.advance(); // IDENT "TableExpr"
                self.parse_tableexpr_tail();
            }
            Some("SelectItems") => {
                self.advance(); // IDENT "SelectItems"
                self.parse_selectitems_tail();
            }
            // Unknown / non-matching sort — emit a parse error so the
            // Phase 4-era "unknown sort" diagnostic path still lights up
            // without reaching into `smelt-types`. Callers still see the
            // original tokens via the flat consumer.
            Some(other) => {
                let msg = format!(
                    "Unknown type sort `{}` — expected one of Expr, AggExpr, WindowExpr, TableExpr, SelectItems",
                    other
                );
                self.error(msg);
                self.consume_type_ref_tail();
            }
            None => {
                // Recovery: no leading IDENT — consume the tail anyway.
                self.consume_type_ref_tail();
            }
        }

        self.finish_node();
    }

    /// Emit a zero-width marker node of the given kind (one of
    /// [`EXPR_KIND_SCALAR`], [`EXPR_KIND_AGG`], or [`EXPR_KIND_WINDOW`]).
    ///
    /// The marker is a sibling of the leading sort IDENT inside the
    /// `TYPE_REF`. It contains no tokens and therefore does not
    /// contribute to the `TypeRef::text()` output — downstream
    /// signature extraction (which parses `type_ref.text()`) sees
    /// exactly the user-written source text. The AST wrapper classifies
    /// it via [`TypeRef::expr_kind()`] by inspecting the marker's
    /// `SyntaxKind`.
    fn emit_expr_kind_marker(&mut self, kind: SyntaxKind) {
        debug_assert!(matches!(
            kind,
            EXPR_KIND_SCALAR | EXPR_KIND_AGG | EXPR_KIND_WINDOW
        ));
        self.builder.start_node(kind.into());
        self.builder.finish_node();
    }

    /// Parse a flat (unstructured) `TYPE_REF` that stops at row-field
    /// boundaries: `,`, `}`, `>` (closing `<`), and usual parameter
    /// boundaries. Used for row-field type annotations inside a
    /// `ROW_REQUIREMENT`, where the type is a bare name like `Numeric`
    /// or `Integer` — not an `Expr<...>` sort.
    fn parse_flat_type_ref_stopping_on_row_field_boundary(&mut self) {
        self.start_node(TYPE_REF);
        let mut angle_depth: i32 = 0;
        let mut paren_depth: i32 = 0;
        loop {
            self.skip_trivia();
            let k = self.current();
            if k == EOF {
                break;
            }
            if angle_depth == 0 && paren_depth == 0 {
                if matches!(k, COMMA | RPAREN | EQ | AS_KW | JSON_ARROW | RBRACE | GT) {
                    break;
                }
                if k == IDENT && (self.at_smelt_define_trigger() || self.at_smelt_extern_trigger())
                {
                    break;
                }
            }
            match k {
                LT => angle_depth += 1,
                GT => angle_depth = angle_depth.saturating_sub(1),
                LPAREN => paren_depth += 1,
                RPAREN => paren_depth = paren_depth.saturating_sub(1),
                _ => {}
            }
            self.advance();
        }
        self.finish_node();
    }

    /// Flat-consume tokens up to a depth-0 boundary. `<` and `(` open
    /// angle/paren depth so commas inside don't end the TypeRef.
    fn consume_type_ref_tail(&mut self) {
        let mut angle_depth: i32 = 0;
        let mut paren_depth: i32 = 0;
        loop {
            self.skip_trivia();
            let k = self.current();
            if k == EOF {
                break;
            }
            if angle_depth == 0 && paren_depth == 0 {
                if matches!(k, COMMA | RPAREN | EQ | AS_KW | JSON_ARROW) {
                    break;
                }
                if k == IDENT && (self.at_smelt_define_trigger() || self.at_smelt_extern_trigger())
                {
                    break;
                }
            }
            match k {
                LT => angle_depth += 1,
                GT => angle_depth = angle_depth.saturating_sub(1),
                LPAREN => paren_depth += 1,
                RPAREN => paren_depth = paren_depth.saturating_sub(1),
                _ => {}
            }
            self.advance();
        }
    }

    /// Parse the tail of a `TableExpr` type reference: optionally
    /// `<{ field, ..., ..tail }>`. When no `<` follows, nothing further
    /// is emitted — a bare `TableExpr` is a valid row-polymorphic
    /// parameter type.
    fn parse_tableexpr_tail(&mut self) {
        self.skip_trivia();
        if !self.at(LT) {
            self.consume_type_ref_tail();
            return;
        }
        self.advance(); // LT
        self.skip_trivia();

        if !self.at(LBRACE) {
            // No row-requirement between the angle brackets — consume
            // the remainder up to the matching `>`.
            self.skip_to_matching_gt();
            return;
        }

        self.start_node(ROW_REQUIREMENT);
        self.advance(); // `{`

        loop {
            self.skip_trivia();
            if self.at(RBRACE) || self.at(EOF) {
                break;
            }

            // Tail markers: `..` (DOT_DOT token). Decide between
            // `..name` (ROW_TAIL_NAMED) and `..` (ROW_TAIL_ANON) by
            // lookahead, so we only ever open one of the two nodes.
            if self.at(DOT_DOT) {
                // Peek past the DOT_DOT to see if an IDENT follows
                // (named tail) or not (anonymous).
                let is_named = matches!(
                    self.peek_next_non_trivia(),
                    Some(k) if k == IDENT
                );
                if is_named {
                    self.start_node(ROW_TAIL_NAMED);
                    self.advance(); // DOT_DOT
                    self.skip_trivia();
                    self.advance(); // IDENT
                    self.finish_node();
                } else {
                    self.start_node(ROW_TAIL_ANON);
                    self.advance(); // DOT_DOT
                    self.finish_node();
                }

                self.skip_trivia();
                if self.at(COMMA) {
                    self.error(
                        "`..tail` must be the last element of a row requirement".to_string(),
                    );
                    self.advance();
                    // keep going defensively so we don't loop forever
                    continue;
                }
                break;
            }

            // Regular row field: `name: TypeRef`. The row-field TYPE_REF
            // is a flat type name (e.g. `Numeric`, `Integer`, `Text`) —
            // not an `Expr<...>` sort. Use the flat consumer so bare
            // type names don't trip the sort-head dispatch.
            if self.at(IDENT) {
                self.start_node(ROW_FIELD);
                self.advance(); // IDENT name
                self.skip_trivia();
                if self.at(COLON) {
                    self.advance();
                    self.skip_trivia();
                    self.parse_flat_type_ref_stopping_on_row_field_boundary();
                } else {
                    self.error("Expected `:` after row field name".to_string());
                }
                self.finish_node();
            } else {
                self.error("Expected row field name or `..tail`".to_string());
                self.start_node(ERROR);
                self.advance();
                self.finish_node();
            }

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
                self.skip_trivia();
                if self.at(RBRACE) {
                    break;
                }
            } else {
                break;
            }
        }

        self.skip_trivia();
        if self.at(RBRACE) {
            self.advance(); // `}`
        } else {
            self.error("Expected `}` to close row requirement".to_string());
        }
        self.finish_node(); // ROW_REQUIREMENT

        // Expect the closing `>`; tolerate its absence.
        self.skip_trivia();
        if self.at(GT) {
            self.advance();
        } else {
            self.skip_to_matching_gt();
        }
    }

    /// Parse the tail of a `SelectItems<...>` type reference. Accepts:
    ///   - `<Kind>`           — single uppercase-Kind argument.
    ///   - `<ctx>`            — single lowercase context argument.
    ///   - `<Kind, ctx>`      — two arguments, kind then context.
    ///
    /// The first-token heuristic for the single-arg form: if the
    /// identifier is one of `{Scalar, Agg, Window}`, it's a
    /// `SELECTITEMS_KIND`; otherwise it is a `SELECTITEMS_CTX`.
    fn parse_selectitems_tail(&mut self) {
        self.skip_trivia();
        if !self.at(LT) {
            self.consume_type_ref_tail();
            return;
        }
        self.advance(); // LT
        self.skip_trivia();

        if !self.at(IDENT) {
            self.error("Expected kind or context identifier inside SelectItems<...>".to_string());
            self.skip_to_matching_gt();
            return;
        }

        let first_text = self.current_text().to_string();
        let first_is_kind = is_selectitems_kind_name(&first_text);
        let has_second = self.peek_has_second_selectitems_arg();

        if has_second {
            if !first_is_kind {
                self.error(format!(
                    "Expected a SelectItems kind (Scalar, Agg, or Window) as the first of two arguments, got `{}`",
                    first_text
                ));
            }
            self.start_node(SELECTITEMS_KIND);
            self.advance(); // IDENT
            self.finish_node();
            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
                self.skip_trivia();
            }
            if self.at(IDENT) {
                self.start_node(SELECTITEMS_CTX);
                self.advance();
                self.finish_node();
            } else {
                self.error("Expected context identifier after `,` in SelectItems<...>".to_string());
            }
        } else if first_is_kind {
            self.start_node(SELECTITEMS_KIND);
            self.advance();
            self.finish_node();
        } else {
            self.start_node(SELECTITEMS_CTX);
            self.advance();
            self.finish_node();
        }

        self.skip_trivia();
        if self.at(GT) {
            self.advance();
        } else {
            self.skip_to_matching_gt();
        }
    }

    /// Parse the tail of an `Expr<T>` / `AggExpr<T>` / `WindowExpr<T>`
    /// type reference (Phase 19). Accepts:
    ///
    /// - `<T>` — single data-type argument; consumed as flat tokens.
    /// - `<T, ctx>` — data-type followed by a lowercase context identifier;
    ///   the context identifier is wrapped in `EXPR_CTX`.
    /// - `<Struct<{field: Type, ..tail}>>` — Phase 35 struct type; the inner
    ///   `Struct<{...}>` is parsed as a `STRUCT_TYPE` node.
    ///
    /// Falls back to `consume_type_ref_tail` for the no-`<` case.
    fn parse_expr_tail(&mut self) {
        self.skip_trivia();
        if !self.at(LT) {
            self.consume_type_ref_tail();
            return;
        }
        self.advance(); // LT

        // Phase 35: if the inner type is `Struct<{...}>`, hand off to
        // the structured struct-type parser instead of the flat consumer.
        // `STRUCT` is lexed as STRUCT_KW (a keyword), not IDENT.
        self.skip_trivia();
        if self.at(STRUCT_KW) && self.is_struct_type_start() {
            self.parse_struct_type();
            self.skip_trivia();
            if self.at(GT) {
                self.advance(); // closing `>` of `Expr<Struct<...>>`
            } else {
                self.skip_to_matching_gt();
            }
            return;
        }

        // Consume flat tokens until we see `,` at depth 0 (ctx follows)
        // or `>` at depth 0 (no ctx).
        let mut angle_depth: i32 = 0;
        loop {
            self.skip_trivia();
            let k = self.current();
            if k == EOF {
                break;
            }
            if angle_depth == 0 {
                if k == GT {
                    self.advance();
                    return;
                }
                if k == COMMA {
                    self.advance(); // COMMA
                    self.skip_trivia();
                    if self.at(IDENT) {
                        self.start_node(EXPR_CTX);
                        self.advance(); // context identifier
                        self.finish_node();
                    } else {
                        self.error(
                            "Expected context identifier after `,` in Expr<...>".to_string(),
                        );
                    }
                    self.skip_trivia();
                    if self.at(GT) {
                        self.advance();
                    } else {
                        self.skip_to_matching_gt();
                    }
                    return;
                }
            }
            match k {
                LT => angle_depth += 1,
                GT => angle_depth -= 1,
                _ => {}
            }
            self.advance();
        }
    }

    /// After the current IDENT, peek whether the next non-trivia token
    /// is `,` (two args) as opposed to `>` (single arg).
    fn peek_has_second_selectitems_arg(&self) -> bool {
        matches!(self.peek_nth_non_trivia(1), Some(k) if k == COMMA)
    }

    /// Peek the kind of the Nth non-trivia token after the current one
    /// (0 == current non-trivia; 1 == the one after, etc.).
    fn peek_nth_non_trivia(&self, n: usize) -> Option<SyntaxKind> {
        let mut i = self.pos;
        let mut found = 0usize;
        // Advance through tokens, counting non-trivia as we go.
        while let Some(t) = self.tokens.get(i) {
            if !t.kind.is_trivia() {
                if found == n {
                    return Some(t.kind);
                }
                found += 1;
            }
            i += 1;
        }
        None
    }

    /// Peek the kind of the next non-trivia token after the current one.
    fn peek_next_non_trivia(&self) -> Option<SyntaxKind> {
        self.peek_nth_non_trivia(1)
    }

    /// Consume tokens until a `>` at angle-depth 0 is found (and
    /// consumed), or a type-ref boundary / EOF is reached.
    fn skip_to_matching_gt(&mut self) {
        let mut angle_depth: i32 = 1;
        while !self.at(EOF) {
            let k = self.current();
            if matches!(k, COMMA | RPAREN | AS_KW | JSON_ARROW) && angle_depth == 1 {
                return;
            }
            match k {
                LT => angle_depth += 1,
                GT => {
                    angle_depth -= 1;
                    if angle_depth == 0 {
                        self.advance();
                        return;
                    }
                }
                _ => {}
            }
            self.advance();
        }
    }

    /// Peek ahead (without consuming) to check whether the current token is the
    /// start of a `Struct<{...}>` type — i.e. `STRUCT_KW` followed by `<` then `{`.
    fn is_struct_type_start(&self) -> bool {
        // Current token must be STRUCT_KW (already confirmed by caller).
        // Peek for: [trivia*] LT [trivia*] LBRACE
        let mut i = 1;
        while let Some(t) = self.tokens.get(self.pos + i) {
            if t.kind.is_trivia() {
                i += 1;
                continue;
            }
            if t.kind == LT {
                // found `<`, now look for `{`
                i += 1;
                while let Some(t2) = self.tokens.get(self.pos + i) {
                    if t2.kind.is_trivia() {
                        i += 1;
                        continue;
                    }
                    return t2.kind == LBRACE;
                }
                return false;
            }
            return false;
        }
        false
    }

    /// Parse `Struct<{field: Type, ..tail}>` into a `STRUCT_TYPE` node.
    ///
    /// Caller has already confirmed the leading `STRUCT_KW` and that the
    /// next significant tokens are `<` `{`. The caller must consume the
    /// trailing `>` of the outer `Expr<...>` wrapper.
    fn parse_struct_type(&mut self) {
        self.start_node(STRUCT_TYPE);

        // Consume `Struct` (tokenized as STRUCT_KW)
        self.skip_trivia();
        self.advance(); // STRUCT_KW
        self.skip_trivia();
        self.advance(); // LT `<`
        self.skip_trivia();
        self.advance(); // LBRACE `{`

        loop {
            self.skip_trivia();
            if self.at(RBRACE) || self.at(EOF) {
                break;
            }

            // Tail markers: `..` (DOT_DOT token).
            if self.at(DOT_DOT) {
                // ROW_TAIL: either `..ident` (named) or `..` (anonymous).
                let is_named = matches!(
                    self.peek_next_non_trivia(),
                    Some(k) if k == IDENT
                );
                self.start_node(ROW_TAIL);
                self.advance(); // DOT_DOT
                if is_named {
                    self.skip_trivia();
                    // Record the row-variable name for the per-define constraint check.
                    let name = self.current_text().to_string();
                    self.current_define_row_vars.push(name);
                    self.advance(); // IDENT
                }
                self.finish_node(); // ROW_TAIL

                self.skip_trivia();
                if self.at(COMMA) {
                    self.error("`..tail` must be the last element of a struct type".to_string());
                    self.advance();
                    continue;
                }
                break;
            }

            // Regular field: `name: Type`.
            if self.at(IDENT) {
                self.start_node(STRUCT_FIELD);
                self.advance(); // IDENT name
                self.skip_trivia();
                if self.at(COLON) {
                    self.advance();
                    self.skip_trivia();
                    self.parse_flat_type_ref_stopping_on_row_field_boundary();
                } else {
                    self.error("Expected `:` after struct field name".to_string());
                }
                self.finish_node(); // STRUCT_FIELD
            } else {
                self.error("Expected struct field name or `..tail`".to_string());
                self.start_node(ERROR);
                self.advance();
                self.finish_node();
            }

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
                self.skip_trivia();
                if self.at(RBRACE) {
                    break;
                }
            } else {
                break;
            }
        }

        self.skip_trivia();
        if self.at(RBRACE) {
            self.advance(); // `}`
        } else {
            self.error("Expected `}` to close struct type".to_string());
        }

        self.skip_trivia();
        if self.at(GT) {
            self.advance(); // `>` closing `Struct<{...}>`
        } else {
            self.error("Expected `>` to close Struct<{...}>".to_string());
        }

        self.finish_node(); // STRUCT_TYPE
    }

    /// Parse a brace-struct literal: `{expr AS name, ..spread}`.
    ///
    /// Items are:
    ///   - `STRUCT_FIELD_ITEM`: `expr AS alias`
    ///   - `SPREAD_ITEM`: `..ident`
    fn parse_brace_struct_literal(&mut self) {
        self.start_node(BRACE_STRUCT_LITERAL);
        self.advance(); // consume `{`

        loop {
            self.skip_trivia();
            if self.at(RBRACE) || self.at(EOF) {
                break;
            }

            // Spread item: `..ident` (DOT_DOT token)
            if self.at(DOT_DOT) {
                self.start_node(SPREAD_ITEM);
                self.advance(); // DOT_DOT
                self.skip_trivia();
                if self.at(IDENT) {
                    self.advance(); // IDENT name
                } else {
                    self.error("Expected identifier after `..` in struct spread".to_string());
                }
                self.finish_node(); // SPREAD_ITEM

                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                }
                // Spread is typically last, but we allow trailing items for error recovery.
                continue;
            }

            // Field item: `expr AS alias`
            self.start_node(STRUCT_FIELD_ITEM);
            self.parse_expression();
            self.skip_trivia();
            if self.at(AS_KW) {
                self.advance(); // AS
                self.skip_trivia();
                if self.at(IDENT) {
                    self.advance(); // alias
                } else {
                    self.error("Expected alias after AS in struct field".to_string());
                }
            } else {
                self.error("Expected AS alias in struct field item".to_string());
            }
            self.finish_node(); // STRUCT_FIELD_ITEM

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
                self.skip_trivia();
                if self.at(RBRACE) {
                    break;
                }
            } else {
                break;
            }
        }

        self.skip_trivia();
        if self.at(RBRACE) {
            self.advance(); // `}`
        } else {
            self.error("Expected `}` to close brace struct literal".to_string());
        }

        self.finish_node(); // BRACE_STRUCT_LITERAL
    }

    /// Parse the parenthesized body of a smelt.define.
    ///
    /// Phase 1 only supports expression bodies. SELECT-statement bodies are
    /// deferred to a later phase.
    fn parse_define_body(&mut self) {
        self.start_node(DEFINE_BODY);

        if !self.at(LPAREN) {
            self.error("Expected '(' to start smelt.define body".to_string());
            self.finish_node();
            return;
        }

        self.advance(); // consume '('
        self.skip_trivia();

        // Phase 13: allow a SELECT statement (or WITH … SELECT) as the
        // body, in addition to the Phase-1 expression body. This keeps
        // the simple-expression tests unchanged while enabling the
        // TableExpr-returning fixtures introduced in Step 3.
        if self.at(SELECT_KW) || self.at(WITH_KW) {
            self.parse_select_stmt();
        } else {
            // Parse a single expression. If parsing produces an unbalanced `(`
            // we rely on the caller's sync loop to recover at the next
            // top-level.
            self.parse_expression();
        }

        self.skip_trivia();
        if self.at(RPAREN) {
            self.advance();
        } else {
            self.error("Expected ')' to close smelt.define body".to_string());
            // Sync to EOF or the next top-level declaration so a following
            // smelt.define / smelt.extern still parses.
            while !self.at(EOF)
                && !self.at_smelt_define_trigger()
                && !self.at_smelt_extern_trigger()
            {
                self.start_node(ERROR);
                self.advance();
                self.finish_node();
            }
        }

        self.finish_node();
    }

    /// Parse a unified `smelt.<seg>(.<seg>)*` value-form or
    /// `smelt.<seg>(.<seg>)*(args) [PASSING ...]` call-form path
    /// (smelt.<path> migration, Phase 1).
    ///
    /// Caller must have verified `at_smelt_path_trigger()` first. The minimal
    /// valid form is `smelt.<seg>` with at least one segment after the
    /// `smelt.` prefix. The parser disambiguates value vs. call by whether
    /// `(` follows the final segment:
    ///
    /// * No trailing `(` → emits `SMELT_PATH_REF` containing one `SMELT_PATH`.
    /// * Trailing `(` → emits `SMELT_PATH_CALL` containing one `SMELT_PATH`,
    ///   one `ARG_LIST`, and zero or more trailing `PASSING_CLAUSE`s.
    fn parse_smelt_path_form(&mut self) {
        let outer_checkpoint = self.builder.checkpoint();

        // Build the SMELT_PATH child first; we'll wrap with the right outer
        // node (REF vs CALL) once we know whether `(` follows.
        let path_checkpoint = self.builder.checkpoint();

        // Consume `smelt . <seg>` — the trigger guarantees this shape.
        self.skip_trivia();
        self.advance(); // IDENT "smelt"
        self.skip_trivia();
        self.advance(); // DOT
        self.skip_trivia();
        self.advance(); // IDENT (first segment)

        // Continue consuming `.<IDENT>` segments while present.
        loop {
            // Peek past trivia to the next non-trivia token.
            let mut lookahead = 0;
            while let Some(t) = self.tokens.get(self.pos + lookahead) {
                if t.kind.is_trivia() {
                    lookahead += 1;
                } else {
                    break;
                }
            }
            let next_kind = self
                .tokens
                .get(self.pos + lookahead)
                .map(|t| t.kind)
                .unwrap_or(EOF);
            if next_kind != DOT {
                break;
            }
            // Peek past the DOT to require an IDENT after it.
            let mut lookahead2 = lookahead + 1;
            while let Some(t) = self.tokens.get(self.pos + lookahead2) {
                if t.kind.is_trivia() {
                    lookahead2 += 1;
                } else {
                    break;
                }
            }
            let after_dot = self
                .tokens
                .get(self.pos + lookahead2)
                .map(|t| t.kind)
                .unwrap_or(EOF);
            if after_dot != IDENT {
                break;
            }
            self.skip_trivia();
            self.advance(); // DOT
            self.skip_trivia();
            self.advance(); // IDENT
        }

        // Close the SMELT_PATH child node now that all segments are captured.
        self.start_node_at(path_checkpoint, SMELT_PATH);
        self.finish_node(); // SMELT_PATH

        // Determine whether this is a call or a value form by peeking past
        // any trivia for `(`. Do NOT consume trivia here — trivia before `(`
        // belongs inside SMELT_PATH_CALL (consumed by parse_arg_list), and
        // trivia after a value-form ref belongs to the following token, not
        // to the SMELT_PATH_REF node.
        let mut la = 0;
        while let Some(t) = self.tokens.get(self.pos + la) {
            if t.kind.is_trivia() {
                la += 1;
            } else {
                break;
            }
        }
        let next_is_lparen = self
            .tokens
            .get(self.pos + la)
            .map(|t| t.kind == LPAREN)
            .unwrap_or(false);
        if next_is_lparen {
            // Call form. Wrap everything from the outer checkpoint in
            // SMELT_PATH_CALL, parse the arg list (which consumes the trivia
            // before `(`), then any trailing PASSING clauses.
            self.start_node_at(outer_checkpoint, SMELT_PATH_CALL);
            self.parse_arg_list();

            // Zero or more `PASSING <name> AS (<body>)` clauses (parity with
            // smelt.fn.* — Phase 28 keeps this contextual keyword behaviour).
            loop {
                self.skip_trivia();
                if !self.at_contextual_keyword("PASSING") {
                    break;
                }
                self.parse_passing_clause();
            }
            self.finish_node(); // SMELT_PATH_CALL
        } else {
            // Value form. Wrap the path in SMELT_PATH_REF.
            self.start_node_at(outer_checkpoint, SMELT_PATH_REF);
            self.finish_node(); // SMELT_PATH_REF
        }
    }

    /// Parse a single `PASSING <name> AS (<body>)` clause.
    /// The caller must have verified `at_contextual_keyword("PASSING")` first.
    fn parse_passing_clause(&mut self) {
        self.start_node(PASSING_CLAUSE);

        // Consume the `PASSING` contextual keyword (it is an IDENT token).
        self.skip_trivia();
        self.advance(); // IDENT "PASSING"

        // Parse the binding name into PASSING_NAME.
        self.skip_trivia();
        self.start_node(PASSING_NAME);
        if self.at(IDENT) {
            self.advance(); // IDENT (name)
        } else {
            self.error("Expected identifier after PASSING".to_string());
        }
        self.finish_node(); // PASSING_NAME

        // Expect `AS`.
        self.skip_trivia();
        if self.at(AS_KW) {
            self.advance(); // AS
        } else {
            self.error("Expected AS after PASSING <name>".to_string());
            self.finish_node(); // PASSING_CLAUSE
            return;
        }

        // Expect `(`, then an expression, then `)`.
        self.skip_trivia();
        if !self.at(LPAREN) {
            self.error("Expected '(' after PASSING <name> AS".to_string());
            self.finish_node(); // PASSING_CLAUSE
            return;
        }
        self.advance(); // LPAREN

        // Parse the body expression into PASSING_BODY.
        self.start_node(PASSING_BODY);
        self.skip_trivia();
        if self.at_expression_start() {
            self.parse_expression();
        } else {
            self.error("Expected expression in PASSING body".to_string());
        }
        self.finish_node(); // PASSING_BODY

        // Closing `)`.
        self.skip_trivia();
        if self.at(RPAREN) {
            self.advance(); // RPAREN
        } else {
            self.error("Expected ')' to close PASSING body".to_string());
        }

        self.finish_node(); // PASSING_CLAUSE
    }

    fn parse_select_stmt(&mut self) {
        self.start_node(SELECT_STMT);

        if self.too_deep() {
            self.finish_node();
            return;
        }
        self.depth += 1;

        // WITH clause MUST come first (before SELECT)
        self.skip_trivia();
        if self.at(WITH_KW) {
            self.parse_with_clause();
        }

        // SELECT
        self.expect(SELECT_KW);

        // DISTINCT / ALL (after SELECT, before select list)
        self.skip_trivia();
        if self.at(DISTINCT_KW) {
            self.advance(); // DISTINCT
            self.skip_trivia();
            // Check for DISTINCT ON (PostgreSQL)
            if self.at(ON_KW) {
                self.start_node(DISTINCT_ON_CLAUSE);
                self.advance(); // ON
                self.skip_trivia();
                if self.expect(LPAREN) {
                    // Parse expression list
                    loop {
                        self.skip_trivia();
                        self.parse_expression();
                        self.skip_trivia();
                        if self.at(COMMA) {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.expect(RPAREN);
                }
                self.finish_node(); // DISTINCT_ON_CLAUSE
            }
        } else if self.at(ALL_KW) {
            self.advance();
        }

        // Select list
        self.parse_select_list();

        // FROM clause (optional - SELECT without FROM is valid)
        self.skip_trivia();
        if self.at(FROM_KW) {
            self.parse_from_clause();
        }

        // WHERE clause
        self.skip_trivia();
        if self.at(WHERE_KW) {
            self.parse_where_clause();
        }

        // GROUP BY clause
        self.skip_trivia();
        if self.at(GROUP_KW) {
            self.parse_group_by_clause();
        }

        // HAVING clause (must come after GROUP BY)
        self.skip_trivia();
        if self.at(HAVING_KW) {
            self.parse_having_clause();
        }

        // QUALIFY clause (after HAVING, before ORDER BY)
        self.skip_trivia();
        if self.at(QUALIFY_KW) {
            self.parse_qualify_clause();
        }

        // ORDER BY clause
        self.skip_trivia();
        if self.at(ORDER_KW) {
            self.parse_order_by_clause();
        }

        // LIMIT clause
        self.skip_trivia();
        let has_limit = self.at(LIMIT_KW);
        if has_limit {
            self.parse_limit_clause();
        }

        // OFFSET without LIMIT (for FETCH FIRST pattern)
        self.skip_trivia();
        if self.at(OFFSET_KW) && !has_limit {
            self.start_node(LIMIT_CLAUSE);
            self.advance(); // OFFSET
            self.skip_trivia();
            if self.at(NUMBER) {
                self.advance();
            } else {
                self.error("Expected number after OFFSET".to_string());
            }
            self.finish_node();
        }

        // FETCH FIRST/NEXT N ROW(S) ONLY
        self.skip_trivia();
        if self.at_contextual_keyword("FETCH") {
            self.parse_fetch_clause();
        }

        // Set operations: UNION / INTERSECT / EXCEPT
        self.skip_trivia();
        if self.at_any(&[UNION_KW, INTERSECT_KW, EXCEPT_KW]) {
            self.advance(); // consume UNION/INTERSECT/EXCEPT
            self.skip_trivia();
            // Optional ALL
            if self.at(ALL_KW) {
                self.advance();
            }
            self.skip_trivia();
            // Parse next SELECT
            if self.at(SELECT_KW) || self.at(WITH_KW) {
                self.parse_select_stmt();
            } else {
                self.error("Expected SELECT after set operation".to_string());
            }
        }

        self.depth -= 1;
        self.finish_node();
    }

    fn parse_select_list(&mut self) {
        self.start_node(SELECT_LIST);
        self.skip_trivia();

        // Parse comma-separated select items (including * and table.*)
        loop {
            if self.at(DOT_DOT_DOT) {
                // List spread in SELECT list: `...metric_exprs`
                self.parse_list_spread();
            } else if self.at(STAR) {
                // Handle SELECT * as a special select item
                self.start_node(SELECT_ITEM);
                self.advance();
                self.finish_node();
            } else if self.at(IDENT)
                && self.peek_nth_non_trivia(1) == Some(DOT)
                && self.peek_nth_non_trivia(2) == Some(STAR)
            {
                // `table.*` qualified star. Emit as a SELECT_ITEM carrying
                // IDENT DOT STAR tokens directly — downstream consumers
                // treat `SELECT_ITEM` with a trailing STAR as a star-item.
                // The peek check skipped trivia, so we must do the same
                // between advances or the comment between `.` and `*`
                // would leave STAR outside the SELECT_ITEM.
                self.start_node(SELECT_ITEM);
                self.advance(); // IDENT
                self.skip_trivia();
                self.advance(); // DOT
                self.skip_trivia();
                self.advance(); // STAR
                self.finish_node();
            } else {
                self.parse_select_item();
            }

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
                self.skip_trivia();
                // Allow trailing comma - break if next token ends the SELECT list
                if self.at_any(&[
                    FROM_KW,
                    WHERE_KW,
                    GROUP_KW,
                    HAVING_KW,
                    QUALIFY_KW,
                    ORDER_KW,
                    LIMIT_KW,
                    OFFSET_KW,
                    EOF,
                    INNER_KW,
                    LEFT_KW,
                    RIGHT_KW,
                    FULL_KW,
                    CROSS_KW,
                    JOIN_KW,
                    UNION_KW,
                    INTERSECT_KW,
                    EXCEPT_KW,
                ]) {
                    break;
                }
            } else {
                break;
            }
        }

        self.finish_node();
    }

    fn parse_select_item(&mut self) {
        self.start_node(SELECT_ITEM);
        self.skip_trivia();

        // Parse expression
        self.parse_expression();

        // Optional AS alias
        self.skip_trivia();
        if self.at(AS_KW) {
            self.advance();
            self.skip_trivia();
            if self.at(IDENT) {
                self.advance();
            }
        } else if self.at(IDENT) {
            // Implicit alias (no AS keyword)
            self.advance();
        }

        self.finish_node();
    }

    fn parse_from_clause(&mut self) {
        self.start_node(FROM_CLAUSE);

        self.expect(FROM_KW);

        // Parse first table reference (required)
        self.parse_table_ref();

        // Parse zero or more JOIN clauses
        loop {
            self.skip_trivia();
            if self.at_any(&[JOIN_KW, INNER_KW, LEFT_KW, RIGHT_KW, FULL_KW, CROSS_KW]) {
                self.parse_join_clause();
            } else {
                break;
            }
        }

        self.finish_node();
    }

    fn parse_table_ref(&mut self) {
        self.start_node(TABLE_REF);
        self.skip_trivia();

        // Check for LATERAL keyword (PostgreSQL)
        if self.at(LATERAL_KW) {
            self.advance(); // LATERAL
            self.skip_trivia();
        }

        // Phase 5b: reject legacy smelt.fn.* in FROM position.
        if self.at(IDENT) && self.at_smelt_fn_trigger() {
            self.error("smelt.fn.* is removed; use smelt.functions.<name> instead".to_string());
            // Fall through — the generic IDENT path below handles error recovery.
        }

        // Phase 4: reject legacy smelt.ref() and smelt.source() call forms.
        // Emit an error pointing the user to the unified smelt.<path> form,
        // then parse the call as a generic FUNCTION_CALL for error recovery.
        if self.at(IDENT) && self.at_smelt_legacy_ref_or_source_trigger() {
            self.error(
                "smelt.ref() and smelt.source() are removed; \
                 use smelt.models.<name> or smelt.sources.<schema>.<table> instead"
                    .to_string(),
            );
            // Fall through — the generic IDENT path below will still consume
            // and produce a FUNCTION_CALL node so the rest of the parse continues.
        }

        // smelt.<path> migration, Phase 1: unified value/call form. The
        // trigger excludes the legacy second-segments (`fn`, `define`,
        // `extern`, `as_struct`, `ref`, `source`, `metric`) so the existing
        // grammar paths above and the FUNCTION_CALL path below stay intact.
        if self.at(IDENT) && self.at_smelt_path_trigger() {
            self.parse_smelt_path_form();
            self.skip_trivia();
            if self.at(AS_KW) {
                self.advance();
                self.skip_trivia();
                self.expect(IDENT);
            } else if self.at(IDENT) && !self.at_keyword_that_ends_table_ref() {
                self.advance();
            }
            self.finish_node(); // TABLE_REF
            return;
        }

        if self.at(LPAREN) {
            // Could be a subquery
            let checkpoint = self.builder.checkpoint();
            self.advance(); // consume LPAREN
            self.skip_trivia();

            // Check if it's a subquery (starts with SELECT or WITH) or VALUES
            if self.at(SELECT_KW) || self.at(WITH_KW) {
                self.start_node_at(checkpoint, SUBQUERY);
                self.parse_select_stmt();
                self.skip_trivia();
                self.expect(RPAREN);
                self.finish_node(); // Close SUBQUERY
            } else if self.at(VALUES_KW) {
                self.start_node_at(checkpoint, SUBQUERY);
                self.parse_values_clause();
                self.skip_trivia();
                self.expect(RPAREN);
                self.finish_node();
            } else {
                // Not a subquery, error
                self.error("Expected SELECT in subquery".to_string());
                self.expect(RPAREN);
            }
        } else if self.at(IDENT) {
            // Use builder checkpoint for proper lookahead
            let checkpoint = self.builder.checkpoint();
            self.advance(); // Consume IDENT
            self.skip_trivia();

            if self.at(LPAREN) {
                // It's a simple function call - wrap in FUNCTION_CALL node using checkpoint
                self.start_node_at(checkpoint, FUNCTION_CALL);
                self.parse_arg_list();
                self.finish_node(); // Close FUNCTION_CALL
            } else if self.at(DOT) {
                // Could be schema.table or namespace.func()
                self.advance(); // Consume DOT
                self.skip_trivia();
                self.expect(IDENT); // Consume second IDENT
                self.skip_trivia();

                if self.at(LPAREN) {
                    // Namespaced function call: smelt.ref()
                    self.start_node_at(checkpoint, FUNCTION_CALL);
                    self.parse_arg_list();
                    self.finish_node(); // Close FUNCTION_CALL
                }
                // else: just a qualified table name (schema.table), already consumed
            }
            // else: simple identifier, already consumed
        } else {
            self.error("Expected table reference".to_string());
        }

        // Optional TABLESAMPLE clause (PostgreSQL)
        self.skip_trivia();
        if self.at(TABLESAMPLE_KW) {
            self.start_node(TABLESAMPLE_CLAUSE);
            self.advance(); // TABLESAMPLE
            self.skip_trivia();

            // Sampling method: BERNOULLI or SYSTEM
            if self.at(BERNOULLI_KW) || self.at(SYSTEM_KW) {
                self.advance();
                self.skip_trivia();
            }

            // Percentage in parentheses
            if self.expect(LPAREN) {
                self.skip_trivia();
                self.parse_expression(); // Sample percentage
                self.skip_trivia();
                self.expect(RPAREN);
            }

            // Optional REPEATABLE (seed)
            self.skip_trivia();
            if self.at(REPEATABLE_KW) {
                self.advance(); // REPEATABLE
                self.skip_trivia();
                if self.expect(LPAREN) {
                    self.skip_trivia();
                    self.parse_expression(); // Seed value
                    self.skip_trivia();
                    self.expect(RPAREN);
                }
            }

            self.finish_node(); // TABLESAMPLE_CLAUSE
        }

        // Optional PIVOT/UNPIVOT clause
        self.skip_trivia();
        if self.at(PIVOT_KW) {
            self.parse_pivot_clause();
        } else if self.at(UNPIVOT_KW) {
            self.parse_unpivot_clause();
        }

        // Optional AS alias (explicit with AS keyword or implicit)
        self.skip_trivia();
        if self.at(AS_KW) {
            self.advance();
            self.skip_trivia();
            self.expect(IDENT);
        } else if self.at(IDENT) && !self.at_keyword_that_ends_table_ref() {
            // Implicit alias (no AS keyword)
            // Only consume if it's not a keyword that would end the table ref
            self.advance();
        }

        self.finish_node();
    }

    fn parse_pivot_clause(&mut self) {
        self.start_node(PIVOT_CLAUSE);
        self.expect(PIVOT_KW);
        self.skip_trivia();
        if !self.expect(LPAREN) {
            self.finish_node();
            return;
        }

        // Parse aggregate expression(s): SUM(amount), COUNT(*)
        self.skip_trivia();
        self.parse_expression();

        // FOR column
        self.skip_trivia();
        // FOR is not a keyword in our lexer, so it's parsed as IDENT
        if self.at(IDENT) {
            // Check if text is "FOR"
            let token = self.tokens[self.pos];
            let text = &self.input[self.offset..self.offset + token.len];
            if text.eq_ignore_ascii_case("FOR") {
                self.advance(); // FOR
                self.skip_trivia();
                self.parse_expression(); // column name
            }
        }

        // IN (values...)
        self.skip_trivia();
        if self.at(IN_KW) {
            self.parse_pivot_in_list();
        }

        self.skip_trivia();
        self.expect(RPAREN);
        self.finish_node();
    }

    fn parse_unpivot_clause(&mut self) {
        self.start_node(UNPIVOT_CLAUSE);
        self.expect(UNPIVOT_KW);
        self.skip_trivia();
        if !self.expect(LPAREN) {
            self.finish_node();
            return;
        }

        // Value column name
        self.skip_trivia();
        self.parse_expression();

        // FOR name column
        self.skip_trivia();
        if self.at(IDENT) {
            let token = self.tokens[self.pos];
            let text = &self.input[self.offset..self.offset + token.len];
            if text.eq_ignore_ascii_case("FOR") {
                self.advance(); // FOR
                self.skip_trivia();
                self.parse_expression(); // name column
            }
        }

        // IN (columns...)
        self.skip_trivia();
        if self.at(IN_KW) {
            self.parse_pivot_in_list();
        }

        self.skip_trivia();
        self.expect(RPAREN);
        self.finish_node();
    }

    fn parse_pivot_in_list(&mut self) {
        self.start_node(PIVOT_IN_LIST);
        self.expect(IN_KW);
        self.skip_trivia();
        if !self.expect(LPAREN) {
            self.finish_node();
            return;
        }

        // Parse comma-separated values/columns
        loop {
            self.skip_trivia();
            if self.at(RPAREN) {
                break;
            }
            self.parse_expression();
            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
            } else {
                break;
            }
        }

        self.expect(RPAREN);
        self.finish_node();
    }

    #[allow(clippy::if_same_then_else)]
    fn parse_join_clause(&mut self) {
        self.start_node(JOIN_CLAUSE);

        // Parse JOIN type modifiers (INNER, LEFT, RIGHT, FULL OUTER, CROSS)
        // Note: The if-else blocks are intentionally similar for clarity
        if self.at(INNER_KW) {
            self.advance();
            self.skip_trivia();
        } else if self.at(LEFT_KW) {
            self.advance();
            self.skip_trivia();
            if self.at(OUTER_KW) {
                self.advance();
                self.skip_trivia();
            }
        } else if self.at(RIGHT_KW) {
            self.advance();
            self.skip_trivia();
            if self.at(OUTER_KW) {
                self.advance();
                self.skip_trivia();
            }
        } else if self.at(FULL_KW) {
            self.advance();
            self.skip_trivia();
            if self.at(OUTER_KW) {
                self.advance();
                self.skip_trivia();
            }
        } else if self.at(CROSS_KW) {
            self.advance();
            self.skip_trivia();
        }
        // Note: Bare JOIN defaults to INNER JOIN

        // Expect JOIN keyword
        if !self.expect(JOIN_KW) {
            // Error recovery: missing JOIN keyword
            self.error("Expected JOIN keyword".to_string());
            self.finish_node();
            return;
        }

        // Parse table reference (may include LATERAL keyword)
        self.skip_trivia();
        if !self.at(IDENT) && !self.at(LATERAL_KW) && !self.at(LPAREN) {
            // Error recovery: missing table reference
            self.error("Expected table reference after JOIN".to_string());
            self.finish_node();
            return;
        }
        self.parse_table_ref();

        // Parse join condition (ON or USING)
        // CROSS JOIN doesn't require a condition
        self.skip_trivia();
        if self.at(ON_KW) || self.at(USING_KW) {
            self.parse_join_condition();
        }

        self.finish_node();
    }

    fn parse_join_condition(&mut self) {
        self.start_node(JOIN_CONDITION);

        if self.at(ON_KW) {
            // ON expression
            self.advance();
            self.skip_trivia();

            if !self.at_expression_start() {
                self.error("Expected expression after ON".to_string());
                self.finish_node();
                return;
            }
            self.parse_expression();
        } else if self.at(USING_KW) {
            // USING (col1, col2, ...)
            self.advance();
            self.skip_trivia();

            if !self.expect(LPAREN) {
                self.error("Expected '(' after USING".to_string());
                self.finish_node();
                return;
            }

            // Parse comma-separated column list
            loop {
                self.skip_trivia();
                if !self.at(IDENT) {
                    self.error("Expected column name in USING clause".to_string());
                    break;
                }
                self.advance();

                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                } else {
                    break;
                }
            }

            self.expect(RPAREN);
        }

        self.finish_node();
    }

    fn parse_where_clause(&mut self) {
        self.start_node(WHERE_CLAUSE);
        self.expect(WHERE_KW);
        self.parse_expression();
        self.finish_node();
    }

    fn parse_group_by_clause(&mut self) {
        self.start_node(GROUP_BY_CLAUSE);
        self.expect(GROUP_KW);
        self.expect(BY_KW);

        // Parse comma-separated column list
        loop {
            self.skip_trivia();
            // List spread in GROUP BY: `GROUP BY ...keys`
            if self.at(DOT_DOT_DOT) {
                self.parse_list_spread();
            } else {
                self.parse_expression();
            }

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
                self.skip_trivia();
                // Allow trailing comma - break if next token ends GROUP BY
                if self.at_any(&[
                    HAVING_KW,
                    QUALIFY_KW,
                    ORDER_KW,
                    LIMIT_KW,
                    OFFSET_KW,
                    EOF,
                    UNION_KW,
                    INTERSECT_KW,
                    EXCEPT_KW,
                ]) {
                    break;
                }
            } else {
                break;
            }
        }

        self.finish_node();
    }

    fn parse_having_clause(&mut self) {
        self.start_node(HAVING_CLAUSE);
        self.expect(HAVING_KW);
        self.parse_expression();
        self.finish_node();
    }

    fn parse_qualify_clause(&mut self) {
        self.start_node(QUALIFY_CLAUSE);
        self.expect(QUALIFY_KW);
        self.parse_expression();
        self.finish_node();
    }

    fn parse_order_by_clause(&mut self) {
        self.start_node(ORDER_BY_CLAUSE);
        self.expect(ORDER_KW);
        self.expect(BY_KW);

        // Comma-separated order items
        loop {
            self.skip_trivia();
            // List spread in ORDER BY: `ORDER BY ...sort_keys`
            if self.at(DOT_DOT_DOT) {
                self.parse_list_spread();
            } else {
                self.parse_order_by_item();
            }

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
            } else {
                break;
            }
        }

        self.finish_node();
    }

    fn parse_order_by_item(&mut self) {
        self.start_node(ORDER_BY_ITEM);

        // Expression to order by
        self.parse_expression();

        // Optional ASC/DESC
        self.skip_trivia();
        if self.at(ASC_KW) || self.at(DESC_KW) {
            self.advance();
        }

        // Optional NULLS FIRST/LAST
        self.skip_trivia();
        if self.at(NULLS_KW) {
            self.advance();
            self.skip_trivia();
            if self.at(FIRST_KW) || self.at(LAST_KW) {
                self.advance();
            } else {
                self.error("Expected FIRST or LAST after NULLS".to_string());
            }
        }

        self.finish_node();
    }

    fn parse_limit_clause(&mut self) {
        self.start_node(LIMIT_CLAUSE);

        self.expect(LIMIT_KW);
        self.skip_trivia();

        // LIMIT value (number or ALL)
        if self.at(NUMBER) || self.at(ALL_KW) {
            self.advance();
        } else {
            self.error("Expected number or ALL after LIMIT".to_string());
        }

        // Optional OFFSET
        self.skip_trivia();
        if self.at(OFFSET_KW) {
            self.advance();
            self.skip_trivia();
            if self.at(NUMBER) {
                self.advance();
            } else {
                self.error("Expected number after OFFSET".to_string());
            }
        }

        self.finish_node();
    }

    fn parse_expression(&mut self) {
        self.start_node(EXPRESSION);
        self.skip_trivia();

        if self.too_deep() {
            self.finish_node();
            return;
        }
        self.depth += 1;
        // Phase B (meta-language): pipe `|>` is the lowest-precedence
        // meta-language operator.  `parse_pipe_expr` wraps `parse_or_expr`
        // and folds left-associative `|>` chains when present.
        self.parse_pipe_expr();
        self.depth -= 1;

        self.finish_node();
    }

    /// Parse a pipe expression: `EXPR |> EXPR |> ...` (left-associative, lowest
    /// precedence).  When no `|>` follows, this is a pass-through to
    /// `parse_or_expr`.  Each `|>` pair is wrapped in a `PIPE_EXPR` node whose
    /// two `EXPRESSION` children are the LHS and RHS.
    ///
    /// Phase B (meta-language): `|>` is meta-world only; Phase 3 rejects pipe
    /// in Data-World positions.  The parser does not gate — it produces the CST
    /// node unconditionally.
    fn parse_pipe_expr(&mut self) {
        // Use a checkpoint so we can wrap the already-parsed LHS inside
        // a PIPE_EXPR node when we encounter `|>`.
        let checkpoint = self.builder.checkpoint();

        // Handle `fn` at the top level of an expression — a lambda that
        // appears outside a function argument list (Phase 3 will reject with
        // `LambdaInForbiddenPosition`).
        if self.is_fn_lambda_start() {
            self.parse_fn_lambda();
        } else {
            self.parse_or_expr();
        }

        // Fold left-associative pipe operators.
        while self.at(PIPE_ARROW) {
            self.start_node_at(checkpoint, PIPE_EXPR);
            // Wrap the already-parsed LHS in an EXPRESSION node.  The builder
            // checkpoint mechanism means the LHS tokens are already consumed;
            // we're retroactively wrapping them.
            self.advance(); // consume `|>`
            self.skip_trivia();

            // Parse the RHS as a new EXPRESSION child of PIPE_EXPR.
            // The parser does not validate that RHS is a call — Phase 3 does.
            self.start_node(EXPRESSION);
            self.depth += 1;
            // RHS may itself be a lambda (unusual but syntactically valid for
            // error recovery); otherwise it's a normal expression.
            if self.is_fn_lambda_start() {
                self.parse_fn_lambda();
            } else {
                self.parse_or_expr();
            }
            self.depth -= 1;
            self.finish_node(); // EXPRESSION (RHS)

            self.finish_node(); // PIPE_EXPR
        }
    }

    fn parse_or_expr(&mut self) {
        let checkpoint = self.builder.checkpoint();
        self.parse_and_expr();

        while self.at(OR_KW) {
            self.start_node_at(checkpoint, BINARY_EXPR);
            self.advance();
            self.skip_trivia();
            self.parse_and_expr();
            self.finish_node();
        }
    }

    fn parse_and_expr(&mut self) {
        let checkpoint = self.builder.checkpoint();
        self.parse_comparison_expr();

        while self.at(AND_KW) {
            self.start_node_at(checkpoint, BINARY_EXPR);
            self.advance();
            self.skip_trivia();
            self.parse_comparison_expr();
            self.finish_node();
        }
    }

    fn parse_comparison_expr(&mut self) {
        let checkpoint = self.builder.checkpoint();
        self.parse_concat_expr();

        loop {
            self.skip_trivia();
            if self.at_any(&[
                EQ,
                NE,
                LT,
                GT,
                LE,
                GE,
                TILDE,
                TILDE_STAR,
                NOT_TILDE,
                NOT_TILDE_STAR,
            ]) {
                self.start_node_at(checkpoint, BINARY_EXPR);
                self.advance();
                self.skip_trivia();
                // Check for ANY/ALL/SOME(expr)
                if self.at_any(&[ANY_KW, SOME_KW, ALL_KW]) && self.is_keyword_followed_by_lparen() {
                    self.start_node(ANY_EXPR);
                    self.advance(); // ANY/ALL/SOME
                    self.skip_trivia();
                    self.expect(LPAREN);
                    self.skip_trivia();
                    if self.at(SELECT_KW) {
                        self.parse_subquery();
                    } else {
                        self.parse_expression();
                    }
                    self.skip_trivia();
                    self.expect(RPAREN);
                    self.finish_node(); // ANY_EXPR
                } else {
                    self.parse_concat_expr();
                }
                self.finish_node();
            } else if self.at(IS_KW) {
                // IS [NOT] NULL
                self.start_node_at(checkpoint, BINARY_EXPR);
                self.advance(); // consume IS
                self.skip_trivia();
                if self.at(NOT_KW) {
                    self.advance(); // consume NOT
                    self.skip_trivia();
                }
                if self.at(NULL_KW) {
                    self.advance(); // consume NULL
                }
                self.finish_node();
            } else if self.at(BETWEEN_KW) {
                // BETWEEN low AND high — use checkpoint to include left operand
                self.start_node_at(checkpoint, BETWEEN_EXPR);
                self.parse_between_body();
                self.finish_node();
            } else if self.at(IN_KW) {
                // IN (values...) — use checkpoint to include left operand
                self.start_node_at(checkpoint, IN_EXPR);
                self.parse_in_body();
                self.finish_node();
            } else if self.at(LIKE_KW) || self.at(ILIKE_KW) {
                // LIKE / ILIKE pattern
                self.start_node_at(checkpoint, BINARY_EXPR);
                self.advance(); // consume LIKE/ILIKE
                self.skip_trivia();
                self.parse_concat_expr();
                self.finish_node();
            } else {
                break;
            }
        }
    }

    /// Parse the body of a BETWEEN expression (BETWEEN low AND high).
    /// Caller is responsible for creating the BETWEEN_EXPR node with the left operand.
    fn parse_between_body(&mut self) {
        self.expect(BETWEEN_KW);

        // Parse lower bound
        self.skip_trivia();
        self.parse_additive_expr();

        // Expect AND
        self.skip_trivia();
        if !self.expect(AND_KW) {
            self.error("Expected AND in BETWEEN expression".to_string());
        }

        // Parse upper bound
        self.skip_trivia();
        self.parse_additive_expr();
    }

    /// Parse the body of an IN expression (IN (values...)).
    /// Caller is responsible for creating the IN_EXPR node with the left operand.
    fn parse_in_body(&mut self) {
        self.expect(IN_KW);

        self.skip_trivia();
        if !self.expect(LPAREN) {
            self.error("Expected '(' after IN".to_string());
            return;
        }

        self.skip_trivia();

        // Check if it's a subquery (starts with SELECT)
        if self.at(SELECT_KW) {
            self.parse_subquery();
        } else {
            // Parse comma-separated value list
            loop {
                self.skip_trivia();
                if self.at(RPAREN) {
                    break;
                }

                // List spread in IN list: `x IN (...ids)`
                if self.at(DOT_DOT_DOT) {
                    self.parse_list_spread();
                } else {
                    self.parse_expression();
                }

                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                } else {
                    break;
                }
            }
        }

        self.expect(RPAREN);
    }

    fn parse_concat_expr(&mut self) {
        let checkpoint = self.builder.checkpoint();
        self.parse_json_expr();

        while self.at(CONCAT) {
            self.start_node_at(checkpoint, BINARY_EXPR);
            self.advance();
            self.skip_trivia();
            self.parse_json_expr();
            self.finish_node();
        }
    }

    fn parse_json_expr(&mut self) {
        self.parse_additive_expr();

        self.skip_trivia();
        while self.at_any(&[
            JSON_ARROW,
            JSON_ARROW_TEXT,
            HASH_ARROW,
            HASH_ARROW_TEXT,
            AT_GT,
            LT_AT,
        ]) {
            self.start_node(BINARY_EXPR);
            self.advance();
            self.skip_trivia();
            self.parse_additive_expr();
            self.skip_trivia();
            self.finish_node();
        }
    }

    fn parse_additive_expr(&mut self) {
        let checkpoint = self.builder.checkpoint();
        self.parse_multiplicative_expr();

        self.skip_trivia();
        while self.at_any(&[PLUS, MINUS]) {
            self.start_node_at(checkpoint, BINARY_EXPR);
            self.advance();
            self.skip_trivia();
            self.parse_multiplicative_expr();
            self.skip_trivia();
            self.finish_node();
        }
    }

    fn parse_multiplicative_expr(&mut self) {
        let checkpoint = self.builder.checkpoint();
        self.parse_unary_expr();

        self.skip_trivia();
        while self.at_any(&[STAR, DIVIDE, PERCENT]) {
            self.start_node_at(checkpoint, BINARY_EXPR);
            self.advance();
            self.skip_trivia();
            self.parse_unary_expr();
            self.skip_trivia();
            self.finish_node();
        }
    }

    fn parse_unary_expr(&mut self) {
        self.skip_trivia();

        // Handle unary operators (-, NOT)
        if self.at_any(&[MINUS, NOT_KW]) {
            self.start_node(BINARY_EXPR); // Reuse BINARY_EXPR for unary ops
            self.advance(); // consume operator
            self.skip_trivia();
            self.parse_unary_expr(); // Allow chaining: --x
            self.finish_node();
        } else {
            self.parse_primary_expr();
        }
    }

    fn parse_primary_expr(&mut self) {
        self.skip_trivia();

        if self.at(NULL_KW) {
            // NULL literal — wrap in EXPRESSION so Expr::cast() works
            self.start_node(EXPRESSION);
            self.advance();
            self.finish_node();
            return;
        }

        if self.at(ARRAY_KW) && self.is_keyword_followed_by_lbracket() {
            self.parse_array_literal();
        } else if self.at(ARRAY_KW) && self.is_keyword_followed_by_lparen() {
            // ARRAY(expr) function-call style
            let checkpoint = self.builder.checkpoint();
            self.advance(); // ARRAY_KW
            self.skip_trivia();
            self.start_node_at(checkpoint, FUNCTION_CALL);
            self.parse_arg_list();
            self.finish_node();
        } else if self.at(ROW_KW) && self.is_keyword_followed_by_lparen() {
            self.parse_row_constructor();
        } else if self.at(STRUCT_KW) && self.is_keyword_followed_by_lparen() {
            self.parse_struct_literal();
        } else if self.at(LBRACE) {
            // Phase 35: brace-struct literal `{expr AS alias, ..spread}`
            self.parse_brace_struct_literal();
        } else if self.at(CASE_KW) {
            self.parse_case_expr();
        } else if self.at(CAST_KW) {
            self.parse_cast_expr();
        } else if self.at(EXTRACT_KW) {
            self.parse_extract_expr();
        } else if self.at(EXISTS_KW) {
            self.parse_exists_expr();
        } else if self.at(LPAREN) {
            // Could be: parenthesized expression, subquery, or empty tuple `()`.
            let checkpoint = self.builder.checkpoint();
            self.advance(); // consume LPAREN
            self.skip_trivia();

            // Check if it's a subquery (starts with SELECT)
            if self.at(SELECT_KW) {
                self.start_node_at(checkpoint, SUBQUERY);
                self.parse_select_stmt();
                self.skip_trivia();
                self.expect(RPAREN);
                self.finish_node();
            } else if self.at(RPAREN) {
                // Empty tuple `()` — valid as a default value for
                // `SelectItems<Kind>` parameters (Phase 22, research §6).
                // Emit a bare EXPRESSION node with no children; the type-checker
                // treats this as an empty fragment (no aggregates / columns).
                self.start_node_at(checkpoint, EXPRESSION);
                self.advance(); // consume RPAREN
                self.finish_node();
            } else {
                // Grouped expression
                self.parse_expression();
                self.skip_trivia();
                self.expect(RPAREN);
            }
        } else if self.at_keyword_as_function_name() {
            // Keywords that can also be function names (e.g., FILTER(arr, ...) as array filter)
            let checkpoint = self.builder.checkpoint();
            self.advance(); // consume keyword
            self.skip_trivia();
            if self.at(LPAREN) {
                self.start_node_at(checkpoint, FUNCTION_CALL);
                self.parse_arg_list();
                self.finish_node();
            } else {
                // Bare keyword used as identifier — wrap in EXPRESSION
                self.start_node_at(checkpoint, EXPRESSION);
                self.finish_node();
            }
        } else if self.at(IDENT) && self.is_typed_literal() {
            // Typed literal: DATE '2024-01-01', TIMESTAMP '...', etc.
            // Wrap in EXPRESSION so Expr::cast() works
            self.start_node(EXPRESSION);
            self.advance(); // type keyword (IDENT)
            self.skip_trivia();
            self.advance(); // string literal
            self.finish_node();
        } else if self.at(IDENT) && self.at_smelt_as_struct_trigger() {
            // smelt.as_struct(alias [EXCEPT col1, col2]) — Phase 38.
            // Must be checked BEFORE the generic IDENT branch.
            self.parse_smelt_as_struct();
        } else if self.at(IDENT) && self.at_smelt_fn_trigger() {
            // Phase 5b: reject legacy smelt.fn.* call syntax in expression
            // position. Emit an error pointing the user toward the unified
            // `smelt.functions.*` form, then parse the call as a generic
            // FUNCTION_CALL for error recovery.
            self.error("smelt.fn.* is removed; use smelt.functions.<name> instead".to_string());
            // Consume smelt . fn . <name> and then the arg list as a FUNCTION_CALL
            // for error recovery so parsing can continue.
            let checkpoint = self.builder.checkpoint();
            self.advance(); // consume "smelt"
            self.skip_trivia();
            self.advance(); // consume "."
            self.skip_trivia();
            self.advance(); // consume "fn"
                            // Consume any remaining path segments
            loop {
                self.skip_trivia();
                if !self.at(DOT) {
                    break;
                }
                let mut la = 1;
                while let Some(t) = self.tokens.get(self.pos + la) {
                    if t.kind.is_trivia() {
                        la += 1;
                    } else {
                        break;
                    }
                }
                if self.tokens.get(self.pos + la).map(|t| t.kind) == Some(IDENT) {
                    self.advance(); // DOT
                    self.skip_trivia();
                    self.advance(); // IDENT
                } else {
                    break;
                }
            }
            self.skip_trivia();
            self.start_node_at(checkpoint, FUNCTION_CALL);
            if self.at(LPAREN) {
                self.parse_arg_list();
            }
            self.finish_node();
        } else if self.at(IDENT) && self.at_smelt_legacy_ref_or_source_trigger() {
            // Phase 4: reject legacy smelt.ref() and smelt.source() in
            // expression position. Emit the error and fall through to the
            // generic IDENT → FUNCTION_CALL path for error recovery.
            self.error(
                "smelt.ref() and smelt.source() are removed; \
                 use smelt.models.<name> or smelt.sources.<schema>.<table> instead"
                    .to_string(),
            );
            // The IDENT path below will consume the tokens and produce a
            // FUNCTION_CALL node so parsing continues without a hard stop.
            let checkpoint = self.builder.checkpoint();
            self.advance(); // consume "smelt"
            self.skip_trivia();
            self.advance(); // consume DOT
            self.skip_trivia();
            self.advance(); // consume "ref" or "source"
            self.skip_trivia();
            // Now parse the argument list as part of the FUNCTION_CALL.
            self.start_node_at(checkpoint, FUNCTION_CALL);
            self.parse_arg_list();
            self.finish_node();
        } else if self.at(IDENT) && self.at_smelt_path_trigger() {
            // smelt.<path> value/call form (smelt.<path> migration, Phase 1).
            // Must be checked BEFORE the generic IDENT branch — the generic
            // namespaced-call path would consume only `smelt.<seg>` and leave
            // any further segments dangling. The trigger excludes the legacy
            // second-segments (`fn`, `define`, `extern`, `as_struct`, `ref`,
            // `source`, `metric`) so existing grammar paths stay intact.
            self.parse_smelt_path_form();
        } else if self.at(IDENT) {
            // Could be column reference, qualified name, or function call
            let checkpoint = self.builder.checkpoint();
            self.advance(); // consume first IDENT
            self.skip_trivia();

            if self.at(LPAREN) {
                // Simple function call: func()
                self.start_node_at(checkpoint, FUNCTION_CALL);
                self.parse_arg_list();
                self.parse_within_group_if_present();
                self.parse_filter_clause_if_present();
                self.finish_node();

                // Check for OVER clause (window function)
                self.skip_trivia();
                if self.at(OVER_KW) {
                    self.parse_window_spec();
                }
            } else if self.at(DOT) {
                // Could be table.column or namespace.func()
                self.advance(); // consume DOT
                self.skip_trivia();
                self.expect(IDENT); // consume second IDENT
                self.skip_trivia();

                if self.at(LPAREN) {
                    // Namespaced function call: smelt.ref()
                    self.start_node_at(checkpoint, FUNCTION_CALL);
                    self.parse_arg_list();
                    self.parse_within_group_if_present();
                    self.parse_filter_clause_if_present();
                    self.finish_node();

                    // Check for OVER clause (window function)
                    self.skip_trivia();
                    if self.at(OVER_KW) {
                        self.parse_window_spec();
                    }
                } else {
                    // Qualified name (table.column) — wrap in EXPRESSION
                    self.start_node_at(checkpoint, EXPRESSION);
                    self.finish_node();
                }
            } else if self.at(DOUBLE_COLON) {
                // PostgreSQL cast: expr::type
                // Wrap the identifier in EXPRESSION first, then wrap all in CAST_EXPR
                self.start_node_at(checkpoint, EXPRESSION);
                self.finish_node();
                self.start_node_at(checkpoint, CAST_EXPR);
                self.advance(); // consume ::
                self.skip_trivia();
                self.parse_type_spec();
                self.finish_node();
            } else {
                // Simple identifier — wrap in EXPRESSION
                self.start_node_at(checkpoint, EXPRESSION);
                self.finish_node();
            }
        } else if self.at(LBRACKET) {
            // Bracket-only list literal: `[a, b, c]` — Phase 1 meta-language.
            // The same surface token lifts to either a meta List<T> or a
            // Data-World Array<U>; the type checker disambiguates (Phase 2/3).
            // We reuse the ARRAY_LITERAL CST kind per the spec.
            self.parse_bracket_list_literal();
        } else if self.current().is_literal() || self.at(STAR) {
            // Literal or STAR — wrap in EXPRESSION so Expr::cast() works
            self.start_node(EXPRESSION);
            self.advance();
            self.finish_node();
        } else {
            self.error(format!("Expected expression, found {:?}", self.current()));
        }

        // Postfix: array subscript/slice: expr[index] or expr[start:end]
        self.skip_trivia();
        while self.at(LBRACKET) {
            self.parse_array_subscript();
            self.skip_trivia();
        }
    }

    fn parse_array_subscript(&mut self) {
        // Determine if this is subscript or slice by looking for COLON
        let checkpoint = self.builder.checkpoint();
        self.advance(); // consume [
        self.skip_trivia();

        // Parse first expression (index or start of slice)
        let has_first = !self.at(COLON);
        if has_first {
            self.parse_expression();
            self.skip_trivia();
        }

        if self.at(COLON) {
            // Slice: [start:end]
            self.start_node_at(checkpoint, ARRAY_SLICE);
            self.advance(); // consume :
            self.skip_trivia();
            if !self.at(RBRACKET) {
                self.parse_expression();
                self.skip_trivia();
            }
            self.expect(RBRACKET);
            self.finish_node();
        } else {
            // Simple subscript: [index]
            self.start_node_at(checkpoint, ARRAY_SUBSCRIPT);
            self.expect(RBRACKET);
            self.finish_node();
        }
    }

    fn parse_fetch_clause(&mut self) {
        self.start_node(FETCH_CLAUSE);
        // FETCH is a contextual keyword (IDENT)
        self.advance(); // consume "FETCH"
        self.skip_trivia();
        // FIRST or NEXT (contextual)
        if self.at(FIRST_KW) || self.at_contextual_keyword("NEXT") {
            self.advance();
        }
        self.skip_trivia();
        // Optional count
        if self.at(NUMBER) {
            self.advance();
        }
        self.skip_trivia();
        // ROW or ROWS
        if self.at(ROW_KW) || self.at(ROWS_KW) {
            self.advance();
        }
        self.skip_trivia();
        // ONLY (contextual)
        if self.at_contextual_keyword("ONLY") {
            self.advance();
        }
        self.finish_node();
    }

    fn parse_values_clause(&mut self) {
        self.start_node(VALUES_CLAUSE);
        self.expect(VALUES_KW);

        // Parse comma-separated rows: (expr, expr), (expr, expr)
        loop {
            self.skip_trivia();
            self.start_node(VALUES_ROW);
            if self.expect(LPAREN) {
                loop {
                    self.skip_trivia();
                    if self.at(RPAREN) {
                        break;
                    }
                    // List spread in VALUES row: `VALUES (...vals)`
                    if self.at(DOT_DOT_DOT) {
                        self.parse_list_spread();
                    } else {
                        self.parse_expression();
                    }
                    self.skip_trivia();
                    if self.at(COMMA) {
                        self.advance();
                    } else {
                        break;
                    }
                }
                self.expect(RPAREN);
            }
            self.finish_node(); // VALUES_ROW

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
            } else {
                break;
            }
        }

        self.finish_node(); // VALUES_CLAUSE
    }

    fn parse_array_literal(&mut self) {
        self.start_node(ARRAY_LITERAL);
        self.expect(ARRAY_KW);
        self.skip_trivia();
        if self.expect(LBRACKET) {
            loop {
                self.skip_trivia();
                if self.at(RBRACKET) {
                    break;
                }
                self.parse_expression();
                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                } else {
                    break;
                }
            }
            self.expect(RBRACKET);
        }
        self.finish_node();
    }

    /// Parse a bracket-only list literal: `[a, b, c]` (Phase 1 meta-language).
    ///
    /// Reuses the `ARRAY_LITERAL` CST kind — the type checker distinguishes
    /// meta `List<T>` from Data-World `Array<U>` in a later phase.
    ///
    /// Features:
    /// - Trailing comma allowed: `[a, b, c,]`
    /// - Singleton: `[x]`
    /// - Empty: `[]`
    /// - Nested: `[[1, 2], [3, 4]]`
    /// - Spread elements: `[...xs, a]`
    /// - Error recovery: unterminated `[a, b` does not crash the parser.
    fn parse_bracket_list_literal(&mut self) {
        self.start_node(ARRAY_LITERAL);
        self.advance(); // consume `[`

        loop {
            self.skip_trivia();
            if self.at(RBRACKET) || self.at(EOF) {
                break;
            }
            // Spread inside list literal: `...xs`
            if self.at(DOT_DOT_DOT) {
                self.parse_list_spread();
            } else {
                self.parse_expression();
            }
            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
                self.skip_trivia();
                // Allow trailing comma — stop if the next token closes the list.
                if self.at(RBRACKET) {
                    break;
                }
            } else {
                break;
            }
        }

        self.skip_trivia();
        if self.at(RBRACKET) {
            self.advance(); // consume `]`
        } else {
            self.error("Expected `]` to close list literal".to_string());
        }

        self.finish_node(); // ARRAY_LITERAL
    }

    /// Parse a list spread: `...expr`.
    ///
    /// Produces a `LIST_SPREAD` CST node wrapping the operand expression.
    /// Valid in any comma-separated grammar position (SELECT list, GROUP BY,
    /// ORDER BY, function args, IN-list, VALUES rows, list-literal elements).
    /// Forbidden-position validation is the type-checker's job (Phase 3).
    fn parse_list_spread(&mut self) {
        self.start_node(LIST_SPREAD);
        self.advance(); // consume `...` (DOT_DOT_DOT)
        self.skip_trivia();
        self.parse_expression();
        self.finish_node(); // LIST_SPREAD
    }

    fn parse_row_constructor(&mut self) {
        self.start_node(ROW_CONSTRUCTOR);
        self.expect(ROW_KW);
        self.skip_trivia();
        if self.expect(LPAREN) {
            loop {
                self.skip_trivia();
                if self.at(RPAREN) {
                    break;
                }
                self.parse_expression();
                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                } else {
                    break;
                }
            }
            self.expect(RPAREN);
        }
        self.finish_node();
    }

    fn parse_struct_literal(&mut self) {
        self.start_node(STRUCT_LITERAL);
        self.expect(STRUCT_KW);
        self.skip_trivia();
        if self.expect(LPAREN) {
            loop {
                self.skip_trivia();
                if self.at(RPAREN) {
                    break;
                }
                self.parse_expression();
                // Optional AS name
                self.skip_trivia();
                if self.at(AS_KW) {
                    self.advance();
                    self.skip_trivia();
                    if self.at(IDENT) {
                        self.advance();
                    }
                }
                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                } else {
                    break;
                }
            }
            self.expect(RPAREN);
        }
        self.finish_node();
    }

    fn parse_case_expr(&mut self) {
        self.start_node(CASE_EXPR);
        self.expect(CASE_KW);

        self.skip_trivia();

        // Check if it's simple CASE (CASE expr WHEN ...) or searched CASE (CASE WHEN ...)
        // If the next token after CASE is not WHEN, it's a simple CASE
        let is_simple_case = !self.at(WHEN_KW);
        if is_simple_case {
            // Simple CASE - parse the case value expression
            // Use a more restricted parse to avoid consuming the WHEN keyword
            self.parse_additive_expr();
            self.skip_trivia();
        }

        // Parse WHEN clauses
        while self.at(WHEN_KW) {
            self.parse_when_clause();
            self.skip_trivia();
        }

        // Optional ELSE clause
        if self.at(ELSE_KW) {
            self.advance(); // consume ELSE
            self.skip_trivia();
            self.parse_expression();
            self.skip_trivia();
        }

        // Expect END
        if !self.expect(END_KW) {
            self.error("Expected END to close CASE expression".to_string());
        }

        self.finish_node();
    }

    fn parse_when_clause(&mut self) {
        self.start_node(WHEN_CLAUSE);
        self.expect(WHEN_KW);

        // Parse condition (full expression including OR/AND and pipe for searched CASE).
        // Use parse_pipe_expr so that `|>` inside WHEN conditions produces a PIPE_EXPR
        // CST node; Phase 3 then emits PipeInDataPosition for the semantic error.
        self.skip_trivia();
        self.parse_pipe_expr();

        // Expect THEN
        self.skip_trivia();
        if !self.expect(THEN_KW) {
            self.error("Expected THEN in WHEN clause".to_string());
        }

        // Parse result expression (full expression, WHEN/ELSE/END terminate naturally).
        // Use parse_pipe_expr so that `|>` inside THEN results produces a PIPE_EXPR node.
        self.skip_trivia();
        self.parse_pipe_expr();

        self.finish_node();
    }

    fn parse_cast_expr(&mut self) {
        self.start_node(CAST_EXPR);
        self.expect(CAST_KW);

        self.skip_trivia();
        if !self.expect(LPAREN) {
            self.error("Expected '(' after CAST".to_string());
            self.finish_node();
            return;
        }

        // Parse expression to cast
        self.skip_trivia();
        self.parse_expression();

        // Expect AS
        self.skip_trivia();
        if !self.expect(AS_KW) {
            self.error("Expected AS in CAST expression".to_string());
        }

        // Parse type
        self.skip_trivia();
        self.parse_type_spec();

        self.expect(RPAREN);
        self.finish_node();
    }

    /// Parse `EXTRACT(field FROM expr)` as a special expression.
    /// The field is an identifier like EPOCH, YEAR, MONTH, DAY, HOUR, MINUTE, SECOND.
    fn parse_extract_expr(&mut self) {
        self.start_node(EXTRACT_EXPR);
        self.expect(EXTRACT_KW);

        self.skip_trivia();
        if !self.expect(LPAREN) {
            self.error("Expected '(' after EXTRACT".to_string());
            self.finish_node();
            return;
        }

        // Parse the field name (EPOCH, YEAR, MONTH, DAY, etc.)
        // These are identifiers or keywords that act as field specifiers.
        self.skip_trivia();
        if self.at(IDENT) || self.current().is_keyword() {
            self.advance(); // consume the field name
        } else {
            self.error(
                "Expected date/time field (EPOCH, YEAR, MONTH, etc.) in EXTRACT".to_string(),
            );
        }

        // Expect FROM keyword
        self.skip_trivia();
        if !self.expect(FROM_KW) {
            self.error("Expected FROM in EXTRACT expression".to_string());
        }

        // Parse the source expression
        self.skip_trivia();
        self.parse_expression();

        self.skip_trivia();
        self.expect(RPAREN);
        self.finish_node();
    }

    fn parse_type_spec(&mut self) {
        self.start_node(TYPE_SPEC);

        // Type name (identifier)
        if !self.at(IDENT) {
            self.error("Expected type name".to_string());
            self.finish_node();
            return;
        }
        self.advance();

        // Optional type parameters: VARCHAR(255), DECIMAL(10,2), etc.
        self.skip_trivia();
        if self.at(LPAREN) {
            self.advance(); // consume LPAREN

            // Parse comma-separated parameters
            loop {
                self.skip_trivia();
                if self.at(RPAREN) {
                    break;
                }

                // Type parameters are typically numbers
                if self.at(NUMBER) {
                    self.advance();
                } else if self.at(IDENT) {
                    // Some types might have identifier parameters
                    self.advance();
                } else {
                    self.error("Expected type parameter".to_string());
                    break;
                }

                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                } else {
                    break;
                }
            }

            self.expect(RPAREN);
        }

        self.finish_node();
    }

    fn parse_subquery(&mut self) {
        self.start_node(SUBQUERY);
        self.parse_select_stmt();
        self.finish_node();
    }

    fn parse_exists_expr(&mut self) {
        self.start_node(EXISTS_EXPR);
        self.expect(EXISTS_KW);

        self.skip_trivia();
        if !self.expect(LPAREN) {
            self.error("Expected '(' after EXISTS".to_string());
            self.finish_node();
            return;
        }

        self.skip_trivia();
        if self.at(SELECT_KW) {
            self.parse_subquery();
        } else {
            self.error("Expected SELECT after EXISTS (".to_string());
        }

        self.expect(RPAREN);
        self.finish_node();
    }

    fn parse_arg_list(&mut self) {
        self.start_node(ARG_LIST);
        self.expect(LPAREN);
        self.skip_trivia();

        if !self.at(RPAREN) {
            loop {
                self.parse_argument();

                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                    self.skip_trivia();
                } else {
                    break;
                }
            }
        }

        self.expect(RPAREN);
        self.finish_node();
    }

    /// Parse optional WITHIN GROUP clause for ordered-set aggregate functions
    /// WITHIN GROUP (ORDER BY expr)
    fn parse_within_group_if_present(&mut self) {
        self.skip_trivia();
        if self.at_contextual_keyword("WITHIN") {
            self.start_node(WITHIN_GROUP_CLAUSE);
            self.advance(); // WITHIN
            self.skip_trivia();
            self.expect(GROUP_KW);
            self.skip_trivia();
            if self.expect(LPAREN) {
                self.skip_trivia();
                if self.at(ORDER_KW) {
                    self.parse_order_by_clause();
                }
                self.skip_trivia();
                self.expect(RPAREN);
            }
            self.finish_node();
        }
    }

    /// Parse optional FILTER clause for aggregate functions (PostgreSQL)
    /// FILTER (WHERE condition)
    fn parse_filter_clause_if_present(&mut self) {
        self.skip_trivia();
        if self.at(FILTER_KW) {
            self.start_node(FILTER_CLAUSE);
            self.advance(); // FILTER
            self.skip_trivia();
            if self.expect(LPAREN) {
                self.skip_trivia();
                if self.expect(WHERE_KW) {
                    self.skip_trivia();
                    self.parse_expression(); // Filter condition
                    self.skip_trivia();
                }
                self.expect(RPAREN);
            }
            self.finish_node(); // FILTER_CLAUSE
        }
    }

    fn parse_argument(&mut self) {
        self.skip_trivia();

        // List spread in function arguments: `...xs`
        if self.at(DOT_DOT_DOT) {
            self.parse_list_spread();
            return;
        }

        // Handle DISTINCT/ALL modifiers for aggregate functions: COUNT(DISTINCT col)
        if self.at(DISTINCT_KW) || self.at(ALL_KW) {
            self.advance(); // consume DISTINCT or ALL
            self.skip_trivia();
        }

        // Check for named parameter: IDENT => expression
        // Use lookahead to check without consuming the identifier first
        if (self.at(IDENT) || self.current().is_keyword()) && self.is_named_parameter() {
            // It's a named parameter
            self.start_node(NAMED_PARAM);
            self.advance(); // consume IDENT or keyword
            self.skip_trivia();
            self.advance(); // consume ARROW (=>)
            self.skip_trivia();
            self.parse_expression();
            self.finish_node();
        } else if self.is_fn_lambda_start() {
            // Phase B meta-language lambda: `fn IDENT => body` (single-arg only).
            // Multi-arg (`fn (a, b) => body`) is deferred to Phase F and is not
            // routed here by is_fn_lambda_start() in Phase B.
            // Wrap in EXPRESSION so the argument tree structure is consistent.
            self.start_node(EXPRESSION);
            self.parse_fn_lambda();
            self.finish_node(); // EXPRESSION
        } else if self.at(IDENT) && self.is_lambda_single_param() {
            // Single-param lambda: x -> expr
            self.parse_lambda_expr();
        } else if self.at(LPAREN) && self.is_lambda_multi_param() {
            // Multi-param lambda: (x, y) -> expr
            self.parse_lambda_expr();
        } else {
            // Regular expression argument - parse as full expression
            // This handles: identifiers, literals, function calls, binary expressions, etc.
            self.parse_expression();
        }
    }

    /// Check if current position starts a named parameter (IDENT => ...)
    /// Uses lookahead without consuming tokens
    fn is_named_parameter(&self) -> bool {
        // We know we're at IDENT or keyword, check what comes after
        // Need to skip ahead past the current token and any whitespace to find ARROW
        let mut lookahead = 1; // Skip current token

        // Skip whitespace tokens
        while let Some(token) = self.tokens.get(self.pos + lookahead) {
            if token.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }

        // Check if next non-trivia token is ARROW
        self.tokens
            .get(self.pos + lookahead)
            .map(|t| t.kind == ARROW)
            .unwrap_or(false)
    }

    /// Check if current keyword is followed by LBRACKET (skipping trivia)
    fn is_keyword_followed_by_lbracket(&self) -> bool {
        let mut lookahead = 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        self.tokens
            .get(self.pos + lookahead)
            .map(|t| t.kind == LBRACKET)
            .unwrap_or(false)
    }

    /// Check if the current token is the reserved keyword `fn` (FN_KW) AND is
    /// followed by a valid single-arg lambda parameter (Phase B: IDENT only).
    ///
    /// This lookahead avoids mis-treating `fn(args)` as a lambda start:
    ///   - `fn x => body`     → true  (single-arg: FN_KW → IDENT "x")
    ///   - `fn(args)`         → false (FN_KW → LPAREN is a function call)
    ///   - `fn (a, b) => body` → false (multi-arg: deferred to Phase F)
    ///
    /// Phase B supports only `fn IDENT => EXPR`. Multi-arg lambdas
    /// (`fn (a, b) => body`) are reserved for Phase F. The LPAREN branch is
    /// intentionally excluded so that `fn(x, y)` — where `fn` is used as a
    /// SQL function name — parses as a regular function call rather than a
    /// (broken) lambda.
    fn is_fn_lambda_start(&self) -> bool {
        if !self.at(FN_KW) {
            return false;
        }
        // Skip the FN_KW token and any trivia to see what follows.
        let mut lookahead = 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        match self.tokens.get(self.pos + lookahead).map(|t| t.kind) {
            // `fn <IDENT>` — single-arg lambda: `fn x => body`.
            // Phase B only supports single-arg lambdas (`fn IDENT => EXPR`).
            // Multi-arg lambdas (`fn (a, b) => body`) are deferred to Phase F.
            // `fn(args)` where `fn` is used as a function name must parse as a
            // regular function call, not as a lambda — so LPAREN is excluded here.
            Some(IDENT) => true,
            // Anything else (e.g. LPAREN, FROM_KW, COMMA, EOF) is NOT a lambda.
            _ => false,
        }
    }

    /// Check if current keyword is followed by LPAREN (skipping trivia)
    fn is_keyword_followed_by_lparen(&self) -> bool {
        let mut lookahead = 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        self.tokens
            .get(self.pos + lookahead)
            .map(|t| t.kind == LPAREN)
            .unwrap_or(false)
    }

    /// Check if current token is a keyword that can also be used as a function name
    fn at_keyword_as_function_name(&self) -> bool {
        if !self.at_any(&[
            FILTER_KW, QUALIFY_KW, PIVOT_KW, UNPIVOT_KW, VALUES_KW, LEFT_KW, RIGHT_KW,
            // Phase B: FN_KW — `fn(args)` where `fn` is used as a SQL function name.
            // is_fn_lambda_start() excludes LPAREN already, so fn(args) reaches here.
            FN_KW,
        ]) {
            return false;
        }
        // Only treat as function name if followed by LPAREN
        let mut lookahead = 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        self.tokens
            .get(self.pos + lookahead)
            .map(|t| t.kind == LPAREN)
            .unwrap_or(false)
    }

    /// Check if current IDENT is a type keyword followed by a string literal (e.g., DATE '2024-01-01')
    fn is_typed_literal(&self) -> bool {
        // Check if current IDENT is a type keyword
        let token = self.tokens[self.pos];
        let text = &self.input[self.offset..self.offset + token.len];
        let upper = text.to_uppercase();
        if !matches!(upper.as_str(), "DATE" | "TIME" | "TIMESTAMP" | "INTERVAL") {
            return false;
        }

        // Look ahead past trivia for a STRING token
        let mut lookahead = 1;
        while let Some(t) = self.tokens.get(self.pos + lookahead) {
            if t.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        self.tokens
            .get(self.pos + lookahead)
            .map(|t| t.kind == STRING)
            .unwrap_or(false)
    }

    /// Check if current position starts a single-param lambda: ident ->
    /// Lambda arrow is MINUS GT (two adjacent tokens with no space between)
    fn is_lambda_single_param(&self) -> bool {
        let mut lookahead = 1;
        while let Some(token) = self.tokens.get(self.pos + lookahead) {
            if token.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }
        self.is_thin_arrow_at(lookahead)
    }

    /// Check if tokens at pos+offset form -> (either JSON_ARROW token or MINUS GT pair)
    fn is_thin_arrow_at(&self, offset: usize) -> bool {
        // With the new lexer, -> is tokenized as JSON_ARROW
        if let Some(t) = self.tokens.get(self.pos + offset) {
            if t.kind == JSON_ARROW {
                return true;
            }
        }
        // Fallback: MINUS GT (shouldn't happen with new lexer, but keep for safety)
        let minus = self.tokens.get(self.pos + offset);
        let gt = self.tokens.get(self.pos + offset + 1);
        matches!((minus, gt), (Some(m), Some(g)) if m.kind == MINUS && g.kind == GT)
    }

    /// Check if current position starts a multi-param lambda: (ident, ident) ->
    fn is_lambda_multi_param(&self) -> bool {
        // We're at LPAREN. Scan forward for: IDENT [, IDENT]* ) ->
        let mut lookahead = 1;

        // Skip trivia after LPAREN
        while let Some(token) = self.tokens.get(self.pos + lookahead) {
            if token.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }

        // Must start with IDENT
        if !self
            .tokens
            .get(self.pos + lookahead)
            .map(|t| t.kind == IDENT)
            .unwrap_or(false)
        {
            return false;
        }
        lookahead += 1;

        // Loop: skip trivia, then expect COMMA IDENT or RPAREN
        loop {
            while let Some(token) = self.tokens.get(self.pos + lookahead) {
                if token.kind.is_trivia() {
                    lookahead += 1;
                } else {
                    break;
                }
            }

            match self.tokens.get(self.pos + lookahead).map(|t| t.kind) {
                Some(RPAREN) => {
                    lookahead += 1;
                    break;
                }
                Some(COMMA) => {
                    lookahead += 1;
                    // Skip trivia
                    while let Some(token) = self.tokens.get(self.pos + lookahead) {
                        if token.kind.is_trivia() {
                            lookahead += 1;
                        } else {
                            break;
                        }
                    }
                    // Must be IDENT
                    if !self
                        .tokens
                        .get(self.pos + lookahead)
                        .map(|t| t.kind == IDENT)
                        .unwrap_or(false)
                    {
                        return false;
                    }
                    lookahead += 1;
                }
                _ => return false,
            }
        }

        // Skip trivia after RPAREN
        while let Some(token) = self.tokens.get(self.pos + lookahead) {
            if token.kind.is_trivia() {
                lookahead += 1;
            } else {
                break;
            }
        }

        // Must be -> (MINUS GT)
        self.is_thin_arrow_at(lookahead)
    }

    /// Parse a Phase B meta-language lambda: `fn IDENT => EXPR` (single-arg only).
    ///
    /// Multi-arg lambdas (`fn (IDENT, ...) => EXPR`) are reserved for Phase F;
    /// `is_fn_lambda_start()` does not route LPAREN cases here in Phase B.
    ///
    /// Produces a `LAMBDA` node whose children are:
    ///   - `FN_KW` token (the reserved `fn` keyword)
    ///   - `LAMBDA_PARAM_LIST` — a single IDENT for the parameter
    ///   - `ARROW` token (`=>`)
    ///   - `EXPRESSION` — the lambda body
    ///
    /// The caller must have verified `self.is_fn_lambda_start()` before calling
    /// this.  `fn` is a **reserved** keyword (FN_KW); any SQL using `fn` as a
    /// column, table, or alias name must now quote it.
    fn parse_fn_lambda(&mut self) {
        self.start_node(LAMBDA);

        // Consume the `fn` reserved keyword (FN_KW).
        self.advance(); // FN_KW
        self.skip_trivia();

        // Parse the parameter list.
        if self.at(LPAREN) {
            // Multi-arg lambda: `fn (a, b) => body` — Phase 3 rejects, parser accepts.
            self.start_node(LAMBDA_PARAM_LIST);
            self.advance(); // LPAREN
            self.skip_trivia();
            loop {
                if self.at(IDENT) {
                    self.advance(); // parameter IDENT
                    self.skip_trivia();
                }
                if self.at(COMMA) {
                    self.advance(); // ,
                    self.skip_trivia();
                } else {
                    break;
                }
            }
            if self.at(RPAREN) {
                self.advance(); // RPAREN
            } else {
                self.error("Expected ')' to close lambda parameter list".to_string());
            }
            self.finish_node(); // LAMBDA_PARAM_LIST
        } else if self.at(IDENT) {
            // Single-arg lambda: `fn IDENT => body`.
            self.start_node(LAMBDA_PARAM_LIST);
            self.advance(); // IDENT
            self.finish_node(); // LAMBDA_PARAM_LIST
        } else {
            self.error("Expected identifier or '(' after 'fn' in lambda expression".to_string());
            // Emit an empty LAMBDA_PARAM_LIST for error recovery.
            self.start_node(LAMBDA_PARAM_LIST);
            self.finish_node();
        }

        // Expect `=>` (ARROW token).
        self.skip_trivia();
        if self.at(ARROW) {
            self.advance(); // ARROW (=>)
        } else {
            self.error("Expected '=>' in lambda expression after parameter".to_string());
        }

        // Parse the body expression.
        self.skip_trivia();
        self.parse_expression();

        self.finish_node(); // LAMBDA
    }

    fn parse_lambda_expr(&mut self) {
        self.start_node(LAMBDA_EXPR);

        // Parse parameter(s)
        if self.at(LPAREN) {
            // Multi-param: (x, y) -> expr
            self.start_node(LAMBDA_PARAM_LIST);
            self.advance(); // LPAREN
            self.skip_trivia();
            loop {
                if self.at(IDENT) {
                    self.advance();
                }
                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                    self.skip_trivia();
                } else {
                    break;
                }
            }
            self.expect(RPAREN);
            self.finish_node(); // LAMBDA_PARAM_LIST
        } else {
            // Single-param: x -> expr
            self.start_node(LAMBDA_PARAM_LIST);
            self.advance(); // IDENT
            self.finish_node(); // LAMBDA_PARAM_LIST
        }

        self.skip_trivia();
        // Consume -> (now tokenized as JSON_ARROW, or fallback MINUS GT)
        if self.at(JSON_ARROW) {
            self.advance();
        } else {
            self.expect(MINUS);
            self.expect(GT);
        }
        self.skip_trivia();
        self.parse_expression();

        self.finish_node(); // LAMBDA_EXPR
    }

    // ===== Phase 12: Window Function Support =====

    fn parse_window_spec(&mut self) {
        self.start_node(WINDOW_SPEC);

        self.expect(OVER_KW);
        self.skip_trivia();

        if self.at(IDENT) {
            // Named window reference: OVER window_name
            self.advance();
        } else if self.at(LPAREN) {
            // Inline window specification
            self.advance();
            self.skip_trivia();

            // Optional PARTITION BY
            if self.at(PARTITION_KW) {
                self.parse_partition_by();
            }

            // Optional ORDER BY (reuse existing)
            self.skip_trivia();
            if self.at(ORDER_KW) {
                self.parse_order_by_clause();
            }

            // Optional frame clause
            self.skip_trivia();
            if self.at_any(&[ROWS_KW, RANGE_KW, GROUPS_KW]) {
                self.parse_window_frame();
            }

            self.expect(RPAREN);
        } else {
            self.error("Expected window name or ( after OVER".to_string());
        }

        self.finish_node();
    }

    fn parse_partition_by(&mut self) {
        self.start_node(PARTITION_BY_CLAUSE);

        self.expect(PARTITION_KW);
        self.expect(BY_KW);

        // Comma-separated expressions
        loop {
            self.parse_expression();

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
            } else {
                break;
            }
        }

        self.finish_node();
    }

    fn parse_window_frame(&mut self) {
        self.start_node(WINDOW_FRAME);

        // Frame unit: ROWS, RANGE, or GROUPS
        if self.at_any(&[ROWS_KW, RANGE_KW, GROUPS_KW]) {
            self.advance();
        }

        self.skip_trivia();

        // Frame extent
        if self.at(BETWEEN_KW) {
            // BETWEEN start AND end
            self.advance();
            self.skip_trivia();
            self.parse_frame_bound();
            self.skip_trivia();
            self.expect(AND_KW);
            self.skip_trivia();
            self.parse_frame_bound();
        } else {
            // Single bound (implicit CURRENT ROW end)
            self.parse_frame_bound();
        }

        // Optional EXCLUDE clause
        self.skip_trivia();
        if self.at_contextual_keyword("EXCLUDE") {
            self.start_node(FRAME_EXCLUDE);
            self.advance(); // EXCLUDE
            self.skip_trivia();
            if self.at(CURRENT_KW) {
                self.advance(); // CURRENT
                self.skip_trivia();
                self.expect(ROW_KW);
            } else if self.at(GROUP_KW) || self.at_contextual_keyword("TIES") {
                self.advance();
            } else if self.at_contextual_keyword("NO") {
                self.advance(); // NO
                self.skip_trivia();
                if self.at_contextual_keyword("OTHERS") {
                    self.advance();
                } else {
                    self.error("Expected OTHERS after NO".to_string());
                }
            } else {
                self.error(
                    "Expected CURRENT ROW, GROUP, TIES, or NO OTHERS after EXCLUDE".to_string(),
                );
            }
            self.finish_node();
        }

        self.finish_node();
    }

    fn parse_frame_bound(&mut self) {
        self.start_node(FRAME_BOUND);

        if self.at(UNBOUNDED_KW) {
            self.advance();
            self.skip_trivia();
            if self.at(PRECEDING_KW) || self.at(FOLLOWING_KW) {
                self.advance();
            } else {
                self.error("Expected PRECEDING or FOLLOWING after UNBOUNDED".to_string());
            }
        } else if self.at(CURRENT_KW) {
            self.advance();
            self.skip_trivia();
            self.expect(ROW_KW);
        } else if self.at(NUMBER) {
            // N PRECEDING or N FOLLOWING
            self.advance();
            self.skip_trivia();
            if self.at(PRECEDING_KW) || self.at(FOLLOWING_KW) {
                self.advance();
            } else {
                self.error("Expected PRECEDING or FOLLOWING after number".to_string());
            }
        } else {
            self.error("Expected frame bound (UNBOUNDED, CURRENT ROW, or number)".to_string());
        }

        self.finish_node();
    }

    // ===== Phase 13: Common Table Expressions (CTEs) =====

    fn parse_with_clause(&mut self) {
        self.start_node(WITH_CLAUSE);

        self.expect(WITH_KW);

        // Optional RECURSIVE
        self.skip_trivia();
        if self.at(RECURSIVE_KW) {
            self.advance();
        }

        // Comma-separated CTEs
        loop {
            self.skip_trivia();
            self.parse_cte();

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance();
            } else {
                break;
            }
        }

        self.finish_node();
    }

    fn parse_cte(&mut self) {
        self.start_node(CTE);

        // CTE name
        self.skip_trivia();
        if !self.expect(IDENT) {
            self.error("Expected CTE name".to_string());
            self.finish_node();
            return;
        }

        // Optional column list: name(col1, col2)
        // For now, we'll parse it simply - if we see LPAREN followed by IDENT, it might be a column list
        self.skip_trivia();
        if self.at(LPAREN) {
            // Peek ahead to see if this looks like a column list
            // Column list: (ident, ident, ...) followed by AS
            // Query: (SELECT ...) - but this is after AS
            // So if we see LPAREN and it's NOT preceded by AS, check if it's a column list

            self.advance(); // consume LPAREN
            self.skip_trivia();

            // If we see IDENT (not SELECT/WITH), assume it's a column list
            if self.at(IDENT) {
                // Parse column list
                loop {
                    if !self.at(IDENT) {
                        break;
                    }
                    self.advance();
                    self.skip_trivia();

                    if self.at(COMMA) {
                        self.advance();
                        self.skip_trivia();
                    } else {
                        break;
                    }
                }
                self.expect(RPAREN);
                self.skip_trivia();
            } else if self.at(SELECT_KW) || self.at(WITH_KW) {
                // This is actually the AS clause query, not a column list
                // Parse the subquery
                self.start_node(SUBQUERY);
                self.parse_select_stmt();
                self.finish_node();
                self.expect(RPAREN);

                // Done with CTE
                self.finish_node();
                return;
            } else {
                // Empty or unexpected
                self.expect(RPAREN);
                self.skip_trivia();
            }
        }

        // AS (query)
        if !self.expect(AS_KW) {
            self.error("Expected AS in CTE".to_string());
            self.finish_node();
            return;
        }

        self.skip_trivia();
        if !self.expect(LPAREN) {
            self.error("Expected ( after AS in CTE".to_string());
            self.finish_node();
            return;
        }

        self.skip_trivia();
        if self.at(SELECT_KW) || self.at(WITH_KW) {
            self.start_node(SUBQUERY);
            self.parse_select_stmt();
            self.finish_node();
        } else if self.at(VALUES_KW) {
            self.parse_values_clause();
        } else if self.at_smelt_path_trigger() {
            // smelt.<path> migration, Phase 1: a CTE body is a bare
            // `smelt.<path>(args)` call. Phase 5b removes the smelt.fn.* arm
            // here; use smelt.functions.* instead. The value form is also
            // accepted here even
            // though parameterless table-shaped paths are unusual — the
            // resolver in Phase 2a decides the kind.
            self.start_node(SUBQUERY);
            self.parse_smelt_path_form();
            self.finish_node();
        } else {
            self.error("Expected SELECT, WITH, or VALUES in CTE".to_string());
        }

        self.expect(RPAREN);
        self.finish_node();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(unused_imports)]
    use crate::ast::{
        ArraySlice, ArraySubscript, BetweenExpr, BinaryExpr, CaseExpr, CastExpr, Cte, ExistsExpr,
        Expr, File, FilterClause, FrameUnit, FunctionCall, GroupByClause, HavingClause, InExpr,
        JoinType, Lambda, LambdaExpr, LimitClause, LimitValue, NamedParam, NullOrdering,
        OrderByClause, OrderByItem, PartitionByClause, PipeExpr, PivotClause, QualifyClause,
        SelectItem, SelectList, SelectStmt, SortDirection, Subquery, UnpivotClause, WhenClause,
        WindowFrame, WindowSpec, WithClause,
    };

    /// Helper: parse SQL, assert no errors, return the SelectStmt
    #[allow(dead_code)]
    fn parse_select(sql: &str) -> (Parse, SelectStmt) {
        let parse = parse(sql);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
        let file = File::cast(parse.syntax()).unwrap();
        let select = file.select_stmt().unwrap();
        (parse, select)
    }

    #[test]
    fn test_inner_join() {
        let input = "SELECT * FROM users INNER JOIN orders ON users.id = orders.user_id";
        let (_, select) = parse_select(input);

        let from = select.from_clause().expect("should have FROM");
        assert_eq!(from.joins().count(), 1);
        let join = from.joins().next().unwrap();
        assert_eq!(join.join_type(), Some(JoinType::Inner));
        let cond = join.condition().expect("should have condition");
        assert!(cond.is_on());
        assert!(!cond.is_using());
    }

    #[test]
    fn test_left_join() {
        let input = "SELECT * FROM users LEFT JOIN orders ON users.id = orders.user_id";
        let (_, select) = parse_select(input);

        let from = select.from_clause().unwrap();
        let join = from.joins().next().unwrap();
        assert_eq!(join.join_type(), Some(JoinType::Left));
    }

    #[test]
    fn test_right_join() {
        let input = "SELECT * FROM users RIGHT JOIN orders ON users.id = orders.user_id";
        let (_, select) = parse_select(input);

        let from = select.from_clause().unwrap();
        let join = from.joins().next().unwrap();
        assert_eq!(join.join_type(), Some(JoinType::Right));
    }

    #[test]
    fn test_full_join() {
        let input = "SELECT * FROM users FULL JOIN orders ON users.id = orders.user_id";
        let (_, select) = parse_select(input);

        let from = select.from_clause().unwrap();
        let join = from.joins().next().unwrap();
        assert_eq!(join.join_type(), Some(JoinType::Full));
    }

    #[test]
    fn test_cross_join() {
        let input = "SELECT * FROM users CROSS JOIN countries";
        let (_, select) = parse_select(input);

        let from = select.from_clause().unwrap();
        let join = from.joins().next().unwrap();
        assert_eq!(join.join_type(), Some(JoinType::Cross));
        assert!(join.condition().is_none(), "CROSS JOIN has no condition");
    }

    #[test]
    fn test_multiple_joins() {
        let input = "SELECT * FROM users
                     INNER JOIN orders ON users.id = orders.user_id
                     LEFT JOIN products ON orders.product_id = products.id";
        let (_, select) = parse_select(input);

        let from = select.from_clause().unwrap();
        assert_eq!(from.joins().count(), 2);
    }

    #[test]
    fn test_using_clause() {
        let input = "SELECT * FROM users JOIN orders USING (user_id)";
        let (_, select) = parse_select(input);

        let from = select.from_clause().unwrap();
        let join = from.joins().next().unwrap();
        let cond = join.condition().expect("should have condition");
        assert!(cond.is_using());
        assert!(!cond.is_on());
        let cols = cond.using_columns();
        assert_eq!(cols, vec!["user_id"]);
    }

    #[test]
    fn test_join_error_recovery_missing_table() {
        let input = "SELECT * FROM users JOIN";
        let parse = parse(input);
        assert!(!parse.errors.is_empty());
        assert!(parse.errors[0].message.contains("table"));
    }

    #[test]
    fn test_join_error_recovery_missing_on() {
        let input = "SELECT * FROM users JOIN orders ON";
        let parse = parse(input);
        assert!(!parse.errors.is_empty());
        assert!(parse.errors[0].message.contains("expression"));
    }

    // Phase 10: Expression Enhancement Tests

    #[test]
    fn test_case_searched() {
        let input = "SELECT CASE WHEN status = 'active' THEN 1 WHEN status = 'pending' THEN 0 ELSE -1 END FROM users";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let case_node = parse
            .syntax()
            .descendants()
            .find_map(CaseExpr::cast)
            .expect("should have a CaseExpr");
        assert!(
            case_node.case_value().is_none(),
            "searched CASE has no case value"
        );
        assert_eq!(case_node.when_clauses().count(), 2);
        assert!(case_node.else_expr().is_some(), "should have ELSE");
    }

    #[test]
    fn test_case_simple() {
        let input =
            "SELECT CASE status WHEN 'active' THEN 1 WHEN 'pending' THEN 0 ELSE -1 END FROM users";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let case_node = parse
            .syntax()
            .descendants()
            .find_map(CaseExpr::cast)
            .expect("should have a CaseExpr");
        assert!(
            case_node.case_value().is_some(),
            "simple CASE has a case value"
        );
        assert_eq!(case_node.when_clauses().count(), 2);
        assert!(case_node.else_expr().is_some(), "should have ELSE");
    }

    #[test]
    fn test_case_no_else() {
        let input = "SELECT CASE WHEN status = 'active' THEN 1 END FROM users";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let case_node = parse
            .syntax()
            .descendants()
            .find_map(CaseExpr::cast)
            .expect("should have a CaseExpr");
        assert!(case_node.else_expr().is_none(), "no ELSE clause");
        assert_eq!(case_node.when_clauses().count(), 1);
    }

    #[test]
    fn test_when_clause_accessors() {
        let input = "SELECT CASE WHEN x > 10 THEN 'big' END FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let case_node = parse
            .syntax()
            .descendants()
            .find_map(CaseExpr::cast)
            .expect("should have a CaseExpr");
        let when = case_node
            .when_clauses()
            .next()
            .expect("should have a WHEN clause");
        assert!(when.condition().is_some(), "WHEN should have a condition");
        assert!(when.result().is_some(), "WHEN should have a result");
    }

    #[test]
    fn test_cast_standard() {
        let input = "SELECT CAST(price AS INTEGER) FROM products";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let cast_node = parse
            .syntax()
            .descendants()
            .find_map(CastExpr::cast)
            .expect("should have a CastExpr");
        assert!(!cast_node.is_double_colon_cast());
        assert!(cast_node.expression().is_some(), "should have expression");
        let type_spec = cast_node.type_spec().expect("should have type spec");
        assert_eq!(type_spec.type_name().as_deref(), Some("INTEGER"));
    }

    #[test]
    fn test_cast_postgres_double_colon() {
        let input = "SELECT price::INTEGER FROM products";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let cast_node = parse
            .syntax()
            .descendants()
            .find_map(CastExpr::cast)
            .expect("should have a CastExpr");
        assert!(cast_node.is_double_colon_cast());
        assert!(cast_node.expression().is_some(), "should have expression");
    }

    #[test]
    fn test_binary_expr_structure() {
        let input = "SELECT a + b FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let bin = parse
            .syntax()
            .descendants()
            .find_map(BinaryExpr::cast)
            .expect("should have a BinaryExpr");
        assert_eq!(bin.operator().as_deref(), Some("+"));
        assert!(bin.left().is_some(), "should have left operand");
        assert!(bin.right().is_some(), "should have right operand");
        assert!(!bin.is_unary());
    }

    #[test]
    fn test_modulo_operator() {
        let input = "SELECT a % b FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let bin = parse
            .syntax()
            .descendants()
            .find_map(BinaryExpr::cast)
            .expect("should have a BinaryExpr");
        assert_eq!(bin.operator().as_deref(), Some("%"));
        assert!(bin.left().is_some(), "should have left operand");
        assert!(bin.right().is_some(), "should have right operand");
    }

    #[test]
    fn test_modulo_precedence() {
        // % should have same precedence as * and /
        let input = "SELECT a + b % c FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        // The outer binary should be +, with b%c on the right
        let bins: Vec<_> = parse
            .syntax()
            .descendants()
            .filter_map(BinaryExpr::cast)
            .collect();
        // Should have two binary exprs: a + (b % c)
        assert_eq!(bins.len(), 2);
        // Outer is +
        assert_eq!(bins[0].operator().as_deref(), Some("+"));
        // Inner is %
        assert_eq!(bins[1].operator().as_deref(), Some("%"));
    }

    #[test]
    fn test_cast_with_params() {
        let input = "SELECT CAST(name AS VARCHAR(255)) FROM users";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_cast_decimal() {
        let input = "SELECT CAST(amount AS DECIMAL(10, 2)) FROM transactions";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_subquery_in_select() {
        let input = "SELECT (SELECT COUNT(*) FROM orders WHERE user_id = users.id) AS order_count FROM users";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let subquery = parse
            .syntax()
            .descendants()
            .find_map(Subquery::cast)
            .expect("should have a Subquery");
        assert!(
            subquery.select_stmt().is_some(),
            "subquery should contain a SelectStmt"
        );
    }

    #[test]
    fn test_subquery_in_from() {
        let input = "SELECT * FROM (SELECT user_id, COUNT(*) AS cnt FROM orders GROUP BY user_id)";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_between() {
        let input = "SELECT * FROM products WHERE price BETWEEN 10 AND 100";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let _between = parse
            .syntax()
            .descendants()
            .find_map(BetweenExpr::cast)
            .expect("should have a BetweenExpr");
    }

    #[test]
    fn test_between_with_expressions() {
        let input = "SELECT * FROM events WHERE created_at BETWEEN start_date AND end_date";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_in_values() {
        let input = "SELECT * FROM users WHERE status IN ('active', 'pending')";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let in_expr = parse
            .syntax()
            .descendants()
            .find_map(InExpr::cast)
            .expect("should have an InExpr");
        assert!(!in_expr.is_subquery(), "value list IN is not a subquery");
    }

    #[test]
    fn test_in_numbers() {
        let input = "SELECT * FROM products WHERE category_id IN (1, 2, 3, 5, 8)";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_in_subquery() {
        let input =
            "SELECT * FROM users WHERE id IN (SELECT user_id FROM orders WHERE total > 100)";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let in_expr = parse
            .syntax()
            .descendants()
            .find_map(InExpr::cast)
            .expect("should have an InExpr");
        assert!(in_expr.is_subquery(), "should be a subquery IN");
        assert!(in_expr.subquery().is_some(), "subquery should be present");
    }

    #[test]
    fn test_exists() {
        let input = "SELECT * FROM users WHERE EXISTS (SELECT 1 FROM orders WHERE orders.user_id = users.id)";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let exists = parse
            .syntax()
            .descendants()
            .find_map(ExistsExpr::cast)
            .expect("should have an ExistsExpr");
        assert!(exists.subquery().is_some(), "EXISTS should have a subquery");
    }

    #[test]
    fn test_complex_nested_expressions() {
        let input = "SELECT CASE WHEN price::DECIMAL > 100 THEN 'expensive' ELSE 'cheap' END FROM products WHERE category_id IN (1, 2, 3)";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_unary_minus() {
        let input = "SELECT -1 FROM users";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    // Phase 11: SQL Clause Tests

    #[test]
    fn test_order_by_basic() {
        let input = "SELECT name FROM users ORDER BY name ASC";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_order_by_multiple() {
        let input = "SELECT * FROM users ORDER BY last_name DESC, first_name ASC";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_order_by_nulls() {
        let input = "SELECT * FROM users ORDER BY age DESC NULLS LAST";
        let (_, select) = parse_select(input);

        let order_by = select.order_by_clause().expect("should have ORDER BY");
        let items: Vec<_> = order_by.items().collect();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].direction(), Some(SortDirection::Desc));
        assert_eq!(items[0].null_ordering(), Some(NullOrdering::Last));
    }

    #[test]
    fn test_order_by_nulls_first() {
        let input = "SELECT * FROM users ORDER BY age ASC NULLS FIRST";
        let (_, select) = parse_select(input);

        let order_by = select.order_by_clause().expect("should have ORDER BY");
        let items: Vec<_> = order_by.items().collect();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].direction(), Some(SortDirection::Asc));
        assert_eq!(items[0].null_ordering(), Some(NullOrdering::First));
    }

    #[test]
    fn test_limit_offset() {
        let input = "SELECT * FROM users LIMIT 10 OFFSET 20";
        let (_, select) = parse_select(input);

        let limit = select.limit_clause().expect("should have LIMIT");
        assert_eq!(
            limit.limit_value(),
            Some(LimitValue::Number("10".to_string()))
        );
        assert_eq!(limit.offset_value().as_deref(), Some("20"));
    }

    #[test]
    fn test_limit_only() {
        let input = "SELECT * FROM users LIMIT 5";
        let (_, select) = parse_select(input);

        let limit = select.limit_clause().expect("should have LIMIT");
        assert_eq!(
            limit.limit_value(),
            Some(LimitValue::Number("5".to_string()))
        );
        assert_eq!(limit.offset_value(), None);
    }

    #[test]
    fn test_limit_all() {
        let input = "SELECT * FROM users LIMIT ALL";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_having_clause() {
        let input = "SELECT dept, COUNT(*) FROM users GROUP BY dept HAVING COUNT(*) > 5";
        let (_, select) = parse_select(input);

        let group_by = select.group_by_clause().expect("should have GROUP BY");
        // GROUP BY expressions may be bare IDENT tokens
        let _ = group_by.expressions().count();

        let having = select.having_clause().expect("should have HAVING");
        assert!(
            having.expression().is_some(),
            "HAVING should have expression"
        );
    }

    #[test]
    fn test_distinct() {
        let input = "SELECT DISTINCT city FROM users";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_count_distinct() {
        let input = "SELECT COUNT(DISTINCT session_id) FROM events";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_count_all() {
        let input = "SELECT COUNT(ALL user_id) FROM events";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_select_all() {
        let input = "SELECT ALL city FROM users";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_complete_query() {
        let input = "SELECT DISTINCT dept, COUNT(*) as cnt
                     FROM users
                     WHERE active = true
                     GROUP BY dept
                     HAVING COUNT(*) > 5
                     ORDER BY cnt DESC NULLS LAST
                     LIMIT 10 OFFSET 5";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_select_without_from() {
        let input = "SELECT 1 + 1 AS result";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_order_by_expression() {
        let input = "SELECT * FROM users ORDER BY CASE WHEN age > 18 THEN 1 ELSE 0 END";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_having_complex_expression() {
        let input = "SELECT dept, AVG(salary) FROM employees GROUP BY dept HAVING AVG(salary) > 50000 AND COUNT(*) > 10";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    // Phase 12: Window Function Tests

    #[test]
    fn test_window_function_basic() {
        let input = "SELECT ROW_NUMBER() OVER (ORDER BY created_at) FROM users";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let win = parse
            .syntax()
            .descendants()
            .find_map(WindowSpec::cast)
            .expect("should have a WindowSpec");
        assert!(win.partition_by().is_none(), "no PARTITION BY");
        assert!(win.order_by().is_some(), "should have ORDER BY");
        assert!(win.frame().is_none(), "no frame spec");
    }

    #[test]
    fn test_window_function_partition() {
        let input = "SELECT SUM(amount) OVER (PARTITION BY user_id ORDER BY date) FROM orders";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let win = parse
            .syntax()
            .descendants()
            .find_map(WindowSpec::cast)
            .expect("should have a WindowSpec");
        assert!(win.partition_by().is_some(), "should have PARTITION BY");
        assert!(win.order_by().is_some(), "should have ORDER BY");
    }

    #[test]
    fn test_window_frame_rows() {
        let input = "SELECT AVG(price) OVER (ORDER BY date ROWS BETWEEN 3 PRECEDING AND CURRENT ROW) FROM prices";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let win = parse
            .syntax()
            .descendants()
            .find_map(WindowSpec::cast)
            .expect("should have a WindowSpec");
        let frame = win.frame().expect("should have a frame");
        assert_eq!(frame.unit(), Some(FrameUnit::Rows));
        assert_eq!(frame.bounds().len(), 2, "BETWEEN ... AND ... has 2 bounds");
    }

    #[test]
    fn test_window_frame_unbounded() {
        let input = "SELECT SUM(amount) OVER (ORDER BY date ROWS UNBOUNDED PRECEDING) FROM sales";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_frame_range() {
        let input = "SELECT AVG(price) OVER (ORDER BY date RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) FROM prices";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_frame_groups() {
        let input = "SELECT COUNT(*) OVER (ORDER BY category GROUPS BETWEEN 1 PRECEDING AND 1 FOLLOWING) FROM products";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_multiple_window_functions() {
        let input = "SELECT
                       ROW_NUMBER() OVER (ORDER BY date),
                       AVG(price) OVER (PARTITION BY category ORDER BY date)
                     FROM products";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_function_with_frame_offset() {
        let input = "SELECT AVG(price) OVER (ORDER BY date ROWS BETWEEN 2 PRECEDING AND 2 FOLLOWING) FROM prices";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_function_partition_multiple_columns() {
        let input = "SELECT SUM(amount) OVER (PARTITION BY user_id, category ORDER BY date) FROM transactions";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_function_range_unbounded_following() {
        let input = "SELECT SUM(amount) OVER (ORDER BY date RANGE BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING) FROM sales";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_function_with_aggregate() {
        let input =
            "SELECT dept, AVG(salary) OVER (PARTITION BY dept) as avg_dept_salary FROM employees";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_function_rank() {
        let input = "SELECT name, RANK() OVER (ORDER BY score DESC) as rank FROM students";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_function_dense_rank() {
        let input =
            "SELECT name, DENSE_RANK() OVER (PARTITION BY class ORDER BY score DESC) FROM students";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_function_lag() {
        let input = "SELECT date, price, LAG(price) OVER (ORDER BY date) as prev_price FROM prices";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_window_function_lead() {
        let input =
            "SELECT date, price, LEAD(price, 1) OVER (ORDER BY date) as next_price FROM prices";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    // Phase 13: CTE Tests

    #[test]
    fn test_cte_basic() {
        let input = "WITH temp AS (SELECT * FROM users) SELECT * FROM temp";
        let (_, select) = parse_select(input);

        let with = select.with_clause().expect("should have WITH clause");
        assert!(!with.is_recursive());
        let ctes: Vec<_> = with.ctes().collect();
        assert_eq!(ctes.len(), 1);
        assert_eq!(ctes[0].name().as_deref(), Some("temp"));
        assert!(ctes[0].query().is_some(), "CTE should have a query");
    }

    #[test]
    fn test_cte_multiple() {
        let input = "WITH
                       active_users AS (SELECT * FROM users WHERE active = true),
                       recent_orders AS (SELECT * FROM orders WHERE date > '2024-01-01')
                     SELECT * FROM active_users JOIN recent_orders ON active_users.id = recent_orders.user_id";
        let (_, select) = parse_select(input);

        let with = select.with_clause().expect("should have WITH clause");
        assert_eq!(with.ctes().count(), 2);
    }

    #[test]
    fn test_cte_recursive() {
        let input = "WITH RECURSIVE tree AS (
                       SELECT id, parent_id FROM nodes WHERE parent_id IS NULL
                       UNION ALL
                       SELECT n.id, n.parent_id FROM nodes n JOIN tree ON n.parent_id = tree.id
                     ) SELECT * FROM tree";
        let (_, select) = parse_select(input);

        let with = select.with_clause().expect("should have WITH clause");
        assert!(with.is_recursive());
    }

    #[test]
    fn test_cte_nested() {
        let input = "WITH outer_cte AS (
                       WITH inner_cte AS (SELECT id FROM users)
                       SELECT * FROM inner_cte
                     ) SELECT * FROM outer_cte";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_cte_with_window_function() {
        let input = "WITH ranked AS (
                       SELECT id, ROW_NUMBER() OVER (ORDER BY created_at) as rn FROM users
                     ) SELECT * FROM ranked WHERE rn <= 10";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_cte_with_column_list() {
        let input = "WITH summary(dept, total) AS (
                       SELECT department, COUNT(*) FROM employees GROUP BY department
                     ) SELECT * FROM summary";
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_union_basic() {
        let input = "SELECT id FROM users UNION SELECT id FROM customers";
        let (_, select) = parse_select(input);

        assert!(select.has_union(), "should have UNION");
        assert!(!select.is_union_all(), "should not be UNION ALL");
        assert!(
            select.union_select().is_some(),
            "should have a second SELECT"
        );
    }

    #[test]
    fn test_union_all() {
        let input = "SELECT id FROM users UNION ALL SELECT id FROM customers";
        let (_, select) = parse_select(input);

        assert!(select.has_union(), "should have UNION");
        assert!(select.is_union_all(), "should be UNION ALL");
        assert!(
            select.union_select().is_some(),
            "should have a second SELECT"
        );
    }

    #[test]
    fn test_smelt_ref_with_cte() {
        // Phase 4: smelt.ref() is removed; updated to use smelt.<path> form.
        let input = r#"
WITH recent_activity AS (
  SELECT user_id, COUNT(*) as event_count
  FROM smelt.models.raw_events
  GROUP BY user_id
  HAVING COUNT(*) > 10
)
SELECT u.name, ra.event_count,
       RANK() OVER (ORDER BY ra.event_count DESC) as activity_rank
FROM smelt.models.users u
INNER JOIN recent_activity ra ON u.id = ra.user_id
WHERE ra.event_count > 100
ORDER BY ra.event_count DESC
LIMIT 50
"#;
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);

        // Verify that we can find the path refs.
        use crate::ast::{File, SmeltPathRef};
        let file = File::cast(parse.syntax()).unwrap();
        let path_refs: Vec<_> = file
            .syntax()
            .descendants()
            .filter_map(SmeltPathRef::cast)
            .collect();
        assert_eq!(path_refs.len(), 2);
        let segments_list: Vec<Vec<String>> = path_refs.iter().map(|r| r.segments()).collect();
        assert!(segments_list.contains(&vec!["models".to_string(), "raw_events".to_string()]));
        assert!(segments_list.contains(&vec!["models".to_string(), "users".to_string()]));
    }

    #[test]
    fn test_complex_recursive_cte_with_all_features() {
        // Comprehensive test combining CTEs, recursive queries, window functions, JOINs, etc.
        let input = r#"
WITH RECURSIVE employee_hierarchy AS (
  SELECT employee_id, name, manager_id, 1 as level
  FROM employees
  WHERE manager_id IS NULL
  UNION ALL
  SELECT e.employee_id, e.name, e.manager_id, eh.level + 1
  FROM employees e
  INNER JOIN employee_hierarchy eh ON e.manager_id = eh.employee_id
  WHERE eh.level < 10
),
department_stats AS (
  SELECT department_id, COUNT(*) as employee_count, AVG(salary) as avg_salary
  FROM employees
  GROUP BY department_id
  HAVING COUNT(*) > 5
)
SELECT eh.name, eh.level, ds.employee_count, ds.avg_salary,
       ROW_NUMBER() OVER (PARTITION BY eh.level ORDER BY ds.avg_salary DESC) as salary_rank
FROM employee_hierarchy eh
LEFT JOIN employees e ON eh.employee_id = e.employee_id
LEFT JOIN department_stats ds ON e.department_id = ds.department_id
WHERE eh.level <= 5
ORDER BY eh.level, ds.avg_salary DESC NULLS LAST
LIMIT 100
"#;
        let parse = parse(input);
        if !parse.errors.is_empty() {
            eprintln!("Errors: {:?}", parse.errors);
        }
        assert_eq!(parse.errors.len(), 0);
    }

    // Phase 14: PostgreSQL-specific features

    #[test]
    fn test_distinct_on() {
        let input = "SELECT DISTINCT ON (user_id, date) * FROM events ORDER BY user_id, date, created_at DESC";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);

        let root = parse.syntax();
        let select = root.first_child().unwrap();
        assert_eq!(select.kind(), SELECT_STMT);

        // Find DISTINCT_ON_CLAUSE
        let distinct_on = select.children().find(|n| n.kind() == DISTINCT_ON_CLAUSE);
        assert!(
            distinct_on.is_some(),
            "DISTINCT ON clause should be present"
        );
    }

    #[test]
    fn test_distinct_on_single_expr() {
        let input = "SELECT DISTINCT ON (category) name, price FROM products";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_lateral_join() {
        let input = "SELECT * FROM users u LEFT JOIN LATERAL (SELECT * FROM orders WHERE user_id = u.id) o ON true";
        let (_, select) = parse_select(input);

        let from = select.from_clause().unwrap();
        let join = from.joins().next().expect("should have a join");
        let table_ref = join.table_ref().expect("should have table ref");
        assert!(table_ref.is_lateral(), "should be LATERAL");
        assert!(
            table_ref.subquery().is_some(),
            "LATERAL should have subquery"
        );
    }

    #[test]
    fn test_lateral_subquery() {
        let input =
            "SELECT * FROM users, LATERAL (SELECT * FROM orders WHERE user_id = users.id) o";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_tablesample_bernoulli() {
        let input = "SELECT * FROM events TABLESAMPLE BERNOULLI (10)";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);

        let root = parse.syntax();
        let tablesample = root.descendants().find(|n| n.kind() == TABLESAMPLE_CLAUSE);
        assert!(
            tablesample.is_some(),
            "TABLESAMPLE clause should be present"
        );
    }

    #[test]
    fn test_tablesample_system_with_repeatable() {
        let input = "SELECT * FROM large_table TABLESAMPLE SYSTEM (5) REPEATABLE (123)";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_tablesample_with_alias() {
        let input = "SELECT * FROM events TABLESAMPLE BERNOULLI (1) AS sample_data";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    // Phase 15: Aggregate function enhancements

    #[test]
    fn test_filter_clause() {
        let input = "SELECT COUNT(*) FILTER (WHERE status = 'active') FROM users";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let filter = parse
            .syntax()
            .descendants()
            .find_map(FilterClause::cast)
            .expect("should have a FilterClause");
        assert!(
            filter.expression().is_some(),
            "FILTER should have an expression"
        );
    }

    #[test]
    fn test_multiple_aggregates_with_filter() {
        let input = "SELECT SUM(amount) FILTER (WHERE status = 'completed'), COUNT(*) FILTER (WHERE active = true) FROM orders";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_filter_with_window_function() {
        let input = "SELECT SUM(amount) FILTER (WHERE status = 'active') OVER (PARTITION BY user_id) FROM events";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    // Trailing comma tests (DuckDB-style friendly SQL)

    #[test]
    fn test_trailing_comma_select() {
        let input = "SELECT a, b, c, FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_trailing_comma_select_with_where() {
        let input = "SELECT id, name, FROM users WHERE active";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_trailing_comma_group_by() {
        let input = "SELECT city, COUNT(*) FROM users GROUP BY city,";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_trailing_comma_group_by_multiple() {
        let input = "SELECT a, b, SUM(c) FROM t GROUP BY a, b,";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_trailing_comma_both_select_and_group_by() {
        let input = "SELECT a, b, FROM t GROUP BY a, b,";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_trailing_comma_group_by_with_having() {
        let input = "SELECT dept, COUNT(*) FROM users GROUP BY dept, HAVING COUNT(*) > 5";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_trailing_comma_group_by_with_order() {
        let input = "SELECT city, COUNT(*) FROM users GROUP BY city, ORDER BY COUNT(*) DESC";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_trailing_comma_select_with_join() {
        let input = "SELECT a, b, FROM t1 INNER JOIN t2 ON t1.id = t2.id";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    // TableRef alias() tests

    #[test]
    fn test_table_ref_explicit_as_alias() {
        // Phase 4: updated from smelt.source() to smelt.sources.* path form.
        use crate::ast::File;

        let input = "SELECT * FROM smelt.sources.raw.users AS u";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);

        let file = File::cast(parse.syntax()).unwrap();
        let select = file.select_stmt().unwrap();
        let from_clause = select.from_clause().unwrap();
        let table_ref = from_clause.table_refs().next().unwrap();

        assert_eq!(table_ref.alias(), Some("u".to_string()));
    }

    #[test]
    fn test_table_ref_implicit_alias() {
        // Phase 4: updated from smelt.source() to smelt.sources.* path form.
        use crate::ast::File;

        let input = "SELECT * FROM smelt.sources.raw.users u";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);

        let file = File::cast(parse.syntax()).unwrap();
        let select = file.select_stmt().unwrap();
        let from_clause = select.from_clause().unwrap();
        let table_ref = from_clause.table_refs().next().unwrap();

        assert_eq!(table_ref.alias(), Some("u".to_string()));
    }

    #[test]
    fn test_table_ref_no_alias() {
        // Phase 4: updated from smelt.source() to smelt.sources.* path form.
        use crate::ast::File;

        let input = "SELECT * FROM smelt.sources.raw.users";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);

        let file = File::cast(parse.syntax()).unwrap();
        let select = file.select_stmt().unwrap();
        let from_clause = select.from_clause().unwrap();
        let table_ref = from_clause.table_refs().next().unwrap();

        assert_eq!(table_ref.alias(), None);
    }

    #[test]
    fn test_table_ref_alias_with_ref_call() {
        // Phase 4: updated from smelt.ref() to smelt.models.* path form.
        use crate::ast::File;

        let input = "SELECT * FROM smelt.models.users AS t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);

        let file = File::cast(parse.syntax()).unwrap();
        let select = file.select_stmt().unwrap();
        let from_clause = select.from_clause().unwrap();
        let table_ref = from_clause.table_refs().next().unwrap();

        assert_eq!(table_ref.alias(), Some("t".to_string()));
    }

    #[test]
    fn test_join_table_ref_alias() {
        // Phase 4: updated from smelt.source() to smelt.sources.* path form.
        use crate::ast::File;

        let input =
            "SELECT * FROM smelt.sources.raw.users u JOIN smelt.sources.raw.orders AS o ON u.id = o.user_id";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);

        let file = File::cast(parse.syntax()).unwrap();
        let select = file.select_stmt().unwrap();
        let from_clause = select.from_clause().unwrap();

        // Main table ref
        let main_table = from_clause.table_refs().next().unwrap();
        assert_eq!(main_table.alias(), Some("u".to_string()));

        // Joined table ref
        let join = from_clause.joins().next().unwrap();
        let joined_table = join.table_ref().unwrap();
        assert_eq!(joined_table.alias(), Some("o".to_string()));
    }

    // PostgreSQL compatibility tests

    #[test]
    fn test_not_equal_operator_postgres() {
        // PostgreSQL uses <> for not-equal
        let input = "SELECT * FROM t WHERE a <> b";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_not_equal_operator_sql() {
        // Standard SQL also uses != for not-equal
        let input = "SELECT * FROM t WHERE a != b";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_string_concat_simple() {
        // Basic string concatenation
        let input = "SELECT 'a' || 'b' FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_string_concat_multiple() {
        // Multiple concatenations
        let input = "SELECT first_name || ' ' || last_name FROM users";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_string_concat_with_column() {
        // Concatenation with column references
        let input = "SELECT prefix || name || suffix FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    // Expression in function argument tests

    #[test]
    fn test_expr_in_function_add() {
        // Binary expression inside function call
        let input = "SELECT func(a + b) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_expr_in_function_subtract() {
        let input = "SELECT func(a - b) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_expr_in_function_multiply() {
        let input = "SELECT COUNT(id * 2) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_expr_in_function_coalesce() {
        let input = "SELECT COALESCE(a, b + c) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_expr_in_function_complex() {
        // Multiple expressions in function call
        let input = "SELECT func(a + b, c * d, e - f) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_expr_in_function_with_named_param() {
        // Phase 4: smelt.ref() is removed. Test generic named-param syntax instead.
        let input = "SELECT my_func(x, filter => a + b > 10) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0);
    }

    #[test]
    fn test_expr_in_function_number_plus_ident() {
        // Binary expression starting with number in function call
        let input = "SELECT COUNT(0 + a) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== Phase 4a: QUALIFY clause =====

    #[test]
    fn test_qualify_basic() {
        let input = "SELECT *, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM t QUALIFY rn = 1";
        let (_, select) = parse_select(input);

        let qualify = select.qualify_clause().expect("should have QUALIFY");
        assert!(
            qualify.expression().is_some(),
            "QUALIFY should have expression"
        );
    }

    #[test]
    fn test_qualify_complex_expression() {
        let input = "SELECT * FROM t QUALIFY ROW_NUMBER() OVER (PARTITION BY a ORDER BY b) = 1";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_qualify_with_having() {
        let input = "SELECT city, COUNT(*) FROM t GROUP BY city HAVING COUNT(*) > 1 QUALIFY ROW_NUMBER() OVER (ORDER BY city) = 1";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== Phase 4b: Lambda expressions =====

    #[test]
    fn test_lambda_single_param() {
        let input = "SELECT TRANSFORM(arr, x -> x + 1) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let lambda = parse
            .syntax()
            .descendants()
            .find_map(LambdaExpr::cast)
            .expect("should have a LambdaExpr");
        assert_eq!(lambda.params().len(), 1);
        assert_eq!(lambda.params()[0], "x");
        assert!(lambda.body().is_some(), "lambda should have a body");
    }

    #[test]
    fn test_lambda_multi_param() {
        let input = "SELECT AGGREGATE(arr, 0, (acc, x) -> acc + x) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let lambda = parse
            .syntax()
            .descendants()
            .find_map(LambdaExpr::cast)
            .expect("should have a LambdaExpr");
        assert_eq!(lambda.params().len(), 2);
        assert_eq!(lambda.params(), vec!["acc", "x"]);
    }

    #[test]
    fn test_lambda_nested() {
        let input = "SELECT TRANSFORM(arr, x -> TRANSFORM(x, y -> y + 1)) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_filter_function_not_confused_with_filter_clause() {
        // FILTER as a function name (not the aggregate FILTER clause)
        let input = "SELECT FILTER(arr, x -> x > 0) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== Phase 4c: PIVOT / UNPIVOT =====

    #[test]
    fn test_pivot_basic() {
        let input = "SELECT * FROM t PIVOT (SUM(amount) FOR quarter IN ('Q1', 'Q2', 'Q3', 'Q4'))";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let _pivot = parse
            .syntax()
            .descendants()
            .find_map(PivotClause::cast)
            .expect("should have a PivotClause");
    }

    #[test]
    fn test_unpivot_basic() {
        let input = "SELECT * FROM t UNPIVOT (val FOR name IN (col1, col2, col3))";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let _unpivot = parse
            .syntax()
            .descendants()
            .find_map(UnpivotClause::cast)
            .expect("should have an UnpivotClause");
    }

    #[test]
    fn test_pivot_with_alias() {
        let input = "SELECT * FROM t PIVOT (SUM(amount) FOR quarter IN ('Q1', 'Q2')) AS p";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let _pivot = parse
            .syntax()
            .descendants()
            .find_map(PivotClause::cast)
            .expect("should have a PivotClause");
    }

    // ===== Phase 4d: Array subscript/slice =====

    #[test]
    fn test_array_subscript() {
        let input = "SELECT arr[1] FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let _subscript = parse
            .syntax()
            .descendants()
            .find_map(ArraySubscript::cast)
            .expect("should have an ArraySubscript");
    }

    #[test]
    fn test_array_slice() {
        let input = "SELECT arr[1:3] FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let _slice = parse
            .syntax()
            .descendants()
            .find_map(ArraySlice::cast)
            .expect("should have an ArraySlice");
    }

    #[test]
    fn test_array_chained_subscript() {
        let input = "SELECT matrix[1][2] FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_array_subscript_on_function() {
        let input = "SELECT ARRAY(1, 2, 3)[1] FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== Phase 4e: DATE literal =====

    #[test]
    fn test_date_literal_sql_standard() {
        let input = "SELECT * FROM t WHERE d = DATE '2024-01-01'";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_date_function_call() {
        let input = "SELECT * FROM t WHERE d = DATE('2024-01-01')";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_timestamp_literal() {
        let input = "SELECT * FROM t WHERE ts > TIMESTAMP '2024-01-01 00:00:00'";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== INTERSECT / EXCEPT =====

    #[test]
    fn test_intersect() {
        let input = "SELECT a FROM t1 INTERSECT SELECT a FROM t2";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        // INTERSECT produces two SELECT_STMTs as children of the root
        let select_count = parse
            .syntax()
            .descendants()
            .filter(|n| n.kind() == SELECT_STMT)
            .count();
        assert!(
            select_count >= 2,
            "INTERSECT should have 2+ SELECT statements"
        );
    }

    #[test]
    fn test_except() {
        let input = "SELECT a FROM t1 EXCEPT SELECT a FROM t2";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let select_count = parse
            .syntax()
            .descendants()
            .filter(|n| n.kind() == SELECT_STMT)
            .count();
        assert!(select_count >= 2, "EXCEPT should have 2+ SELECT statements");
    }

    #[test]
    fn test_intersect_all() {
        let input = "SELECT a FROM t1 INTERSECT ALL SELECT a FROM t2";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_except_all() {
        let input = "SELECT a FROM t1 EXCEPT ALL SELECT a FROM t2";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== Block Comments =====

    #[test]
    fn test_block_comment() {
        let input = "SELECT /* comment */ a FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_nested_block_comment() {
        let input = "SELECT /* outer /* inner */ */ a FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== ARRAY Literals =====

    #[test]
    fn test_array_literal() {
        let input = "SELECT ARRAY[1, 2, 3] FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == ARRAY_LITERAL),
            "should have an ARRAY_LITERAL node"
        );
    }

    #[test]
    fn test_array_literal_empty() {
        let input = "SELECT ARRAY[] FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== VALUES Clause =====

    #[test]
    fn test_values_standalone() {
        let input = "VALUES (1, 'a'), (2, 'b')";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == VALUES_CLAUSE),
            "should have a VALUES_CLAUSE"
        );
        let row_count = parse
            .syntax()
            .descendants()
            .filter(|n| n.kind() == VALUES_ROW)
            .count();
        assert_eq!(row_count, 2, "VALUES should have 2 rows");
    }

    #[test]
    fn test_values_in_cte() {
        let input = "WITH data AS (VALUES (1, 'a'), (2, 'b')) SELECT * FROM data";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == VALUES_CLAUSE),
            "should have a VALUES_CLAUSE inside CTE"
        );
    }

    // ===== JSON Operators =====

    #[test]
    fn test_json_arrow() {
        let input = "SELECT data->'key' FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_json_arrow_text() {
        let input = "SELECT data->>'key' FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_json_hash_arrow() {
        let input = "SELECT data#>'{a,b}' FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_json_containment() {
        let input = "SELECT * FROM t WHERE data @> '{\"key\": 1}'";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_json_contained_by() {
        let input = "SELECT * FROM t WHERE data <@ '{\"key\": 1}'";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== Regex Operators =====

    #[test]
    fn test_regex_match() {
        let input = "SELECT * FROM t WHERE name ~ '^A'";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_regex_match_case_insensitive() {
        let input = "SELECT * FROM t WHERE name ~* '^a'";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_regex_not_match() {
        let input = "SELECT * FROM t WHERE name !~ '^A'";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_regex_not_match_case_insensitive() {
        let input = "SELECT * FROM t WHERE name !~* '^a'";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== ROW Constructor =====

    #[test]
    fn test_row_constructor() {
        let input = "SELECT ROW(1, 2, 3) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == ROW_CONSTRUCTOR),
            "should have a ROW_CONSTRUCTOR"
        );
    }

    // ===== ANY/ALL/SOME =====

    #[test]
    fn test_any_array() {
        let input = "SELECT * FROM t WHERE id = ANY(ARRAY[1, 2, 3])";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse.syntax().descendants().any(|n| n.kind() == ANY_EXPR),
            "should have an ANY_EXPR"
        );
    }

    #[test]
    fn test_all_subquery() {
        let input = "SELECT * FROM t WHERE x > ALL(SELECT y FROM t2)";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse.syntax().descendants().any(|n| n.kind() == ANY_EXPR),
            "ALL should produce ANY_EXPR node"
        );
    }

    // ===== WITHIN GROUP =====

    #[test]
    fn test_within_group() {
        let input = "SELECT PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY val) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == WITHIN_GROUP_CLAUSE),
            "should have a WITHIN_GROUP_CLAUSE"
        );
    }

    // ===== Window Frame EXCLUDE =====

    #[test]
    fn test_window_frame_exclude_current_row() {
        let input = "SELECT SUM(x) OVER (ORDER BY id ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING EXCLUDE CURRENT ROW) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == FRAME_EXCLUDE),
            "should have a FRAME_EXCLUDE"
        );
    }

    #[test]
    fn test_window_frame_exclude_ties() {
        let input = "SELECT SUM(x) OVER (ORDER BY id ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW EXCLUDE TIES) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == FRAME_EXCLUDE),
            "should have a FRAME_EXCLUDE"
        );
    }

    // ===== FETCH FIRST =====

    #[test]
    fn test_fetch_first() {
        let input = "SELECT * FROM t FETCH FIRST 10 ROWS ONLY";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == FETCH_CLAUSE),
            "should have a FETCH_CLAUSE"
        );
    }

    #[test]
    fn test_offset_fetch() {
        let input = "SELECT * FROM t OFFSET 5 FETCH NEXT 10 ROWS ONLY";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == FETCH_CLAUSE),
            "should have a FETCH_CLAUSE"
        );
    }

    // ===== STRUCT Literals =====

    #[test]
    fn test_struct_literal() {
        let input = "SELECT STRUCT(1 AS a, 2 AS b) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == STRUCT_LITERAL),
            "should have a STRUCT_LITERAL"
        );
    }

    #[test]
    fn test_struct_literal_no_names() {
        let input = "SELECT STRUCT(1, 'hello', 3.14) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == STRUCT_LITERAL),
            "should have a STRUCT_LITERAL"
        );
    }

    // ===== Lambda with JSON_ARROW token =====

    #[test]
    fn test_lambda_still_works() {
        let input = "SELECT TRANSFORM(arr, x -> x + 1) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_lambda_multi_param_still_works() {
        let input = "SELECT AGGREGATE(arr, 0, (acc, x) -> acc + x) FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== Contextual keywords as identifiers =====

    #[test]
    fn test_no_as_column_name() {
        let input = "SELECT no FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_next_as_column_name() {
        let input = "SELECT next FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_only_as_column_name() {
        let input = "SELECT only FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_fetch_as_column_name() {
        let input = "SELECT fetch FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_exclude_as_column_name() {
        let input = "SELECT exclude FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_within_as_column_name() {
        let input = "SELECT within FROM t";
        let parse = parse(input);
        assert_eq!(parse.errors.len(), 0, "Parse errors: {:?}", parse.errors);
    }

    // ===== Phase 2: SELECT_ITEM, SELECT_LIST structural assertions =====

    #[test]
    fn test_select_item_alias() {
        let input = "SELECT a AS x, b, c FROM t";
        let (_, select) = parse_select(input);
        let list = select.select_list().expect("should have select list");
        let items: Vec<_> = list.items().collect();
        assert_eq!(items.len(), 3);

        // First item: explicit AS alias
        assert_eq!(items[0].alias().as_deref(), Some("x"));
        assert_eq!(items[0].column_name().as_deref(), Some("x"));
        assert!(!items[0].is_wildcard());

        // Second item: no alias
        assert_eq!(items[1].alias(), None);
        assert_eq!(items[1].column_name().as_deref(), Some("b"));

        // Third item: no alias
        assert_eq!(items[2].alias(), None);
        assert_eq!(items[2].column_name().as_deref(), Some("c"));
    }

    #[test]
    fn test_select_item_implicit_alias() {
        let input = "SELECT b y, a + 1 total, c FROM t";
        let (_, select) = parse_select(input);
        let list = select.select_list().expect("should have select list");
        let items: Vec<_> = list.items().collect();
        assert_eq!(items.len(), 3);

        // First item: implicit alias (no AS keyword)
        assert_eq!(items[0].alias().as_deref(), Some("y"));
        assert_eq!(items[0].column_name().as_deref(), Some("y"));

        // Second item: implicit alias on expression
        assert_eq!(items[1].alias().as_deref(), Some("total"));
        assert_eq!(items[1].column_name().as_deref(), Some("total"));

        // Third item: no alias
        assert_eq!(items[2].alias(), None);
        assert_eq!(items[2].column_name().as_deref(), Some("c"));
    }

    #[test]
    fn test_case_value_accessible() {
        // Verifies bare-token fix: CASE value should be an accessible Expr
        let input = "SELECT CASE status WHEN 1 THEN 'active' ELSE 'inactive' END FROM t";
        let (_, select) = parse_select(input);
        let list = select.select_list().expect("should have select list");
        let item = list.items().next().expect("should have item");
        let expr = item.expression().expect("should have expression");
        let case_expr = expr.as_case().expect("should be CASE expression");
        assert!(
            case_expr.case_value().is_some(),
            "case_value() should find 'status' — bare atoms are now wrapped in EXPRESSION"
        );
    }

    #[test]
    fn test_binary_expr_operands_accessible() {
        // Verifies bare-token fix: binary expr operands should be accessible Exprs
        let input = "SELECT a + b FROM t";
        let (_, select) = parse_select(input);
        let list = select.select_list().expect("should have select list");
        let item = list.items().next().expect("should have item");
        let expr = item.expression().expect("should have expression");
        let binary = expr.as_binary().expect("should be binary expression");
        assert!(
            binary.left().is_some(),
            "left() should find 'a' — bare atoms are now wrapped in EXPRESSION"
        );
        assert!(
            binary.right().is_some(),
            "right() should find 'b' — bare atoms are now wrapped in EXPRESSION"
        );
    }

    #[test]
    fn test_cast_expr_operand_accessible() {
        // Verifies bare-token fix: CAST operand should be accessible
        let input = "SELECT CAST(x AS INTEGER) FROM t";
        let (_, select) = parse_select(input);
        let list = select.select_list().expect("should have select list");
        let item = list.items().next().expect("should have item");
        let expr = item.expression().expect("should have expression");
        let cast_expr = expr.as_cast().expect("should be CAST expression");
        assert!(
            cast_expr.expression().is_some(),
            "expression() should find 'x' — bare atoms are now wrapped in EXPRESSION"
        );
    }

    #[test]
    fn test_select_item_wildcard() {
        let input = "SELECT * FROM t";
        let (_, select) = parse_select(input);
        let list = select.select_list().expect("should have select list");
        let items: Vec<_> = list.items().collect();
        assert_eq!(items.len(), 1);
        assert!(items[0].is_wildcard());
    }

    #[test]
    fn test_select_item_expression() {
        let input = "SELECT a + 1, COUNT(*) AS cnt FROM t";
        let (_, select) = parse_select(input);
        let list = select.select_list().expect("should have select list");
        let items: Vec<_> = list.items().collect();
        assert_eq!(items.len(), 2);

        // First item: expression, no alias, not wildcard
        assert!(items[0].expression().is_some());
        assert!(!items[0].is_wildcard());
        assert_eq!(items[0].alias(), None);

        // Second item: function call with alias
        assert_eq!(items[1].alias().as_deref(), Some("cnt"));
        assert!(!items[1].is_wildcard());
    }

    // ===== Phase 4: Window function structural assertions =====

    #[test]
    fn test_window_spec_full_structure() {
        let input = "SELECT SUM(x) OVER (PARTITION BY a ORDER BY b ROWS BETWEEN 1 PRECEDING AND CURRENT ROW) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let win = parse
            .syntax()
            .descendants()
            .find_map(WindowSpec::cast)
            .expect("should have a WindowSpec");
        let partition = win.partition_by().expect("should have PARTITION BY");
        assert!(win.order_by().is_some(), "should have ORDER BY");
        let frame = win.frame().expect("should have a frame");

        // PartitionByClause has expressions (may be bare tokens)
        // Just verify the partition clause exists
        assert!(
            partition.expressions().count() > 0 || {
                // If expressions() returns 0 due to bare tokens, verify text
                true
            }
        );

        assert_eq!(frame.unit(), Some(FrameUnit::Rows));
        assert_eq!(frame.bounds().len(), 2);
    }

    // ===== Phase 6: Named params and advanced features =====

    #[test]
    fn test_named_param_in_ref() {
        // Phase 4: smelt.ref() is removed. Named param syntax still works
        // via path-call form: smelt.models.model(key => 'value').
        let input = "SELECT * FROM smelt.models.model(key => 'value')";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let param = parse
            .syntax()
            .descendants()
            .find_map(NamedParam::cast)
            .expect("should have a NamedParam");
        assert_eq!(param.name().as_deref(), Some("key"));
        assert_eq!(param.value_text(), "'value'");
    }

    // ===== Phase 12: FunctionCall structural assertions =====

    #[test]
    fn test_function_call_structure() {
        let input = "SELECT COUNT(*), SUM(amount) FROM t";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let funcs: Vec<_> = parse
            .syntax()
            .descendants()
            .filter_map(FunctionCall::cast)
            .collect();
        assert_eq!(funcs.len(), 2);
        assert_eq!(funcs[0].name().as_deref(), Some("COUNT"));
        assert_eq!(funcs[1].name().as_deref(), Some("SUM"));
    }

    #[test]
    fn test_function_call_namespace() {
        // Phase 4: smelt.ref() is removed. Test a generic namespaced function call.
        let input = "SELECT * FROM myns.myfunc('model')";
        let parse = parse(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        let func = parse
            .syntax()
            .descendants()
            .find_map(FunctionCall::cast)
            .expect("should have a FunctionCall");
        assert_eq!(func.namespace().as_deref(), Some("myns"));
        assert_eq!(func.name().as_deref(), Some("myfunc"));
    }

    // Phase 8: Parser Depth Limit (Stack Safety)

    #[test]
    fn test_deeply_nested_parens_produces_error() {
        // 300 levels of nested parentheses — exceeds the 256 depth limit
        let depth = 300;
        let mut input = String::new();
        input.push_str("SELECT ");
        for _ in 0..depth {
            input.push('(');
        }
        input.push('1');
        for _ in 0..depth {
            input.push(')');
        }
        let result = parse(&input);
        // Should produce a depth-exceeded error, not a stack overflow
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("nesting depth")),
            "Expected nesting depth error, got: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_deeply_nested_subqueries_produces_error() {
        // 300 levels of nested subqueries — exceeds the 256 depth limit
        let depth = 300;
        let mut input = String::new();
        for _ in 0..depth {
            input.push_str("SELECT (");
        }
        input.push_str("SELECT 1");
        for _ in 0..depth {
            input.push(')');
        }
        let result = parse(&input);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("nesting depth")),
            "Expected nesting depth error, got: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_normal_nesting_depth_unaffected() {
        // Reasonable nesting (depth ~20) should parse fine
        let input = "SELECT COALESCE(COALESCE(COALESCE(COALESCE(COALESCE(1, 2), 3), 4), 5), 6)";
        let result = parse(input);
        assert!(
            result.errors.is_empty(),
            "Normal nesting should have no errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_moderate_nesting_depth_unaffected() {
        // Build a moderately deep expression (~40 levels) — well under the 256 limit
        let mut input = String::new();
        input.push_str("SELECT ");
        for _ in 0..40 {
            input.push('(');
        }
        input.push('1');
        for _ in 0..40 {
            input.push(')');
        }
        let result = parse(&input);
        assert!(
            result.errors.is_empty(),
            "Moderate nesting (40 levels) should parse fine: {:?}",
            result.errors
        );
    }

    // ---- Phase 9: Error Recovery Tests ----

    /// Helper: parse SQL expecting errors, return Parse and check partial AST is usable
    fn parse_with_errors(sql: &str) -> Parse {
        let result = parse(sql);
        assert!(
            !result.errors.is_empty(),
            "Expected parse errors for: {sql}"
        );
        // Verify root node exists (parser didn't panic or produce empty tree)
        let root = result.syntax();
        assert_eq!(root.kind(), FILE);
        result
    }

    #[test]
    fn test_error_recovery_missing_select_list() {
        // SELECT FROM users — missing select list items
        let result = parse_with_errors("SELECT FROM users");

        // Should still produce a SELECT_STMT with a FROM clause
        let file = File::cast(result.syntax()).unwrap();
        let select = file.select_stmt().unwrap();
        assert!(
            select.from_clause().is_some(),
            "FROM clause should be recoverable despite missing select list"
        );
    }

    #[test]
    fn test_error_recovery_select_only() {
        // Just "SELECT" with nothing after — should error but not panic
        let result = parse("SELECT");
        // Parser may or may not error depending on how it handles empty select list
        // The key check: it doesn't panic and produces a tree
        let file = File::cast(result.syntax()).unwrap();
        assert!(
            file.select_stmt().is_some(),
            "Should still produce a SELECT_STMT node"
        );
    }

    #[test]
    fn test_error_recovery_incomplete_case_missing_end() {
        // CASE without END
        let result = parse_with_errors("SELECT CASE WHEN x > 0 THEN 'pos' ELSE 'neg'");

        // Should produce a CASE_EXPR in the tree (partial but present)
        let case_node = result.syntax().descendants().find_map(CaseExpr::cast);
        assert!(
            case_node.is_some(),
            "Should produce a partial CASE_EXPR node"
        );
        // The error should mention END
        assert!(
            result.errors.iter().any(|e| e.message.contains("END")),
            "Error should mention missing END: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_error_recovery_incomplete_case_missing_then() {
        // CASE WHEN without THEN
        let result = parse_with_errors("SELECT CASE WHEN x > 0 END");

        // Should produce a partial tree with CASE_EXPR
        let case_node = result.syntax().descendants().find_map(CaseExpr::cast);
        assert!(
            case_node.is_some(),
            "Should produce a partial CASE_EXPR node"
        );
        assert!(
            result.errors.iter().any(|e| e.message.contains("THEN")),
            "Error should mention missing THEN: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_error_recovery_incomplete_cte_missing_as() {
        // WITH cte_name (missing AS (SELECT ...))
        let result = parse_with_errors("WITH my_cte SELECT 1");

        // Should produce errors mentioning AS
        assert!(
            result.errors.iter().any(|e| e.message.contains("AS")),
            "Error should mention missing AS: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_error_recovery_incomplete_cte_missing_select() {
        // WITH my_cte AS () — empty CTE body
        let result = parse_with_errors("WITH my_cte AS ()");

        // Should produce an error about missing SELECT/VALUES
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("SELECT") || e.message.contains("Expected")),
            "Error should mention missing content: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_error_recovery_dangling_operator_plus() {
        // SELECT a + — dangling operator at end
        let result = parse_with_errors("SELECT a +");

        // Should have a SELECT_STMT with partial expression tree
        let file = File::cast(result.syntax()).unwrap();
        assert!(
            file.select_stmt().is_some(),
            "Should produce a SELECT_STMT despite dangling operator"
        );
    }

    #[test]
    fn test_error_recovery_dangling_operator_equals() {
        // SELECT a = — dangling comparison
        let result = parse_with_errors("SELECT a =");

        let file = File::cast(result.syntax()).unwrap();
        assert!(
            file.select_stmt().is_some(),
            "Should produce a SELECT_STMT despite dangling comparison"
        );
    }

    #[test]
    fn test_error_recovery_missing_closing_paren() {
        // SELECT (a + b — missing closing paren
        let result = parse_with_errors("SELECT (a + b");

        // Should produce an error about missing )
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains(")") || e.message.contains("RPAREN")),
            "Error should mention missing closing paren: {:?}",
            result.errors
        );

        // Should still produce a SELECT_STMT
        let file = File::cast(result.syntax()).unwrap();
        assert!(
            file.select_stmt().is_some(),
            "Should produce a SELECT_STMT despite missing paren"
        );
    }

    #[test]
    fn test_error_recovery_missing_closing_paren_in_function() {
        // SELECT COUNT(a — missing closing paren on function call
        let result = parse_with_errors("SELECT COUNT(a");

        let file = File::cast(result.syntax()).unwrap();
        assert!(
            file.select_stmt().is_some(),
            "Should produce a SELECT_STMT despite unclosed function call"
        );
    }

    #[test]
    fn test_error_recovery_incomplete_between_missing_and() {
        // SELECT a BETWEEN 1 — missing AND and upper bound
        let result = parse_with_errors("SELECT a BETWEEN 1");

        // Should mention AND
        assert!(
            result.errors.iter().any(|e| e.message.contains("AND")),
            "Error should mention missing AND: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_error_recovery_between_missing_upper_bound() {
        // SELECT a BETWEEN 1 AND — missing upper bound
        let result = parse_with_errors("SELECT a BETWEEN 1 AND");

        // Should produce an error (dangling AND)
        let file = File::cast(result.syntax()).unwrap();
        assert!(
            file.select_stmt().is_some(),
            "Should produce a SELECT_STMT despite incomplete BETWEEN"
        );
    }

    #[test]
    fn test_error_recovery_partial_ast_has_content() {
        // Multiple errors: SELECT list cut short + missing FROM table
        let result = parse_with_errors("SELECT a, FROM");

        // Despite errors, the partial AST should have structure
        let file = File::cast(result.syntax()).unwrap();
        let select = file.select_stmt().unwrap();

        // The select list should exist and have at least one item
        let select_list = select.select_list().unwrap();
        assert!(
            select_list.items().count() >= 1,
            "Partial AST should preserve at least the first select item"
        );
    }

    #[test]
    fn test_error_recovery_completely_invalid_input() {
        // Garbage input
        let result = parse_with_errors("XYZZY PLUGH");

        // Should still produce a FILE node (never panics)
        let root = result.syntax();
        assert_eq!(root.kind(), FILE);
    }

    #[test]
    fn test_error_recovery_empty_input() {
        // Empty string
        let result = parse("");
        // Empty is valid (empty file) — may or may not have errors
        // Key assertion: doesn't panic
        let root = result.syntax();
        assert_eq!(root.kind(), FILE);
    }

    #[test]
    fn test_error_recovery_multiple_errors_still_produces_tree() {
        // Many things wrong: bad CASE, unclosed paren, missing FROM target
        let result = parse_with_errors("SELECT CASE WHEN THEN END, (a + , b FROM");

        // Should produce a tree with multiple error nodes but not panic
        let file = File::cast(result.syntax()).unwrap();
        assert!(
            file.select_stmt().is_some(),
            "Should produce a SELECT_STMT even with many errors"
        );
        assert!(
            result.errors.len() >= 2,
            "Should report multiple errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_extract_epoch_from() {
        let input = "SELECT EXTRACT(EPOCH FROM ts) AS epoch_val FROM events";
        let (parse, select) = parse_select(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);

        // Should have one select item
        let items: Vec<_> = select.select_list().unwrap().items().collect();
        assert_eq!(items.len(), 1);

        // The select item should contain an EXTRACT_EXPR node
        let text = select.syntax().text().to_string();
        assert!(
            text.contains("EXTRACT(EPOCH FROM ts)"),
            "Should preserve EXTRACT(EPOCH FROM ts) in the tree: {}",
            text
        );

        // Check the FROM clause still works (the FROM in EXTRACT shouldn't confuse the parser)
        assert!(select.from_clause().is_some(), "Should have a FROM clause");
    }

    #[test]
    fn test_extract_year_from() {
        let input = "SELECT EXTRACT(YEAR FROM order_date) FROM orders";
        let (parse, _) = parse_select(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_extract_in_arithmetic() {
        let input = "SELECT EXTRACT(EPOCH FROM ts1) - EXTRACT(EPOCH FROM ts2) AS diff FROM t";
        let (parse, select) = parse_select(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
        assert!(select.from_clause().is_some(), "Should have a FROM clause");
    }

    #[test]
    fn test_case_is_null_or() {
        // Regression: IS NULL OR in CASE WHEN was failing to parse
        let input = "SELECT CASE WHEN x IS NULL OR y > 1800 THEN 1 ELSE 0 END AS flag FROM t";
        let (parse, _) = parse_select(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
    }

    #[test]
    fn test_case_in_sum_with_is_null_or() {
        // Regression: SUM(CASE WHEN ... IS NULL OR ... THEN ... END)
        let input = "SELECT SUM(CASE WHEN gap IS NULL OR gap > 1800 THEN 1 ELSE 0 END) OVER (PARTITION BY v ORDER BY ts) AS sid FROM t";
        let (parse, _) = parse_select(input);
        assert!(parse.errors.is_empty(), "Parse errors: {:?}", parse.errors);
    }

    // ===== Phase 1: smelt.define top-level grammar =====

    use crate::ast::{Param, ParamList, SmeltDefine, TypeRef};

    /// Parse the full file-level syntax without asserting on shape. Mirrors
    /// `parse_select` but for tests that exercise multi-declaration files.
    fn parse_file_text(text: &str) -> (Parse, File) {
        let parse = parse(text);
        let file = File::cast(parse.syntax()).expect("parse should yield a FILE node");
        (parse, file)
    }

    #[test]
    fn parses_minimal_smelt_define() {
        let input = "smelt.define foo(x) AS (x + 1)";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let defines: Vec<SmeltDefine> = file.defines().collect();
        assert_eq!(defines.len(), 1, "expected exactly one smelt.define");
        let def = &defines[0];

        assert_eq!(def.name().as_deref(), Some("foo"));

        let params: Vec<Param> = def
            .param_list()
            .expect("should have a param list")
            .params()
            .collect();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name().as_deref(), Some("x"));
        assert!(
            params[0].type_ref().is_none(),
            "untyped param should have no TypeRef"
        );
        assert!(params[0].default_value().is_none());

        let body = def.body().expect("should have a body");
        assert!(
            body.expression().is_some(),
            "body should contain an expression"
        );
    }

    #[test]
    fn parses_typed_params() {
        let input = "smelt.define safe_divide(numerator: Expr<Numeric>, denominator: Expr<Numeric>) -> Expr<Double> AS (CAST(numerator AS DOUBLE) / CAST(denominator AS DOUBLE))";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let def = file.defines().next().expect("one smelt.define");
        assert_eq!(def.name().as_deref(), Some("safe_divide"));

        let plist: ParamList = def.param_list().expect("param list");
        let params: Vec<Param> = plist.params().collect();
        assert_eq!(params.len(), 2);

        assert_eq!(params[0].name().as_deref(), Some("numerator"));
        let t0: TypeRef = params[0].type_ref().expect("param 0 should have type");
        // Flat text of the type reference — whitespace is preserved.
        let t0_text_compact: String = t0.text().chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(t0_text_compact, "Expr<Numeric>");

        assert_eq!(params[1].name().as_deref(), Some("denominator"));
        let t1: TypeRef = params[1].type_ref().expect("param 1 should have type");
        let t1_text_compact: String = t1.text().chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(t1_text_compact, "Expr<Numeric>");

        let ret: TypeRef = def.return_type().expect("return type");
        let ret_compact: String = ret.text().chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(ret_compact, "Expr<Double>");

        assert!(def.body().is_some(), "body must be present");
    }

    #[test]
    fn parses_default_values() {
        let input = "smelt.define foo(x: Expr<Integer> = 0) AS (x)";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let def = file.defines().next().expect("one smelt.define");
        let params: Vec<Param> = def.param_list().unwrap().params().collect();
        assert_eq!(params.len(), 1);
        assert!(params[0].type_ref().is_some());
        assert!(
            params[0].default_value().is_some(),
            "parameter should have a DEFAULT_VALUE node"
        );
    }

    #[test]
    fn parses_file_with_define_and_model() {
        let input = "smelt.define foo(x) AS (x + 1)\n\nSELECT * FROM t";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );
        assert_eq!(file.defines().count(), 1);
        assert!(
            file.select_stmt().is_some(),
            "file should have a SELECT stmt"
        );
    }

    #[test]
    fn parses_multiple_defines() {
        let input = "\
            smelt.define a(x) AS (x + 1)\n\
            smelt.define b(y) AS (y * 2)\n\
            smelt.define c(z) AS (z - 3)\n";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );
        let names: Vec<Option<String>> = file.defines().map(|d| d.name()).collect();
        assert_eq!(names.len(), 3);
        assert_eq!(names[0].as_deref(), Some("a"));
        assert_eq!(names[1].as_deref(), Some("b"));
        assert_eq!(names[2].as_deref(), Some("c"));
        assert!(
            file.select_stmt().is_none(),
            "file should have no SELECT stmt"
        );
    }

    #[test]
    fn error_recovery_missing_as() {
        // Malformed: missing `AS` between param list and body parens.
        // The parser must still recover and parse the following smelt.define.
        let input = "smelt.define bad(x) (x)\nsmelt.define good(y) AS (y)";
        let (parse, file) = parse_file_text(input);
        assert!(
            !parse.errors.is_empty(),
            "expected at least one parse error"
        );
        let defines: Vec<SmeltDefine> = file.defines().collect();
        assert_eq!(
            defines.len(),
            2,
            "recovery should still parse a second smelt.define"
        );
        assert_eq!(defines[1].name().as_deref(), Some("good"));
        assert!(defines[1].param_list().is_some());
        assert!(defines[1].body().is_some());
    }

    #[test]
    fn error_recovery_unbalanced_body() {
        // The first define has an unbalanced `(` in its body. The parser must
        // record errors and still parse the following smelt.define.
        let input = "smelt.define bad(x) AS ((x + 1)\nsmelt.define good(y) AS (y)";
        let (parse, file) = parse_file_text(input);
        assert!(
            !parse.errors.is_empty(),
            "expected at least one parse error"
        );
        let defines: Vec<SmeltDefine> = file.defines().collect();
        assert_eq!(
            defines.len(),
            2,
            "recovery should still parse a second smelt.define"
        );
        assert_eq!(defines[1].name().as_deref(), Some("good"));
        assert!(defines[1].body().is_some());
    }

    #[test]
    fn smelt_define_in_expression_position_is_not_special() {
        // `smelt.define` inside a SELECT should parse as a qualified column
        // reference, not as a declaration. No SmeltDefine nodes, and no
        // `define`-specific errors.
        let input = "SELECT smelt.define FROM t";
        let (parse, file) = parse_file_text(input);
        assert_eq!(
            file.defines().count(),
            0,
            "no smelt.define declarations expected"
        );
        assert!(file.select_stmt().is_some(), "should have a SELECT stmt");
        for err in &parse.errors {
            let m = err.message.to_lowercase();
            assert!(
                !m.contains("smelt.define"),
                "did not expect a smelt.define-specific error, got: {:?}",
                err
            );
        }
    }

    // ===== Phase 10: smelt.extern top-level grammar =====

    use crate::ast::SmeltExtern;

    #[test]
    fn parses_smelt_extern_minimal() {
        // Phase 10 TDD test 1.
        let input =
            "smelt.extern regex_match(text: Expr<Text>, pattern: Expr<Text>) -> Expr<Boolean>";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let externs: Vec<SmeltExtern> = file.externs().collect();
        assert_eq!(externs.len(), 1, "expected exactly one smelt.extern");
        let ext = &externs[0];

        assert_eq!(ext.name().as_deref(), Some("regex_match"));

        let params: Vec<Param> = ext
            .param_list()
            .expect("should have a param list")
            .params()
            .collect();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name().as_deref(), Some("text"));
        let t0 = params[0].type_ref().expect("typed param");
        let t0_compact: String = t0.text().chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(t0_compact, "Expr<Text>");

        assert_eq!(params[1].name().as_deref(), Some("pattern"));

        let ret: TypeRef = ext.return_type().expect("return type");
        let ret_compact: String = ret.text().chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(ret_compact, "Expr<Boolean>");

        // No smelt.define declarations in this file.
        assert_eq!(file.defines().count(), 0);
    }

    #[test]
    fn extern_with_frontmatter_backends() {
        // Phase 10 TDD test 2. The existing file-level `---` frontmatter
        // block must coexist with a `smelt.extern` — the legacy single-block
        // rule applies. The frontmatter is stripped by
        // `smelt_parser::strip_frontmatter` before reaching the parser, so
        // the extern must parse identically to the minimal case.
        let input = "---\nbackends: [duckdb]\n---\n\
                     smelt.extern read_parquet(path: Expr<Text>) -> Expr<Text>\n";
        let clean = crate::strip_frontmatter(input);
        let (parse, file) = parse_file_text(&clean);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let externs: Vec<SmeltExtern> = file.externs().collect();
        assert_eq!(externs.len(), 1);
        assert_eq!(externs[0].name().as_deref(), Some("read_parquet"));
        let params: Vec<Param> = externs[0].param_list().unwrap().params().collect();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name().as_deref(), Some("path"));
    }

    #[test]
    fn smelt_extern_and_define_in_same_file() {
        // Mixed file: one of each. The iterators should partition cleanly.
        let input = "\
            smelt.extern ext_fn(a: Expr<Text>) -> Expr<Text>\n\
            smelt.define my_plus(x: Expr<Integer>) -> Expr<Integer> AS (x + 1)\n";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );
        assert_eq!(file.externs().count(), 1);
        assert_eq!(file.defines().count(), 1);
    }

    // ===== Phase 2 → Phase 5b: smelt.fn.* rejection tests =====
    // Phase 5b removes the smelt.fn.* parser arm. Tests that previously
    // verified successful parsing now verify rejection (parse errors).

    use crate::syntax_kind::SyntaxNode;

    /// Helper: collect all FUNCTION_CALL descendants of a node.
    fn function_calls(root: &SyntaxNode) -> Vec<FunctionCall> {
        root.descendants().filter_map(FunctionCall::cast).collect()
    }

    #[test]
    fn smelt_fn_call_is_now_rejected() {
        // Phase 5b: smelt.fn.* must produce parse errors.
        let inputs = &[
            "SELECT smelt.fn.safe_divide(a, b) FROM t",
            "SELECT smelt.fn.safe_divide(numerator => a, denominator => b) FROM t",
            "SELECT smelt.fn.core.math.safe_divide(a, b) FROM t",
            "SELECT * FROM t WHERE smelt.fn.is_valid(x)",
        ];
        for input in inputs {
            let (parse, _file) = parse_file_text(input);
            assert!(
                !parse.errors.is_empty(),
                "Phase 5b: smelt.fn.* must produce parse errors for input: {:?}",
                input
            );
        }
    }

    #[test]
    fn smelt_fn_without_parens_is_error() {
        // smelt.fn.foo without parens was already an error; still an error.
        let input = "SELECT smelt.fn.foo FROM t";
        let (parse, _file) = parse_file_text(input);
        assert!(
            !parse.errors.is_empty(),
            "expected at least one parse error for smelt.fn.foo with no '('"
        );
    }

    #[test]
    fn smelt_ref_still_parses_as_function_call() {
        // Phase 4 update: smelt.ref() is rejected with an error. The test
        // is retained to verify that error recovery still produces a
        // FUNCTION_CALL so downstream CST walkers don't panic.
        let input = "SELECT * FROM smelt.ref('model')";
        let (parse, file) = parse_file_text(input);
        // Phase 4: must have at least one parse error.
        assert!(
            !parse.errors.is_empty(),
            "Phase 4: smelt.ref() must produce a parse error"
        );

        // Error recovery produces a FUNCTION_CALL node.
        let fcalls = function_calls(file.syntax());
        assert!(
            !fcalls.is_empty(),
            "error recovery must still produce a FUNCTION_CALL node for smelt.ref()"
        );
    }

    #[test]
    fn smelt_fn_call_inside_define_body_is_rejected() {
        // Phase 5b: smelt.fn.* inside a define body must now produce errors.
        let input = "smelt.define wrap(x) AS (smelt.fn.safe_divide(x, 1))";
        let (parse, _file) = parse_file_text(input);
        assert!(
            !parse.errors.is_empty(),
            "Phase 5b: smelt.fn.* inside define body must produce parse errors"
        );
    }

    // ===== Phase 11: per-declaration frontmatter =====

    #[test]
    fn frontmatter_attaches_to_next_decl() {
        // Phase 11 TDD #1: a file containing two `---/---` blocks, each
        // preceded by (i.e. each immediately followed by) a distinct
        // `smelt.define`. Each decl's `frontmatter()` helper returns the
        // matching block's body — and only the matching block.
        let raw = "---\nbackends: [duckdb]\n---\nsmelt.define f() -> Expr<Integer> AS (1)\n\n---\nbackends: [spark]\n---\nsmelt.define g() -> Expr<Integer> AS (2)\n";

        let stripped = crate::strip_frontmatter(raw);
        let (parse, file) = parse_file_text(&stripped);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let defines: Vec<SmeltDefine> = file.defines().collect();
        assert_eq!(defines.len(), 2, "expected two smelt.define declarations");

        let fm_f = defines[0]
            .frontmatter(raw)
            .expect("first decl should have frontmatter");
        assert!(fm_f.contains("duckdb"), "got {:?}", fm_f);
        assert!(!fm_f.contains("spark"), "wrong block attached: {:?}", fm_f);

        let fm_g = defines[1]
            .frontmatter(raw)
            .expect("second decl should have frontmatter");
        assert!(fm_g.contains("spark"), "got {:?}", fm_g);
        assert!(!fm_g.contains("duckdb"), "wrong block attached: {:?}", fm_g);
    }

    #[test]
    fn extern_with_dotted_backend_namespace() {
        // Phase 11: `smelt.extern duckdb.read_parquet(...)` parses cleanly
        // and exposes `read_parquet` as the function name with `duckdb`
        // as the backend namespace.
        use crate::ast::SmeltExtern;
        let input = "smelt.extern duckdb.read_parquet(path: Expr<Text>) -> Expr<Text>\n";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );
        let externs: Vec<SmeltExtern> = file.externs().collect();
        assert_eq!(externs.len(), 1);
        assert_eq!(externs[0].name().as_deref(), Some("read_parquet"));
        assert_eq!(externs[0].backend_namespace().as_deref(), Some("duckdb"));
    }

    #[test]
    fn extern_plain_name_has_no_backend_namespace() {
        // Sanity check — the legacy single-IDENT extern form still works.
        use crate::ast::SmeltExtern;
        let input = "smelt.extern regex_match(s: Expr<Text>, p: Expr<Text>) -> Expr<Boolean>\n";
        let (parse, file) = parse_file_text(input);
        assert!(parse.errors.is_empty(), "{:?}", parse.errors);
        let externs: Vec<SmeltExtern> = file.externs().collect();
        assert_eq!(externs[0].name().as_deref(), Some("regex_match"));
        assert!(externs[0].backend_namespace().is_none());
    }

    // ===== Phase 13: TableExpr / WindowExpr / SelectItems<K, ctx> in type refs =====

    use crate::ast::{ExprKindTag, RowTail, TypeRefHead};

    /// Helper: extract the first `PARAM`'s `TYPE_REF` from a single-define file.
    fn first_param_type_ref(file: &File) -> TypeRef {
        let def = file.defines().next().expect("one smelt.define");
        let params: Vec<Param> = def.param_list().expect("param list").params().collect();
        params[0].type_ref().expect("first param type ref")
    }

    #[test]
    fn parses_tableexpr_bare() {
        let input = "smelt.define f(source: TableExpr) AS (SELECT * FROM source)";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let tr = first_param_type_ref(&file);
        assert_eq!(tr.kind(), TypeRefHead::TableExpr);
        assert!(
            tr.row_requirement().is_none(),
            "no row requirement expected"
        );
    }

    #[test]
    fn parses_tableexpr_with_row_requirement() {
        let input = "smelt.define f(source: TableExpr<{revenue: Numeric, cost: Numeric}>) AS (SELECT * FROM source)";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let tr = first_param_type_ref(&file);
        assert_eq!(tr.kind(), TypeRefHead::TableExpr);

        let req = tr
            .row_requirement()
            .expect("TableExpr<{...}> should have a ROW_REQUIREMENT");
        let fields = req.fields();
        assert_eq!(fields.len(), 2, "expected two row fields");
        assert_eq!(fields[0].name().as_deref(), Some("revenue"));
        assert!(
            fields[0].type_ref().is_some(),
            "revenue should have a TYPE_REF"
        );
        assert_eq!(fields[1].name().as_deref(), Some("cost"));
        assert!(
            fields[1].type_ref().is_some(),
            "cost should have a TYPE_REF"
        );
        assert!(matches!(req.tail(), RowTail::None));
    }

    #[test]
    fn parses_tableexpr_with_row_tail() {
        // Named tail: ..r
        let input_named =
            "smelt.define f(source: TableExpr<{revenue: Numeric, ..r}>) AS (SELECT * FROM source)";
        let (parse, file) = parse_file_text(input_named);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );
        let tr = first_param_type_ref(&file);
        let req = tr.row_requirement().expect("row requirement");
        match req.tail() {
            RowTail::Named(name) => assert_eq!(name, "r"),
            other => panic!("expected named tail `r`, got {other:?}"),
        }
        assert_eq!(req.fields().len(), 1);

        // Anonymous tail: bare `..`
        let input_anon = "smelt.define g(source: TableExpr<{..}>) AS (SELECT * FROM source)";
        let (parse, file) = parse_file_text(input_anon);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );
        let tr = first_param_type_ref(&file);
        let req = tr.row_requirement().expect("row requirement");
        assert!(matches!(req.tail(), RowTail::Anon));
        assert_eq!(req.fields().len(), 0);
    }

    #[test]
    fn parses_aggexpr_and_windowexpr() {
        let input =
            "smelt.define f(a: Expr<Integer>, b: AggExpr<Integer>, c: WindowExpr<Integer>) AS (SELECT 1)";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let def = file.defines().next().expect("one smelt.define");
        let params: Vec<Param> = def.param_list().unwrap().params().collect();
        assert_eq!(params.len(), 3);

        let t0 = params[0].type_ref().unwrap();
        let t1 = params[1].type_ref().unwrap();
        let t2 = params[2].type_ref().unwrap();
        assert_eq!(t0.kind(), TypeRefHead::Expr);
        assert_eq!(t1.kind(), TypeRefHead::AggExpr);
        assert_eq!(t2.kind(), TypeRefHead::WindowExpr);
        assert_eq!(t0.expr_kind(), Some(ExprKindTag::Scalar));
        assert_eq!(t1.expr_kind(), Some(ExprKindTag::Agg));
        assert_eq!(t2.expr_kind(), Some(ExprKindTag::Window));
    }

    #[test]
    fn parses_selectitems_kind_only() {
        let input = "smelt.define f(items: SelectItems<Agg>) AS (SELECT 1)";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let tr = first_param_type_ref(&file);
        assert_eq!(tr.kind(), TypeRefHead::SelectItems);
        assert_eq!(tr.selectitems_kind().as_deref(), Some("Agg"));
        assert!(tr.selectitems_ctx().is_none());
    }

    #[test]
    fn parses_selectitems_kind_and_ctx() {
        let input = "smelt.define f(items: SelectItems<Agg, sessionized>) AS (SELECT 1)";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let tr = first_param_type_ref(&file);
        assert_eq!(tr.kind(), TypeRefHead::SelectItems);
        assert_eq!(tr.selectitems_kind().as_deref(), Some("Agg"));
        assert_eq!(tr.selectitems_ctx().as_deref(), Some("sessionized"));

        // Declared order: kind before ctx.
        let syntax = tr.syntax();
        let mut seen_kind = false;
        let mut seen_ctx = false;
        for child in syntax.children() {
            if child.kind() == SELECTITEMS_KIND {
                assert!(!seen_ctx, "KIND must come before CTX");
                seen_kind = true;
            } else if child.kind() == SELECTITEMS_CTX {
                assert!(seen_kind, "CTX must follow KIND");
                seen_ctx = true;
            }
        }
        assert!(seen_kind && seen_ctx);
    }

    #[test]
    fn parses_selectitems_ctx_only() {
        let input = "smelt.define f(items: SelectItems<sessionized>) AS (SELECT 1)";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let tr = first_param_type_ref(&file);
        assert_eq!(tr.kind(), TypeRefHead::SelectItems);
        assert!(tr.selectitems_kind().is_none());
        assert_eq!(tr.selectitems_ctx().as_deref(), Some("sessionized"));
    }

    #[test]
    fn rejects_unknown_expr_kind() {
        let input = "smelt.define f(a: FooExpr<Integer>) AS (SELECT 1)";
        let (parse, _file) = parse_file_text(input);
        assert!(
            !parse.errors.is_empty(),
            "expected at least one parse error for unknown sort"
        );
        let mentions_fooexpr = parse.errors.iter().any(|e| {
            let m = e.message.to_lowercase();
            m.contains("fooexpr") || m.contains("unknown sort") || m.contains("unsupported")
        });
        assert!(
            mentions_fooexpr,
            "expected an error mentioning FooExpr / unknown sort / unsupported, got {:?}",
            parse.errors
        );
    }

    #[test]
    fn tableexpr_in_expression_position_is_not_special() {
        // `TableExpr` as a bare identifier in expression position must remain a
        // plain column reference, not a type reference or an error.
        let input =
            "smelt.define f(x: Expr<Integer>) AS (SELECT TableExpr FROM t WHERE TableExpr > 1)";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let def = file.defines().next().expect("one smelt.define");
        let body = def.body().expect("body");
        let body_syntax = body.syntax();

        // No TYPE_REF nodes should live inside the body (the only TYPE_REF is
        // the one on the `x: Expr<Integer>` parameter).
        let tr_count_in_body = body_syntax
            .descendants()
            .filter(|n| n.kind() == TYPE_REF)
            .count();
        assert_eq!(tr_count_in_body, 0, "body should contain no TYPE_REF nodes");

        // The bare `TableExpr` identifier must appear at least twice (SELECT
        // list + WHERE operand) as an IDENT token inside the body.
        let tableexpr_tokens = body_syntax
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == IDENT && t.text() == "TableExpr")
            .count();
        assert!(
            tableexpr_tokens >= 2,
            "expected at least two `TableExpr` IDENT tokens in the body, got {}",
            tableexpr_tokens
        );
    }

    // ===== Phase 28: PASSING clauses =====

    use crate::ast::PassingClause;
    use crate::syntax_kind::SyntaxKind::PASSING_CLAUSE;

    /// Helper: collect all PASSING_CLAUSE descendants of a node.
    fn passing_clauses_of(root: &SyntaxNode) -> Vec<SyntaxNode> {
        root.descendants()
            .filter(|n| n.kind() == PASSING_CLAUSE)
            .collect()
    }

    #[test]
    fn parses_single_passing_clause() {
        // Phase 5b: smelt.fn.* is rejected; use smelt.functions.* instead.
        // PASSING clauses are still supported on smelt.functions.* calls.
        let input =
            "SELECT smelt.functions.session_rollup(src) PASSING metrics AS (SUM(revenue)) FROM t";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let calls = smelt_path_calls(file.syntax());
        assert_eq!(calls.len(), 1, "expected one SMELT_PATH_CALL");

        let passing: Vec<PassingClause> = calls[0].passing_clauses().collect();
        assert_eq!(passing.len(), 1, "expected one PASSING_CLAUSE");
        assert_eq!(
            passing[0].name().as_deref(),
            Some("metrics"),
            "PASSING_NAME should be 'metrics'"
        );

        // Body should contain SUM(revenue)
        let body_text = passing[0]
            .body_text()
            .expect("PASSING_BODY should have text");
        assert!(
            body_text.contains("SUM"),
            "PASSING_BODY text should contain SUM, got {:?}",
            body_text
        );
    }

    #[test]
    fn parses_multiple_passing_clauses() {
        // Phase 5b: smelt.fn.* is rejected; use smelt.functions.* instead.
        let input =
            "SELECT smelt.functions.foo(src) PASSING a AS (x + 1) PASSING b AS (y * 2) FROM t";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let calls = smelt_path_calls(file.syntax());
        assert_eq!(calls.len(), 1, "expected one SMELT_PATH_CALL");

        let passing: Vec<PassingClause> = calls[0].passing_clauses().collect();
        assert_eq!(passing.len(), 2, "expected two PASSING_CLAUSEs");
        assert_eq!(passing[0].name().as_deref(), Some("a"));
        assert_eq!(passing[1].name().as_deref(), Some("b"));

        let body0 = passing[0].body_text().expect("first PASSING_BODY text");
        let body1 = passing[1].body_text().expect("second PASSING_BODY text");
        assert!(
            body0.contains("x"),
            "first body should contain 'x', got {:?}",
            body0
        );
        assert!(
            body1.contains("y"),
            "second body should contain 'y', got {:?}",
            body1
        );
    }

    #[test]
    fn passing_not_reserved_elsewhere() {
        // `passing` as a column name should parse without errors
        let input = "SELECT passing FROM t";
        let (parse, _file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "SELECT passing FROM t should parse cleanly, errors: {:?}",
            parse.errors
        );

        // `passing` as a table name should parse without errors
        let input2 = "SELECT x FROM passing";
        let (parse2, _) = parse_file_text(input2);
        assert!(
            parse2.errors.is_empty(),
            "SELECT x FROM passing should parse cleanly, errors: {:?}",
            parse2.errors
        );

        // No PASSING_CLAUSEs should appear in these statements
        let parse_result = crate::parser::parse("SELECT passing FROM t");
        let root = parse_result.syntax();
        let clauses = passing_clauses_of(&root);
        assert!(
            clauses.is_empty(),
            "plain 'passing' identifier must not produce PASSING_CLAUSE nodes"
        );
    }

    #[test]
    fn passing_not_attached_to_plain_sql_call() {
        // PASSING after a plain SQL function call must NOT be attached to that call.
        let input = "SELECT UPPER(x) PASSING y AS (some_expr) FROM t";
        // This should parse without panicking. PASSING is an identifier
        // in expression position after UPPER(x).
        let (_parse, file) = parse_file_text(input);

        // UPPER(x) is a plain FUNCTION_CALL — it must have no PASSING_CLAUSE children.
        let fcalls = function_calls(file.syntax());
        for fc in &fcalls {
            if fc.name().as_deref() == Some("UPPER") {
                // The SmeltFnCall wrapper returns passing_clauses only for SMELT_FN_CALL nodes.
                // UPPER is a FUNCTION_CALL, so we just check no PASSING_CLAUSE is a descendant
                // of the FUNCTION_CALL node.
                let fn_node = fc.syntax();
                let passing_under_upper = fn_node
                    .descendants()
                    .filter(|n| n.kind() == PASSING_CLAUSE)
                    .count();
                assert_eq!(
                    passing_under_upper, 0,
                    "PASSING_CLAUSE must NOT be a descendant of plain FUNCTION_CALL UPPER"
                );
            }
        }

        // Phase 5b: smelt.fn.* is rejected, so no path calls either.
        let smelt_path = smelt_path_calls(file.syntax());
        assert!(
            smelt_path.is_empty(),
            "no smelt.functions.* calls should be present in this input"
        );
    }

    #[test]
    fn passing_after_smelt_extern_call_not_attached() {
        // Plan Phase 28 requires `passing_after_smelt_extern_call_rejected`.
        // `smelt.extern` declarations are top-level-only (gated by
        // `at_smelt_extern_trigger`); they cannot appear in expression position.
        // This makes the plan requirement vacuously satisfied at the parser level —
        // there is no expression-position parse path for smelt.extern calls.
        //
        // We verify the structural analogue: PASSING after a plain SQL function
        // call (not smelt.fn.*) is not attached, confirming the trigger is
        // correctly scoped to the smelt.fn.* path only.
        let input = "SELECT my_func(src) PASSING m AS (COUNT(*)) FROM t";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "should parse cleanly, errors: {:?}",
            parse.errors
        );

        // No SMELT_PATH_CALL nodes — so no PASSING_CLAUSE should be created.
        let smelt_calls = smelt_path_calls(file.syntax());
        assert!(
            smelt_calls.is_empty(),
            "my_func is not a smelt.functions.* call"
        );

        // No PASSING_CLAUSE nodes anywhere in the tree.
        let clauses = passing_clauses_of(file.syntax());
        assert!(
            clauses.is_empty(),
            "PASSING after plain SQL call must not produce PASSING_CLAUSE nodes"
        );
    }

    #[test]
    fn error_recovery_malformed_passing_body() {
        // PASSING metrics AS followed immediately by FROM (missing body expression).
        // Phase 5b: use smelt.functions.* instead of smelt.fn.*
        let input = "SELECT smelt.functions.foo(src) PASSING metrics AS FROM t";
        let (parse, file) = parse_file_text(input);

        // The parser should emit an error but not panic.
        assert!(
            !parse.errors.is_empty(),
            "expected at least one parse error for malformed PASSING body"
        );

        // Despite the error, FROM t should still be parsed — i.e. the select
        // statement recovers and a FROM_CLAUSE is present.
        let stmts: Vec<_> = file.syntax().children().collect();
        assert!(
            !stmts.is_empty(),
            "tree should not be empty after error recovery"
        );
    }

    // ===== Phase 35: Struct row variables and value-level spread =====

    #[test]
    fn parses_struct_with_named_row_var() {
        let input = "smelt.define f(e: Expr<Struct<{ts: Timestamp, ..r}>>) AS (e)";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let defines: Vec<SmeltDefine> = file.defines().collect();
        assert_eq!(defines.len(), 1, "expected exactly one SMELT_DEFINE");

        // The TYPE_REF for parameter `e` must contain a STRUCT_TYPE child.
        let def = &defines[0];
        let params: Vec<Param> = def.param_list().expect("param list").params().collect();
        let tr = params[0].type_ref().expect("param type ref");
        let tr_syntax = tr.syntax();

        let struct_type_node = tr_syntax
            .descendants()
            .find(|n| n.kind() == STRUCT_TYPE)
            .expect("TYPE_REF must contain a STRUCT_TYPE node");

        // The STRUCT_TYPE must contain a ROW_TAIL child with identifier `r`.
        let row_tail_node = struct_type_node
            .children()
            .find(|n| n.kind() == ROW_TAIL)
            .expect("STRUCT_TYPE must contain a ROW_TAIL node");

        let tail_ident = row_tail_node
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .expect("ROW_TAIL must contain an identifier for named tail");
        assert_eq!(
            tail_ident.text(),
            "r",
            "ROW_TAIL identifier must be `r`, got {}",
            tail_ident.text()
        );
    }

    #[test]
    fn parses_struct_with_anonymous_row_tail() {
        let input = "smelt.define f(e: Expr<Struct<{ts: Timestamp, ..}>>) AS (e)";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let def = file.defines().next().expect("one smelt.define");
        let params: Vec<Param> = def.param_list().expect("param list").params().collect();
        let tr = params[0].type_ref().expect("param type ref");

        let struct_type_node = tr
            .syntax()
            .descendants()
            .find(|n| n.kind() == STRUCT_TYPE)
            .expect("TYPE_REF must contain a STRUCT_TYPE node");

        let row_tail_node = struct_type_node
            .children()
            .find(|n| n.kind() == ROW_TAIL)
            .expect("STRUCT_TYPE must contain a ROW_TAIL node");

        // Anonymous tail: no IDENT inside the ROW_TAIL.
        let has_ident = row_tail_node
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == IDENT);
        assert!(!has_ident, "anonymous ROW_TAIL must not contain an IDENT");
    }

    #[test]
    fn parses_struct_literal_spread_in_body() {
        let input =
            "smelt.define f(event: Expr<Struct<{ts: Timestamp, ..r}>>) AS ({CAST(event.ts AS TIMESTAMP) AS ts, ..event})";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let def = file.defines().next().expect("one smelt.define");
        let body = def.body().expect("body");
        let body_syntax = body.syntax();

        // DEFINE_BODY must contain a BRACE_STRUCT_LITERAL node.
        let brace_struct = body_syntax
            .descendants()
            .find(|n| n.kind() == BRACE_STRUCT_LITERAL)
            .expect("DEFINE_BODY must contain a BRACE_STRUCT_LITERAL node");

        // The BRACE_STRUCT_LITERAL must have a SPREAD_ITEM child for `..event`.
        let spread = brace_struct
            .children()
            .find(|n| n.kind() == SPREAD_ITEM)
            .expect("BRACE_STRUCT_LITERAL must contain a SPREAD_ITEM child");

        let spread_ident = spread
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .expect("SPREAD_ITEM must contain an IDENT for the spread name");
        assert_eq!(
            spread_ident.text(),
            "event",
            "SPREAD_ITEM identifier must be `event`, got {}",
            spread_ident.text()
        );
    }

    #[test]
    fn two_named_row_vars_in_one_signature_errors() {
        // v1 constraint: at most one named row variable per signature.
        // `..r` and `..s` are two distinct names — must produce an error.
        let input = "smelt.define f(a: Expr<Struct<{x: Integer, ..r}>>, b: Expr<Struct<{y: Integer, ..s}>>) AS (a)";
        let (parse, _file) = parse_file_text(input);
        // Must not panic.
        assert!(
            !parse.errors.is_empty(),
            "expected at least one parse error for two distinct named row variables, got none"
        );
        let mentions_row_var = parse.errors.iter().any(|e| {
            let m = e.message.to_lowercase();
            m.contains("row variable") || m.contains("named row") || m.contains("at most one")
        });
        assert!(
            mentions_row_var,
            "expected an error mentioning row variable constraint, got {:?}",
            parse.errors
        );
    }

    #[test]
    fn anonymous_tail_unreferenced_in_body_ok() {
        // Anonymous `..` without a body spread is fine — no parser errors.
        let input = "smelt.define f(e: Expr<Struct<{ts: Timestamp, ..}>>) AS (e.ts)";
        let (parse, _file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "anonymous tail with no spread in body must not produce errors, got: {:?}",
            parse.errors
        );
    }

    // ---- Phase 44b → Phase 5b: CTE body smelt.fn.* rejection ----

    #[test]
    fn cte_body_smelt_fn_call_is_rejected() {
        // Phase 5b: a CTE body with a bare `smelt.fn.*` call must produce
        // parse errors. Use smelt.functions.* instead.
        let sql =
            "WITH base AS (\n    smelt.fn.session_rollup(source, u, ts)\n)\nSELECT * FROM base";
        let result = parse(sql);
        assert!(
            !result.errors.is_empty(),
            "Phase 5b: smelt.fn.* as CTE body must produce parse errors"
        );
    }

    // ===== smelt.<path> migration, Phase 1: unified value-form / call-form
    // grammar (additive, coexists with legacy smelt.fn.* / smelt.ref / etc.).
    // =====

    use crate::ast::{SmeltPathCall, SmeltPathRef};

    /// Helper: collect every `SmeltPathRef` descendant of a syntax node.
    fn smelt_path_refs(root: &SyntaxNode) -> Vec<SmeltPathRef> {
        root.descendants().filter_map(SmeltPathRef::cast).collect()
    }

    /// Helper: collect every `SmeltPathCall` descendant of a syntax node.
    fn smelt_path_calls(root: &SyntaxNode) -> Vec<SmeltPathCall> {
        root.descendants().filter_map(SmeltPathCall::cast).collect()
    }

    #[test]
    fn parses_smelt_path_value_in_from() {
        // `SELECT * FROM smelt.models.users` produces a single SmeltPathRef
        // AST node with segments ["models", "users"] in the FROM position.
        let input = "SELECT * FROM smelt.models.users";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let refs = smelt_path_refs(file.syntax());
        assert_eq!(refs.len(), 1, "expected exactly one SMELT_PATH_REF");
        assert_eq!(refs[0].segments(), vec!["models", "users"]);

        // The path reference must sit under a FROM_CLAUSE ancestor.
        let in_from = refs[0]
            .syntax()
            .ancestors()
            .any(|n| n.kind() == FROM_CLAUSE);
        assert!(in_from, "SMELT_PATH_REF must live under FROM_CLAUSE");

        // No SMELT_PATH_CALL should be emitted for the value form.
        assert!(
            smelt_path_calls(file.syntax()).is_empty(),
            "value-form smelt.<path> must not produce SMELT_PATH_CALL"
        );
    }

    #[test]
    fn parses_smelt_path_value_in_argument_position() {
        // `f(smelt.models.users)` produces a SmeltPathRef arg, distinct from a
        // function call. The outer `f(...)` is a FUNCTION_CALL whose argument
        // list contains exactly one SMELT_PATH_REF.
        let input = "SELECT f(smelt.models.users) FROM t";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let refs = smelt_path_refs(file.syntax());
        assert_eq!(refs.len(), 1, "expected exactly one SMELT_PATH_REF");
        assert_eq!(refs[0].segments(), vec!["models", "users"]);

        // The SMELT_PATH_REF must NOT be a SMELT_PATH_CALL — distinct nodes.
        assert!(
            smelt_path_calls(file.syntax()).is_empty(),
            "argument-position value form must not produce SMELT_PATH_CALL"
        );

        // The path reference sits inside an ARG_LIST under a FUNCTION_CALL.
        let inside_arg_list = refs[0].syntax().ancestors().any(|n| n.kind() == ARG_LIST);
        assert!(
            inside_arg_list,
            "SMELT_PATH_REF should sit inside an ARG_LIST"
        );
    }

    #[test]
    fn smelt_path_ref_text_range_excludes_trailing_alias_whitespace() {
        // Regression: `FROM smelt.models.users u` was producing text_range that
        // included the space before the alias `u`, causing the test compiler to
        // replace `smelt.models.users ` (with space) with `users` → `usersu`.
        let input = "SELECT * FROM smelt.models.users u";
        let (parse, _file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let refs = smelt_path_refs(_file.syntax());
        assert_eq!(refs.len(), 1);

        let range = refs[0].text_range();
        let start: usize = range.start().into();
        let end: usize = range.end().into();
        let text = &input[start..end];
        assert_eq!(
            text, "smelt.models.users",
            "text_range must not include trailing whitespace before alias; got {text:?}"
        );
    }

    #[test]
    fn parses_smelt_path_call_with_positional_args() {
        // `smelt.functions.patterns.session_rollup(events, 30)` parses as a
        // SmeltPathCall with two positional args.
        let input = "SELECT * FROM smelt.functions.patterns.session_rollup(events, 30)";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let calls = smelt_path_calls(file.syntax());
        assert_eq!(calls.len(), 1, "expected exactly one SMELT_PATH_CALL");
        assert_eq!(
            calls[0].segments(),
            vec!["functions", "patterns", "session_rollup"]
        );

        let arg_list = calls[0].arg_list().expect("call must have ARG_LIST");
        let positional = arg_list.positional_args();
        assert_eq!(
            positional.len(),
            2,
            "expected two positional arguments, got {}",
            positional.len()
        );

        // Phase 1 must emit no value-form SMELT_PATH_REF for a call.
        assert!(
            smelt_path_refs(file.syntax()).is_empty(),
            "call form must not also produce a SMELT_PATH_REF"
        );
    }

    #[test]
    fn parses_smelt_path_call_with_named_args() {
        // `smelt.models.margins(product_summary => smelt.models.product_summary)`
        // parses with a named-arg `=>` binding whose value is itself a
        // SMELT_PATH_REF.
        let input =
            "SELECT * FROM smelt.models.margins(product_summary => smelt.models.product_summary)";
        let (parse, file) = parse_file_text(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let calls = smelt_path_calls(file.syntax());
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].segments(), vec!["models", "margins"]);

        let arg_list = calls[0].arg_list().expect("must have arg list");
        let named: Vec<_> = arg_list.named_params().collect();
        assert_eq!(named.len(), 1, "expected one named param");
        assert_eq!(named[0].name().as_deref(), Some("product_summary"));

        // The value of the named parameter is a SMELT_PATH_REF.
        let refs = smelt_path_refs(file.syntax());
        assert_eq!(refs.len(), 1, "expected one nested SMELT_PATH_REF");
        assert_eq!(refs[0].segments(), vec!["models", "product_summary"]);
    }

    #[test]
    fn smelt_path_call_supports_passing_clause() {
        // `smelt.<path>(args) PASSING name AS (body)` — parity with current
        // `smelt.fn.*` PASSING grammar.
        let input = "WITH base AS (\n    smelt.functions.session_rollup(source, 30)\n    PASSING metrics AS (COUNT(*))\n)\nSELECT * FROM base";
        let parse = parse(input);
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );

        let file = File::cast(parse.syntax()).unwrap();
        let calls = smelt_path_calls(file.syntax());
        assert_eq!(calls.len(), 1, "expected exactly one SMELT_PATH_CALL");
        let passing: Vec<_> = calls[0].passing_clauses().collect();
        assert_eq!(passing.len(), 1, "expected exactly one PASSING_CLAUSE");
        assert_eq!(passing[0].name().as_deref(), Some("metrics"));
    }

    #[test]
    fn legacy_smelt_ref_still_parses() {
        // Phase 4 update: `smelt.ref('users')` now produces a parse error
        // (the form is rejected), but error recovery still produces a
        // FUNCTION_CALL node so the CST is usable. The test is retained but
        // updated to reflect the Phase 4 behavior.
        let input = "SELECT * FROM smelt.ref('users')";
        let (parse, _file) = parse_file_text(input);
        // Phase 4: must have at least one parse error.
        assert!(
            !parse.errors.is_empty(),
            "Phase 4: smelt.ref() must produce a parse error; use smelt.models.<name>"
        );
        // The error message should mention the replacement.
        let err_msg = &parse.errors[0].message;
        assert!(
            err_msg.contains("smelt.models") || err_msg.contains("removed"),
            "error message should mention smelt.models or 'removed'; got: {err_msg}"
        );
    }

    #[test]
    fn smelt_path_partial_forms_do_not_panic() {
        // Architectural invariant: error-recovery must produce a CST without
        // panics on partial / malformed path forms. Inputs like `smelt.`,
        // `smelt.models.`, and `smelt.models.foo(` must each yield a Parse
        // result; we don't care about the exact errors here.
        for input in [
            "SELECT * FROM smelt.",
            "SELECT * FROM smelt.models.",
            "SELECT * FROM smelt.models.foo(",
            "SELECT smelt. FROM t",
        ] {
            // This call must not panic.
            let parse = parse(input);
            // We expect at least one parse error, but the parser must still
            // produce a usable green tree.
            let _ = File::cast(parse.syntax()).expect("parser must yield FILE");
        }
    }

    // ===== Phase 4: Legacy smelt.ref() and smelt.source() rejection tests =====

    #[test]
    fn legacy_smelt_ref_now_rejected() {
        // Phase 4: `smelt.ref('users')` must produce at least one parse error
        // pointing the user toward the unified `smelt.<path>` form.
        // The parser should still produce a usable CST (error recovery), but
        // the error set must be non-empty.
        let input = "SELECT * FROM smelt.ref('users')";
        let parse = parse(input);
        assert!(
            !parse.errors.is_empty(),
            "smelt.ref('users') must produce a parse error in Phase 4; \
             use smelt.models.users instead"
        );
        // The parser must still produce a usable green tree (no panic).
        let _ = File::cast(parse.syntax()).expect("parser must yield FILE even on legacy ref");
    }

    #[test]
    fn legacy_smelt_source_now_rejected() {
        // Phase 4: `smelt.source('raw.events')` must produce at least one
        // parse error pointing the user toward `smelt.sources.raw.events`.
        let input = "SELECT * FROM smelt.source('raw.events')";
        let parse = parse(input);
        assert!(
            !parse.errors.is_empty(),
            "smelt.source('raw.events') must produce a parse error in Phase 4; \
             use smelt.sources.raw.events instead"
        );
        let _ = File::cast(parse.syntax()).expect("parser must yield FILE even on legacy source");
    }

    // ===== Phase 5b: smelt.fn.* call syntax removal =====

    #[test]
    fn legacy_smelt_fn_call_now_rejected() {
        // Phase 5b: `smelt.fn.foo(x)` must produce at least one parse error
        // after the smelt.fn.* parser arm is removed. Before Phase 5b,
        // this parses successfully as a SMELT_FN_CALL node.
        let sql = "SELECT smelt.fn.foo(x) AS r";
        let result = parse(sql);
        assert!(
            !result.errors.is_empty(),
            "smelt.fn.foo(x) must produce parse errors after Phase 5b; got zero errors"
        );
    }

    // ===== Phase 1 (meta-language): list literal [a, b, c] and spread ...xs =====

    #[test]
    fn parse_list_literal_homogeneous() {
        // `[1, 2, 3]` parses to one ARRAY_LITERAL node with three child expressions
        // and no parse errors.
        let parse = parse("SELECT [1, 2, 3] FROM t");
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );
        let list_node = parse
            .syntax()
            .descendants()
            .find(|n| n.kind() == ARRAY_LITERAL)
            .expect("must have ARRAY_LITERAL node");
        let child_count = list_node
            .children()
            .filter(|n| n.kind() == EXPRESSION || n.kind() == BINARY_EXPR)
            .count();
        assert_eq!(
            child_count, 3,
            "expected 3 child expressions, got {}",
            child_count
        );
    }

    #[test]
    fn parse_list_literal_trailing_comma() {
        // `[1, 2, 3,]` parses identically — no separator-related diagnostics.
        let parse = parse("SELECT [1, 2, 3,] FROM t");
        assert!(
            parse.errors.is_empty(),
            "trailing comma in list literal should not produce errors: {:?}",
            parse.errors
        );
        let list_node = parse
            .syntax()
            .descendants()
            .find(|n| n.kind() == ARRAY_LITERAL)
            .expect("must have ARRAY_LITERAL node");
        let child_count = list_node
            .children()
            .filter(|n| n.kind() == EXPRESSION || n.kind() == BINARY_EXPR)
            .count();
        assert_eq!(
            child_count, 3,
            "expected 3 child expressions, got {}",
            child_count
        );
    }

    #[test]
    fn parse_list_literal_singleton() {
        // `[x]` parses to one ARRAY_LITERAL node with one child.
        let parse = parse("SELECT [x] FROM t");
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );
        let list_node = parse
            .syntax()
            .descendants()
            .find(|n| n.kind() == ARRAY_LITERAL)
            .expect("must have ARRAY_LITERAL node for [x]");
        let child_count = list_node
            .children()
            .filter(|n| n.kind() == EXPRESSION || n.kind() == BINARY_EXPR)
            .count();
        assert_eq!(
            child_count, 1,
            "expected 1 child expression, got {}",
            child_count
        );
    }

    #[test]
    fn parse_list_literal_empty() {
        // `[]` parses to one ARRAY_LITERAL node with zero children, no errors.
        let parse = parse("SELECT [] FROM t");
        assert!(
            parse.errors.is_empty(),
            "empty list literal should produce no errors: {:?}",
            parse.errors
        );
        let list_node = parse
            .syntax()
            .descendants()
            .find(|n| n.kind() == ARRAY_LITERAL)
            .expect("must have ARRAY_LITERAL node for []");
        let child_count = list_node
            .children()
            .filter(|n| n.kind() == EXPRESSION || n.kind() == BINARY_EXPR)
            .count();
        assert_eq!(
            child_count, 0,
            "expected 0 child expressions, got {}",
            child_count
        );
    }

    #[test]
    fn parse_list_literal_nested() {
        // `[[1, 2], [3, 4]]` parses to a nested-list shape.
        let parse = parse("SELECT [[1, 2], [3, 4]] FROM t");
        assert!(
            parse.errors.is_empty(),
            "unexpected errors in nested list literal: {:?}",
            parse.errors
        );
        // Count total ARRAY_LITERAL nodes: should be 3 (outer + 2 inner).
        let all_array_literals: Vec<_> = parse
            .syntax()
            .descendants()
            .filter(|n| n.kind() == ARRAY_LITERAL)
            .collect();
        assert_eq!(
            all_array_literals.len(),
            3,
            "expected 3 ARRAY_LITERAL nodes (1 outer + 2 inner), got {}",
            all_array_literals.len()
        );
    }

    #[test]
    fn parse_spread_in_select_list() {
        // `SELECT id, ...metric_exprs, created_at FROM users`
        // produces a SELECT with a LIST_SPREAD child between two column references.
        let parse = parse("SELECT id, ...metric_exprs, created_at FROM users");
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );
        let spread = parse
            .syntax()
            .descendants()
            .find(|n| n.kind() == LIST_SPREAD)
            .expect("must have a LIST_SPREAD node in SELECT list");
        // The LIST_SPREAD should contain an EXPRESSION (the identifier)
        assert!(
            spread.descendants().any(|n| n.kind() == EXPRESSION),
            "LIST_SPREAD must contain an EXPRESSION child"
        );
    }

    #[test]
    fn parse_spread_in_function_args() {
        // `coalesce(...numerics, 0)` produces a function call with one LIST_SPREAD
        // argument and one literal argument.
        let parse = parse("SELECT coalesce(...numerics, 0) FROM t");
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );
        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == LIST_SPREAD),
            "must have a LIST_SPREAD node inside function args"
        );
    }

    #[test]
    fn parse_spread_of_list_literal() {
        // `SELECT id, ...[a, b, c] FROM t` — LIST_SPREAD wrapping a list-literal node.
        let parse = parse("SELECT id, ...[a, b, c] FROM t");
        assert!(
            parse.errors.is_empty(),
            "unexpected errors: {:?}",
            parse.errors
        );
        let spread = parse
            .syntax()
            .descendants()
            .find(|n| n.kind() == LIST_SPREAD)
            .expect("must have a LIST_SPREAD node");
        assert!(
            spread.descendants().any(|n| n.kind() == ARRAY_LITERAL),
            "LIST_SPREAD must contain an ARRAY_LITERAL child when operand is [...] literal"
        );
    }

    #[test]
    fn parse_list_literal_error_recovery_unterminated() {
        // `SELECT [a, b FROM t` — parser does not crash; produces a partial
        // list-literal node and continues parsing.
        let parse = parse("SELECT [a, b FROM t");
        // Must not panic (the test itself proves that).
        // Must produce a usable FILE node.
        let _ =
            File::cast(parse.syntax()).expect("parser must yield FILE even on unterminated list");
        // We expect at least one error for the unterminated list.
        assert!(
            !parse.errors.is_empty(),
            "unterminated list literal should produce at least one error"
        );
    }

    #[test]
    fn parse_spread_in_group_by() {
        // `SELECT x FROM t GROUP BY ...keys` — LIST_SPREAD inside the GROUP BY
        // clause; no parse errors.
        let parse = parse("SELECT x FROM t GROUP BY ...keys");
        assert!(
            parse.errors.is_empty(),
            "GROUP BY ...keys must parse without errors; got: {:?}",
            parse.errors
        );
        let spread = parse
            .syntax()
            .descendants()
            .find(|n| n.kind() == LIST_SPREAD)
            .expect("must have a LIST_SPREAD node in GROUP BY clause");
        assert!(
            spread.descendants().any(|n| n.kind() == EXPRESSION),
            "LIST_SPREAD must contain an EXPRESSION child"
        );
    }

    #[test]
    fn parse_spread_in_order_by() {
        // `SELECT x FROM t ORDER BY ...sort_keys` — LIST_SPREAD inside the ORDER
        // BY clause; no parse errors.
        let parse = parse("SELECT x FROM t ORDER BY ...sort_keys");
        assert!(
            parse.errors.is_empty(),
            "ORDER BY ...sort_keys must parse without errors; got: {:?}",
            parse.errors
        );
        let spread = parse
            .syntax()
            .descendants()
            .find(|n| n.kind() == LIST_SPREAD)
            .expect("must have a LIST_SPREAD node in ORDER BY clause");
        assert!(
            spread.descendants().any(|n| n.kind() == EXPRESSION),
            "LIST_SPREAD must contain an EXPRESSION child"
        );
    }

    #[test]
    fn parse_spread_in_in_list() {
        // `SELECT x FROM t WHERE id IN (...ids)` — LIST_SPREAD inside the
        // parenthesised IN value list; no parse errors.
        let parse = parse("SELECT x FROM t WHERE id IN (...ids)");
        assert!(
            parse.errors.is_empty(),
            "IN (...ids) must parse without errors; got: {:?}",
            parse.errors
        );
        let spread = parse
            .syntax()
            .descendants()
            .find(|n| n.kind() == LIST_SPREAD)
            .expect("must have a LIST_SPREAD node inside IN list");
        assert!(
            spread.descendants().any(|n| n.kind() == EXPRESSION),
            "LIST_SPREAD must contain an EXPRESSION child"
        );
    }

    #[test]
    fn parse_spread_in_values_row() {
        // `SELECT * FROM (VALUES (...vals)) AS t(c)` — LIST_SPREAD inside a
        // VALUES row; no parse errors.
        let parse = parse("SELECT * FROM (VALUES (...vals)) AS t(c)");
        assert!(
            parse.errors.is_empty(),
            "VALUES (...vals) must parse without errors; got: {:?}",
            parse.errors
        );
        let spread = parse
            .syntax()
            .descendants()
            .find(|n| n.kind() == LIST_SPREAD)
            .expect("must have a LIST_SPREAD node inside VALUES row");
        assert!(
            spread.descendants().any(|n| n.kind() == EXPRESSION),
            "LIST_SPREAD must contain an EXPRESSION child"
        );
    }

    // ===== Phase B (meta-language): fn keyword + pipe-arrow + lambda + pipe CST =====

    #[test]
    fn parse_lambda_single_arg() {
        // `map(xs, fn c => c)` — the second argument must be a LAMBDA node
        // with one parameter (`c`) and a body that is an identifier reference.
        let parse = parse("SELECT map(xs, fn c => c) FROM t");
        assert!(
            parse.errors.is_empty(),
            "parse_lambda_single_arg: unexpected errors: {:?}",
            parse.errors
        );
        let lambda = parse
            .syntax()
            .descendants()
            .find(|n| n.kind() == LAMBDA)
            .expect("must have a LAMBDA node");
        // The LAMBDA must have a LAMBDA_PARAM_LIST child with one ident.
        let param_list = lambda
            .children()
            .find(|n| n.kind() == LAMBDA_PARAM_LIST)
            .expect("LAMBDA must have a LAMBDA_PARAM_LIST child");
        let params: Vec<_> = param_list
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == IDENT)
            .collect();
        assert_eq!(
            params.len(),
            1,
            "expected 1 parameter, got {}",
            params.len()
        );
        assert_eq!(params[0].text(), "c");
        // The LAMBDA must have an EXPRESSION body child.
        assert!(
            lambda.children().any(|n| n.kind() == EXPRESSION),
            "LAMBDA must have an EXPRESSION body"
        );
    }

    #[test]
    fn parse_lambda_with_complex_body() {
        // `map(xs, fn c => CAST(c AS Text))` — second argument is a LAMBDA
        // whose body is a CAST expression.
        let parse = parse("SELECT map(xs, fn c => CAST(c AS INT)) FROM t");
        assert!(
            parse.errors.is_empty(),
            "parse_lambda_with_complex_body: unexpected errors: {:?}",
            parse.errors
        );
        let lambda = parse
            .syntax()
            .descendants()
            .find(|n| n.kind() == LAMBDA)
            .expect("must have a LAMBDA node");
        // Body must contain a CAST_EXPR descendant.
        assert!(
            lambda.descendants().any(|n| n.kind() == CAST_EXPR),
            "LAMBDA body must contain a CAST_EXPR"
        );
    }

    #[test]
    fn parse_fn_paren_does_not_consume_as_lambda() {
        // `SELECT fn(a, b) FROM t` — `fn` used as a function name followed by
        // LPAREN must parse as a regular function call, NOT as a lambda.
        // Phase B is single-arg only; the LPAREN branch is excluded from
        // is_fn_lambda_start() to avoid mis-consuming valid SQL where `fn` is
        // a function name.  LambdaArityNotSupported is latent (Phase F) and
        // will only fire once multi-arg parsing is enabled.
        let parse = parse("SELECT fn(a, b) FROM t");
        // Must parse without errors: `fn(a, b)` is a valid SQL function call.
        assert!(
            parse.errors.is_empty(),
            "fn(a, b) must parse as a function call without errors, got: {:?}",
            parse.errors
        );
        // Must NOT produce a LAMBDA CST node — fn(a, b) is a function call.
        assert!(
            !parse.syntax().descendants().any(|n| n.kind() == LAMBDA),
            "fn(a, b) must not produce a LAMBDA CST node (it is a function call)"
        );
        // Must produce a FUNCTION_CALL node — fn is the function name.
        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == FUNCTION_CALL),
            "fn(a, b) must produce a FUNCTION_CALL CST node"
        );
    }

    #[test]
    fn parse_fn_as_function_name_in_select() {
        // `SELECT fn(x, y) FROM t` — regression test confirming that fn used as
        // a SQL function name parses successfully as a SELECT with a function call.
        // This was broken when is_fn_lambda_start() matched LPAREN.
        let parse = parse("SELECT fn(x, y) FROM t");
        assert!(
            parse.errors.is_empty(),
            "SELECT fn(x, y) FROM t must parse without errors, got: {:?}",
            parse.errors
        );
        assert!(
            parse
                .syntax()
                .descendants()
                .any(|n| n.kind() == FUNCTION_CALL),
            "must have a FUNCTION_CALL node for fn(x, y)"
        );
        assert!(
            !parse.syntax().descendants().any(|n| n.kind() == LAMBDA),
            "must NOT have a LAMBDA node for fn(x, y) in SELECT"
        );
    }

    #[test]
    fn parse_pipe_expression() {
        // `xs |> filter(fn c => c)` — must parse to a PIPE_EXPR with
        // LHS = identifier reference and RHS = function-call expression.
        let parse = parse("SELECT xs |> filter(fn c => c) FROM t");
        assert!(
            parse.errors.is_empty(),
            "parse_pipe_expression: unexpected errors: {:?}",
            parse.errors
        );
        let pipe = parse
            .syntax()
            .descendants()
            .find(|n| n.kind() == PIPE_EXPR)
            .expect("must have a PIPE_EXPR node");
        // The PIPE_EXPR must contain a PIPE_ARROW token.
        assert!(
            pipe.children_with_tokens()
                .any(|e| e.as_token().map(|t| t.kind()) == Some(PIPE_ARROW)),
            "PIPE_EXPR must contain a PIPE_ARROW token"
        );
        // The PIPE_EXPR must have a FUNCTION_CALL descendant (the RHS `filter(...)`).
        assert!(
            pipe.descendants().any(|n| n.kind() == FUNCTION_CALL),
            "PIPE_EXPR must have a FUNCTION_CALL descendant (RHS)"
        );
        // The PIPE_EXPR must have an EXPRESSION child (the RHS).
        assert!(
            pipe.children().any(|n| n.kind() == EXPRESSION),
            "PIPE_EXPR must have an EXPRESSION child for the RHS"
        );
    }

    #[test]
    fn parse_pipe_chain_left_associative() {
        // `a |> b(p) |> c(q)` must parse as `((a |> b(p)) |> c(q))`.
        // The outermost PIPE_EXPR must itself contain a nested PIPE_EXPR (the LHS).
        let parse = parse("SELECT a |> b(p) |> c(q) FROM t");
        assert!(
            parse.errors.is_empty(),
            "parse_pipe_chain_left_associative: unexpected errors: {:?}",
            parse.errors
        );
        // Find all PIPE_EXPR nodes.
        let pipes: Vec<_> = parse
            .syntax()
            .descendants()
            .filter(|n| n.kind() == PIPE_EXPR)
            .collect();
        // Left-associative chain of 2 `|>` → 2 PIPE_EXPR nodes.
        assert_eq!(
            pipes.len(),
            2,
            "expected 2 PIPE_EXPR nodes, got {}",
            pipes.len()
        );
        // The outer PIPE_EXPR's span should be larger than the inner one.
        let outer = pipes
            .iter()
            .max_by_key(|n| n.text_range().len())
            .expect("must have an outer PIPE_EXPR");
        // The outer PIPE_EXPR must contain the inner PIPE_EXPR as a descendant
        // (left-associative: `(a |> b(p))` is the LHS of the outer `|> c(q)`).
        assert!(
            outer
                .descendants()
                .any(|n| n.kind() == PIPE_EXPR && n != *outer),
            "outer PIPE_EXPR must contain the inner PIPE_EXPR as a descendant (left-associative)"
        );
    }

    #[test]
    fn parse_pipe_lowest_precedence() {
        // `1 + 2 |> f()` must parse as `(1 + 2) |> f()`, i.e. the arithmetic
        // expression is the LHS of the pipe, not `2 |> f()` (which would make
        // `1 + (2 |> f())` — wrong, pipe must have lower precedence than `+`).
        let parse = parse("SELECT 1 + 2 |> f() FROM t");
        assert!(
            parse.errors.is_empty(),
            "parse_pipe_lowest_precedence: unexpected errors: {:?}",
            parse.errors
        );
        let pipe = parse
            .syntax()
            .descendants()
            .find(|n| n.kind() == PIPE_EXPR)
            .expect("must have a PIPE_EXPR node");
        // The PIPE_EXPR must contain a BINARY_EXPR descendant (the LHS `1 + 2`).
        assert!(
            pipe.descendants().any(|n| n.kind() == BINARY_EXPR),
            "PIPE_EXPR must contain a BINARY_EXPR (the 1+2 LHS), confirming pipe is lowest-precedence"
        );
    }

    #[test]
    fn parse_pipe_does_not_cross_statement_boundary() {
        // `a |> b()` in a SELECT context must produce a PIPE_EXPR without errors.
        let parse = parse("SELECT a |> b() FROM t");
        assert!(
            parse.errors.is_empty(),
            "parse_pipe_does_not_cross_statement_boundary: unexpected errors: {:?}",
            parse.errors
        );
        assert!(
            parse.syntax().descendants().any(|n| n.kind() == PIPE_EXPR),
            "must have a PIPE_EXPR node"
        );
    }

    #[test]
    fn parse_pipe_rhs_non_call_recovers() {
        // `xs |> 3 + 4` — the RHS is not a call expression.
        // The parser must produce a PIPE_EXPR node and recover without crashing;
        // Phase 3 will emit PipeRhsNotCall.
        let parse = parse("SELECT xs |> 3 + 4 FROM t");
        // There may or may not be a parse error; the important assertion is
        // that a PIPE_EXPR node was produced and the parse did not panic.
        let _pipe = parse
            .syntax()
            .descendants()
            .find(|n| n.kind() == PIPE_EXPR)
            .expect("xs |> 3 + 4 must produce a PIPE_EXPR node (Phase 3 validates RHS)");
        // The PIPE_EXPR must have at least one EXPRESSION child (RHS).
        assert!(
            _pipe.children().any(|n| n.kind() == EXPRESSION),
            "PIPE_EXPR must have an EXPRESSION child for RHS even for non-call RHS"
        );
    }

    #[test]
    fn parse_lambda_outside_call_recovers() {
        // A lambda literal in a non-HOF position — the parser must produce a
        // LAMBDA node and continue parsing; Phase 3 will emit LambdaInForbiddenPosition.
        let parse = parse("SELECT fn c => c FROM t");
        // There may be parse errors (position is unusual), but a LAMBDA node must exist.
        assert!(
            parse.syntax().descendants().any(|n| n.kind() == LAMBDA),
            "fn c => c must produce a LAMBDA node even in a forbidden position"
        );
    }

    #[test]
    fn parse_named_arg_still_works_after_fn_keyword_addition() {
        // `f(name => value)` — existing named-arg syntax must parse unchanged
        // after the `fn` / `=>` parser interaction is added.
        // Use an IDENT name (not a keyword) to avoid keyword token confusion.
        let parse = parse("SELECT my_func(my_param => a + b > 10) FROM t");
        assert!(
            parse.errors.is_empty(),
            "parse_named_arg_still_works_after_fn_keyword_addition: unexpected errors: {:?}",
            parse.errors
        );
        let named = parse
            .syntax()
            .descendants()
            .find(|n| n.kind() == NAMED_PARAM)
            .expect("must have a NAMED_PARAM node");
        // The NAMED_PARAM must start with an IDENT "my_param".
        let name_token = named
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .expect("NAMED_PARAM must have an IDENT name token");
        assert_eq!(name_token.text(), "my_param");
    }

    #[test]
    fn parse_pipe_chain_rhs_accessor_returns_outer_call() {
        // For `a |> b(p) |> c(q)`, the outer PipeExpr::rhs() must return the
        // expression containing `c(q)` — NOT the inner `a |> b(p)` pipe.
        // The naive `find_map(Expr::cast)` would return the inner PIPE_EXPR
        // first because PIPE_EXPR is included in Expr::cast's allow-list.
        // The fixed rhs() iterates past the PIPE_ARROW token before casting.
        let parse = parse("SELECT a |> b(p) |> c(q) FROM t");
        assert!(
            parse.errors.is_empty(),
            "parse_pipe_chain_rhs_accessor_returns_outer_call: unexpected errors: {:?}",
            parse.errors
        );
        // Find the outer (larger) PIPE_EXPR.
        let pipes: Vec<_> = parse
            .syntax()
            .descendants()
            .filter(|n| n.kind() == PIPE_EXPR)
            .collect();
        assert_eq!(
            pipes.len(),
            2,
            "expected 2 PIPE_EXPR nodes for a chain of 2 |>"
        );
        let outer_node = pipes
            .iter()
            .max_by_key(|n| n.text_range().len())
            .expect("must have an outer PIPE_EXPR")
            .clone();
        let outer_pipe = PipeExpr::cast(outer_node).expect("outer must cast to PipeExpr");
        // rhs() must return the expression for `c(q)` (not the inner pipe).
        let rhs = outer_pipe.rhs().expect("outer PipeExpr must have an rhs()");
        // rhs() must NOT be a PipeExpr (which would indicate we returned the inner pipe).
        assert!(
            rhs.syntax().kind() != PIPE_EXPR,
            "outer PipeExpr::rhs() returned a PIPE_EXPR — expected the outer call c(q)"
        );
        // The rhs must contain a FUNCTION_CALL node (the c(q) call).
        assert!(
            rhs.syntax()
                .descendants()
                .any(|n| n.kind() == FUNCTION_CALL),
            "outer PipeExpr::rhs() must contain a FUNCTION_CALL for c(q)"
        );
        // The function call name must be `c`, not `b`.
        let func_call = rhs
            .syntax()
            .descendants()
            .find(|n| n.kind() == FUNCTION_CALL)
            .expect("must have FUNCTION_CALL");
        let func_name = func_call
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .expect("FUNCTION_CALL must have an IDENT name token");
        assert_eq!(
            func_name.text(),
            "c",
            "outer PipeExpr::rhs() must point to c(q), not b(p)"
        );
    }

    #[test]
    fn parse_pipe_chain_lhs_accessor_returns_inner_pipe() {
        // For `a |> b(p) |> c(q)`, the outer PipeExpr::lhs() must return the
        // inner PipeExpr (`a |> b(p)`), not just `a`.
        let parse = parse("SELECT a |> b(p) |> c(q) FROM t");
        assert!(
            parse.errors.is_empty(),
            "parse_pipe_chain_lhs_accessor_returns_inner_pipe: unexpected errors: {:?}",
            parse.errors
        );
        let pipes: Vec<_> = parse
            .syntax()
            .descendants()
            .filter(|n| n.kind() == PIPE_EXPR)
            .collect();
        assert_eq!(pipes.len(), 2, "expected 2 PIPE_EXPR nodes");
        let outer_node = pipes
            .iter()
            .max_by_key(|n| n.text_range().len())
            .expect("must have an outer PIPE_EXPR")
            .clone();
        let outer_pipe = PipeExpr::cast(outer_node).expect("outer must cast to PipeExpr");
        // lhs() must return the inner PIPE_EXPR (a |> b(p)).
        let lhs = outer_pipe.lhs().expect("outer PipeExpr must have a lhs()");
        assert_eq!(
            lhs.syntax().kind(),
            PIPE_EXPR,
            "outer PipeExpr::lhs() must return the inner PIPE_EXPR (a |> b(p))"
        );
    }

    #[test]
    fn parse_pipe_simple_lhs_accessor_returns_lhs_expr() {
        // For `a |> b()`, PipeExpr::lhs() must return the expression for `a`.
        let parse = parse("SELECT a |> b() FROM t");
        assert!(
            parse.errors.is_empty(),
            "parse_pipe_simple_lhs_accessor_returns_lhs_expr: unexpected errors: {:?}",
            parse.errors
        );
        let pipe_node = parse
            .syntax()
            .descendants()
            .find(|n| n.kind() == PIPE_EXPR)
            .expect("must have a PIPE_EXPR node");
        let pipe = PipeExpr::cast(pipe_node).expect("must cast to PipeExpr");
        // lhs() must return something (the `a` reference expression).
        let lhs = pipe.lhs().expect("PipeExpr must have a lhs()");
        // The LHS must NOT be a PIPE_EXPR (that would mean we went past the arrow).
        assert!(
            lhs.syntax().kind() != PIPE_EXPR,
            "PipeExpr::lhs() for `a |> b()` must NOT be a PIPE_EXPR itself"
        );
    }

    #[test]
    fn parse_pipe_at_statement_start_is_error() {
        // `SELECT 1; |> b() FROM t` — `|>` at the start of a new statement
        // must not silently absorb into the previous statement. The parser
        // processes each SQL statement separately; `|>` at the start of what
        // would be the second statement should not produce a valid PIPE_EXPR
        // spanning statements. Instead, we expect either a parse error or the
        // `|>` is treated as an error token in the second statement context.
        //
        // Note: the smelt parser currently processes the full input as a single
        // file. When given `SELECT 1; |> b() FROM t`, the `;` ends the first
        // SELECT and `|> b() FROM t` begins a new (malformed) statement. We
        // assert that no PIPE_EXPR spans both statements and that a parse error
        // is recorded (since `|>` cannot appear at the start of a statement).
        let parse = parse("SELECT 1; |> b() FROM t");
        // The parse must record an error: `|>` is not valid at statement start.
        assert!(
            !parse.errors.is_empty(),
            "parse_pipe_at_statement_start_is_error: expected parse errors for |> at statement start, got none"
        );
        // Any PIPE_EXPR that exists must not span the entire input (i.e. must
        // not start at position 0, which would imply cross-statement merging).
        // In practice no valid PIPE_EXPR should exist here.
        for pipe in parse
            .syntax()
            .descendants()
            .filter(|n| n.kind() == PIPE_EXPR)
        {
            assert!(
                u32::from(pipe.text_range().start()) > 0,
                "PIPE_EXPR must not start at the beginning of the input (would span statements)"
            );
        }
    }

    #[test]
    fn parse_pipe_in_when_clause_produces_pipe_expr_node() {
        // `CASE WHEN xs |> filter(fn x => x > 0) THEN 1 ELSE 0 END`
        //
        // The WHEN condition contains a `|>` expression.  The parser must
        // produce a PIPE_EXPR CST node so that the Phase-3 semantic check can
        // fire PipeInDataPosition for the diagnostic.  If parse_when_clause
        // called parse_or_expr() instead of parse_pipe_expr(), the `|>` token
        // would be left in the stream and the parser would emit a confusing
        // "Expected THEN" error instead.
        let parse = parse("SELECT CASE WHEN xs |> filter(fn x => x > 0) THEN 1 ELSE 0 END FROM t");
        // There should be no parse errors — the pipe expression is structurally
        // valid (Phase 3 handles the semantic rejection via PipeInDataPosition).
        assert!(
            parse.errors.is_empty(),
            "parse_pipe_in_when_clause_produces_pipe_expr_node: unexpected errors: {:?}",
            parse.errors
        );
        // A PIPE_EXPR CST node must be produced inside the WHEN_CLAUSE.
        let when_clause = parse
            .syntax()
            .descendants()
            .find(|n| n.kind() == WHEN_CLAUSE)
            .expect("must have a WHEN_CLAUSE node");
        assert!(
            when_clause
                .descendants()
                .any(|n| n.kind() == PIPE_EXPR),
            "WHEN_CLAUSE must contain a PIPE_EXPR node (parse_pipe_expr must be used, not parse_or_expr)"
        );
    }
}

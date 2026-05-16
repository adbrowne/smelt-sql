//! `smelt.*` extension grammar productions.
//!
//! Covers:
//! - the top-level `parse_file` dispatcher,
//! - lookahead triggers for `smelt.define`, `smelt.record`, `smelt.extern`,
//!   `smelt.fn`, `smelt.<path>`, `smelt.ref`/`smelt.source` legacy, and
//!   `smelt.as_struct`,
//! - the declarations themselves (`smelt.define`, `smelt.extern`,
//!   `smelt.record`),
//! - the unified `smelt.<path>` value/call form and `PASSING` clauses,
//! - `parse_define_body` and `parse_brace_struct_literal`,
//! - the argument list dispatch (`parse_argument` + `is_named_parameter`).

use super::Parser;
use crate::SyntaxKind::*;

impl<'a> Parser<'a> {
    pub(super) fn parse_file(&mut self) {
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

            if self.at_smelt_record_trigger() {
                self.parse_smelt_record_decl();
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
    pub(super) fn at_smelt_define_trigger(&self) -> bool {
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
    /// the start of a top-level `smelt.record` declaration. Does not consume
    /// any tokens. The trigger is exactly three non-trivia tokens:
    ///   IDENT("smelt")  DOT  IDENT("record")
    pub(super) fn at_smelt_record_trigger(&self) -> bool {
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

        // Find the next non-trivia token: must be IDENT "record".
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
        text.eq_ignore_ascii_case("record")
    }

    /// Peek forward (skipping trivia) to check whether the current position is
    /// the start of a top-level `smelt.extern` declaration. Does not consume
    /// any tokens. The trigger is exactly three non-trivia tokens:
    ///   IDENT("smelt")  DOT  IDENT("extern")
    pub(super) fn at_smelt_extern_trigger(&self) -> bool {
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
    pub(super) fn at_smelt_fn_trigger(&self) -> bool {
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
        // Find the next non-trivia token: must be FN_KW (the reserved
        // `fn` keyword that introduces a lambda).
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
    pub(super) fn at_smelt_path_trigger(&self) -> bool {
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
            "record",
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
    pub(super) fn at_smelt_legacy_ref_or_source_trigger(&self) -> bool {
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
    pub(super) fn at_smelt_as_struct_trigger(&self) -> bool {
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
    pub(super) fn parse_smelt_as_struct(&mut self) {
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

    /// Parse a generator file body: skip all leading trivia (the frontmatter's
    /// comment-replacement lines), then parse a single meta-language expression.
    ///
    /// The entire content is wrapped in a `FILE` node so that callers receive
    /// the same `Parse` type as the regular `parse_file` path. Bare SQL keyword
    /// forms (`SELECT`, `WITH`, `VALUES`) at the first non-trivia token are
    /// detected before parsing begins; they are parsed as a best-effort
    /// `SELECT_STMT` for error recovery and the `bare_sql_at_body` field on the
    /// returned `Parse` is set so the diagnostic layer can anchor
    /// `GenerateFileBareSelectForbidden` at the correct span.
    ///
    /// `body_offset` is the byte offset into `self.input` where the body
    /// starts. All tokens before that offset are trivia (comment lines produced
    /// by frontmatter stripping) and are consumed without emitting errors.
    pub(super) fn parse_generator_body(&mut self, body_offset: usize) {
        self.start_node(FILE);

        // Consume all trivia tokens that lie entirely before `body_offset`.
        // `strip_frontmatter` replaces every frontmatter line with a `-- `
        // comment of the same byte length, so these are COMMENT tokens.
        loop {
            if self.at(EOF) {
                break;
            }
            // If the current token's start offset is at or past the body offset,
            // stop consuming prefix trivia.
            if self.offset >= body_offset {
                break;
            }
            self.advance();
        }

        // Skip any inline trivia (whitespace) after the body offset.
        self.skip_trivia();

        // Detect bare SQL forms at the body start.  These are not valid as
        // generator body expressions; parse them for error recovery and let
        // the caller emit the appropriate diagnostic.
        if self.at(SELECT_KW) || self.at(WITH_KW) || self.at(VALUES_KW) {
            // Parse as a SELECT statement for error-recovery purposes.
            // The caller checks the CST for SELECT_STMT presence to emit the
            // GenerateFileBareSelectForbidden diagnostic at the correct span.
            self.parse_select_stmt();
        } else if !self.at(EOF) {
            // Normal generator body: a single meta-language expression.
            self.parse_expression();
        }

        // Consume any trailing trivia or leftover tokens.
        while !self.at(EOF) {
            self.advance();
        }

        self.finish_node(); // FILE
    }

    /// Sync forward to EOF or the start of the next top-level declaration.
    /// Anything skipped is wrapped in ERROR nodes (one per token).
    pub(super) fn sync_to_top_level(&mut self) {
        while !self.at(EOF) {
            // Skip trivia without emitting ERROR so the tree stays sensible.
            if self.current().is_trivia() {
                self.advance();
                continue;
            }
            if self.at_smelt_define_trigger()
                || self.at_smelt_extern_trigger()
                || self.at_smelt_record_trigger()
            {
                return;
            }
            self.start_node(ERROR);
            self.advance();
            self.finish_node();
        }
    }

    /// Parse a top-level `smelt.define` declaration. The caller must have
    /// verified `at_smelt_define_trigger()` first.
    pub(super) fn parse_smelt_define(&mut self) {
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
    pub(super) fn parse_smelt_extern(&mut self) {
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

    /// Parse a top-level `smelt.record Name = { field: Type, ... }` declaration.
    /// The caller must have verified `at_smelt_record_trigger()` first.
    ///
    /// Grammar:
    ///   smelt.record NAME = { RECORD_FIELD (, RECORD_FIELD)* ,? }
    ///
    /// Each RECORD_FIELD is:
    ///   IDENT : TYPE_REF
    /// where the TYPE_REF may itself be an inline record type `{ ... }` (RECORD_TYPE_INLINE).
    pub(super) fn parse_smelt_record_decl(&mut self) {
        self.start_node(SMELT_RECORD_DECL);

        // Consume `smelt . record`
        self.skip_trivia();
        self.advance(); // IDENT "smelt"
        self.skip_trivia();
        self.advance(); // DOT
        self.skip_trivia();
        self.advance(); // IDENT "record"

        // NAME identifier
        self.skip_trivia();
        if self.at(IDENT) {
            self.advance(); // Name token (SourceEntry, Cohort, etc.)
        } else {
            self.error("Expected record type name after smelt.record".to_string());
            self.sync_to(&[EQ, LBRACE, EOF]);
        }

        // `=` separator
        self.skip_trivia();
        if self.at(EQ) {
            self.advance(); // `=`
        } else {
            self.error("Expected '=' after record type name in smelt.record".to_string());
        }

        // Body: `{ field: Type, ... }` — wrapped as RECORD_TYPE_INLINE so that
        // the body node has the same kind as an inline-record type annotation.
        self.skip_trivia();
        if self.at(LBRACE) {
            self.parse_record_type_inline();
        } else {
            self.error("Expected '{' to start record type body".to_string());
        }

        self.finish_node(); // SMELT_RECORD_DECL
    }

    /// Parse a brace-struct literal: `{expr AS name, ..spread}`.
    ///
    /// Items are:
    ///   - `STRUCT_FIELD_ITEM`: `expr AS alias`
    ///   - `SPREAD_ITEM`: `..ident`
    pub(super) fn parse_brace_struct_literal(&mut self) {
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
    pub(super) fn parse_define_body(&mut self) {
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
    pub(super) fn parse_smelt_path_form(&mut self) {
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
            // Allow IDENT segments and also ALL_KW (`all` is a reserved SQL
            // keyword but a valid smelt path segment in `smelt.models.all`).
            if after_dot != IDENT && after_dot != ALL_KW {
                break;
            }
            self.skip_trivia();
            self.advance(); // DOT
            self.skip_trivia();
            self.advance(); // IDENT or ALL_KW
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
            let prev = self.in_smelt_call_args;
            self.in_smelt_call_args = true;
            self.parse_arg_list();
            self.in_smelt_call_args = prev;

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
    pub(super) fn parse_passing_clause(&mut self) {
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

    pub(super) fn parse_argument(&mut self) {
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
            // Phase B/F meta-language lambda: `fn IDENT => body` (single-arg)
            // or `fn (IDENT, ...) => body` (multi-arg, Phase F).
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
        } else if self.in_smelt_call_args && self.at(IDENT) && self.is_generic_type_start() {
            // Generic type expression in a smelt-call argument position:
            // `List<Cohort>`, `Map<Text, {field: Type}>`, etc. Only routed
            // here when inside a `smelt.<path>(...)` arg list — regular SQL
            // function calls keep `IDENT < IDENT` as a comparison expression.
            self.parse_record_field_type_ref();
        } else {
            // Regular expression argument - parse as full expression
            // This handles: identifiers, literals, function calls, binary expressions, etc.
            self.parse_expression();
        }
    }

    /// Check if current position starts a named parameter (IDENT => ...)
    /// Uses lookahead without consuming tokens
    pub(super) fn is_named_parameter(&self) -> bool {
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
}

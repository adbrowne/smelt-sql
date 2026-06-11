//! Type-reference and type-related grammar productions.
//!
//! Covers record bodies/types/literals, struct types, parameter lists, the
//! `Expr<T>` / `AggExpr<T>` / `WindowExpr<T>` / `TableExpr<{...}>` /
//! `SelectItems<...>` sort dispatch, and the lookahead helpers used by the
//! type-ref consumer.

use super::is_selectitems_kind_name;
use super::Parser;
use crate::syntax_kind::SyntaxKind;
use crate::SyntaxKind::*;

impl<'a> Parser<'a> {
    pub(super) fn parse_record_body_as_type(&mut self) {
        self.advance(); // consume `{`

        loop {
            self.skip_trivia();
            if self.at(RBRACE) || self.at(EOF) {
                break;
            }

            // We must see IDENT here for a field name.
            if !self.at(IDENT) {
                self.error("Expected field name in record type".to_string());
                // Recover: skip to the next COMMA or RBRACE.
                self.sync_to(&[COMMA, RBRACE, EOF]);
                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                }
                continue;
            }

            // RECORD_FIELD: IDENT COLON TYPE_REF
            self.start_node(RECORD_FIELD);
            self.advance(); // IDENT (field name)
            self.skip_trivia();

            if self.at(COLON) {
                self.advance(); // COLON
                self.skip_trivia();
                // If next token is `{`, parse as RECORD_TYPE_INLINE.
                // Otherwise parse as a flat type ref.
                if self.at(LBRACE) {
                    self.parse_record_type_inline();
                } else if !self.at_any(&[COMMA, RBRACE, EOF]) {
                    // Normal type ref: flat type name like Text, Integer, List<Text>, etc.
                    self.parse_record_field_type_ref();
                } else {
                    // Missing type — emit error token and let recovery proceed.
                    self.error("Expected type after ':' in record field".to_string());
                }
            } else {
                self.error("Expected ':' after field name in record type".to_string());
                // Try to recover at the type position by looking for COMMA / RBRACE
                self.sync_to(&[COMMA, RBRACE, EOF]);
            }

            self.finish_node(); // RECORD_FIELD

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance(); // COMMA
                self.skip_trivia();
                // Trailing comma is allowed.
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
            self.error("Expected '}' to close record type body".to_string());
        }
    }

    /// Parse an inline record type `{ field: Type, ... }` as a RECORD_TYPE_INLINE node.
    /// The caller must have verified `self.at(LBRACE)` first.
    pub(super) fn parse_record_type_inline(&mut self) {
        self.start_node(RECORD_TYPE_INLINE);
        if self.too_deep() {
            // Bail out — emit an empty RECORD_TYPE_INLINE and skip the body.
            // Recovery: advance past the `{`, then sync to the next sane boundary
            // so the outer parser can continue.
            self.advance(); // consume `{`
            self.sync_to(&[RBRACE, COMMA, EOF]);
            if self.at(RBRACE) {
                self.advance();
            }
            self.finish_node();
            return;
        }
        self.depth += 1;
        self.parse_record_body_as_type();
        self.depth -= 1;
        self.finish_node(); // RECORD_TYPE_INLINE
    }

    /// Parse a type reference for use inside a record field (either inline or
    /// in a smelt.record body). Stops at depth-0 COMMA, RBRACE, GT, EQ, AS, `->`, or EOF.
    /// Handles generic types like `List<Text>`, `Map<Text, Integer>`, etc.
    ///
    /// When `{` is encountered inside `<...>` angle brackets, it is parsed as
    /// a `RECORD_TYPE_INLINE` node so that `List<{ name: Text }>` produces the
    /// correct structured CST.
    pub(super) fn parse_record_field_type_ref(&mut self) {
        self.start_node(TYPE_REF);
        // Consume the type until we hit a depth-0 boundary.
        // Track angle bracket depth to handle List<Text>, Map<K, V>, etc.
        let mut angle_depth: i32 = 0;
        loop {
            self.skip_trivia();
            let k = self.current();
            if k == EOF {
                break;
            }
            if angle_depth == 0
                && matches!(k, COMMA | RBRACE | EQ | AS_KW | JSON_ARROW | RPAREN | GT)
            {
                break;
            }
            // When inside angle brackets and we see `{`, parse it as an inline
            // record type so the RECORD_TYPE_INLINE node is produced for
            // List<{ name: Text }> etc.
            if angle_depth > 0 && k == LBRACE {
                self.parse_record_type_inline();
                self.skip_trivia();
                continue;
            }
            match k {
                LT => angle_depth += 1,
                GT => angle_depth = angle_depth.saturating_sub(1),
                _ => {}
            }
            self.advance();
        }
        self.finish_node(); // TYPE_REF
    }

    /// Dispatcher for type annotations in parameter position. Handles:
    /// 1. Inline record types: `{field: Type, ...}` → `RECORD_TYPE_INLINE`
    /// 2. Known expression sorts: `Expr<T>`, `AggExpr<T>`, `WindowExpr<T>`,
    ///    `TableExpr`, `SelectItems<...>` → full `parse_type_ref` (validates sort head,
    ///    emits error for unknown sorts like `FooExpr`)
    /// 3. Other type heads: `Map<K,V>`, `List<T>`, named types like `Cohort`,
    ///    plain types like `Integer`, `Text` → flat `parse_record_field_type_ref`
    pub(super) fn parse_param_type_annotation(&mut self) {
        if self.at(LBRACE) {
            // Inline record type in annotation position.
            self.parse_record_type_inline();
        } else if self.at(IDENT) {
            // Check if the head looks like a known expression sort head.
            // These go through the full parse_type_ref which validates them.
            let head = self.current_text().to_string();
            if matches!(
                head.as_str(),
                "Expr"
                    | "AggExpr"
                    | "WindowExpr"
                    | "TableExpr"
                    | "SelectItems"
                    | "FooExpr"
                    | "BarExpr" // pattern: anything ending in "Expr" uses full validator
            ) || head.ends_with("Expr")
            {
                self.parse_type_ref();
            } else {
                // Plain/generic type: use flat consumer — no sort validation, no errors.
                self.parse_record_field_type_ref();
            }
        } else {
            // Nothing recognizable — fall back to full type_ref for error reporting.
            self.parse_type_ref();
        }
    }

    /// Parse a record literal `{key: value, key2: value2}` in a VALUE position.
    /// Produces a RECORD_LITERAL node with RECORD_FIELD children.
    ///
    /// Each field is `IDENT : EXPRESSION`.
    /// Error recovery: missing value → advance to COMMA/RBRACE sync point.
    pub(super) fn parse_record_literal(&mut self) {
        self.start_node(RECORD_LITERAL);
        self.parse_record_literal_body();
        self.finish_node(); // RECORD_LITERAL
    }

    /// Parse the body of a record literal starting at `{`, including the closing `}`.
    ///
    /// This is the shared inner implementation used by both the anonymous `{…}`
    /// form (`parse_record_literal`) and the named `TypeName {…}` form (where
    /// the caller has already started a RECORD_LITERAL node via a checkpoint
    /// and consumed the leading IDENT).
    ///
    /// Precondition: `self.at(LBRACE)`.
    pub(super) fn parse_record_literal_body(&mut self) {
        self.advance(); // consume `{`

        loop {
            self.skip_trivia();
            if self.at(RBRACE) || self.at(EOF) {
                break;
            }

            // Field name must be IDENT.
            if !self.at(IDENT) {
                self.error("Expected field name in record literal".to_string());
                self.sync_to(&[COMMA, RBRACE, EOF]);
                self.skip_trivia();
                if self.at(COMMA) {
                    self.advance();
                }
                continue;
            }

            self.start_node(RECORD_FIELD);
            self.advance(); // IDENT (key)
            self.skip_trivia();

            if self.at(COLON) {
                self.advance(); // COLON
                self.skip_trivia();
                // Parse the value expression. When the field value starts with
                // a SQL statement keyword (SELECT, WITH, VALUES), parse it as a
                // SQL statement rather than a meta-language expression. This
                // allows record fields like `body: SELECT * FROM orders` to
                // parse without errors inside generator-file record literals.
                if self.at(SELECT_KW) || self.at(WITH_KW) {
                    self.parse_select_stmt();
                } else if self.at(VALUES_KW) {
                    self.parse_values_clause();
                } else if !self.at_any(&[COMMA, RBRACE, EOF]) {
                    self.parse_expression();
                } else {
                    // Missing value — emit an error token for recovery.
                    self.error(
                        "Expected expression value after ':' in record literal field".to_string(),
                    );
                }
            } else {
                self.error("Expected ':' after field name in record literal".to_string());
                self.sync_to(&[COMMA, RBRACE, EOF]);
            }

            self.finish_node(); // RECORD_FIELD

            self.skip_trivia();
            if self.at(COMMA) {
                self.advance(); // COMMA
                self.skip_trivia();
                // Trailing comma is allowed.
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
            self.error("Expected '}' to close record literal".to_string());
        }
    }

    /// Returns true when the current `{` starts a record literal rather than a
    /// brace-struct literal. A record literal uses `IDENT : expr` fields,
    /// while a brace-struct uses `expr AS alias` fields.
    ///
    /// Peek after `{` (skipping trivia): if we see `IDENT COLON` (and the IDENT
    /// is NOT followed by `AS`), treat as a record literal. Otherwise treat as a
    /// brace-struct literal (legacy Phase 35 behaviour).
    ///
    /// An empty `{}` is treated as a record literal (no fields).
    pub(super) fn is_record_literal_start(&self) -> bool {
        // Must be at LBRACE.
        debug_assert!(self.at(LBRACE));
        // Peek past the `{` to find the next non-trivia token.
        let mut i = 1; // start after `{`
        while let Some(t) = self.tokens.get(self.pos + i) {
            if t.kind.is_trivia() {
                i += 1;
                continue;
            }
            if t.kind == RBRACE {
                // Empty `{}` — treat as record literal.
                return true;
            }
            if t.kind != IDENT {
                // Not IDENT — must be a brace-struct (spread `..`, etc.).
                return false;
            }
            // We found an IDENT. Look at the next non-trivia token.
            let mut j = i + 1;
            while let Some(t2) = self.tokens.get(self.pos + j) {
                if t2.kind.is_trivia() {
                    j += 1;
                    continue;
                }
                // If next non-trivia is COLON → record literal.
                return t2.kind == COLON;
            }
            return false;
        }
        false
    }

    /// Parse the parenthesized parameter list of a smelt.define.
    pub(super) fn parse_param_list(&mut self) {
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
    pub(super) fn parse_param(&mut self) {
        self.start_node(PARAM);

        // Parameter name (required, we've already checked for IDENT).
        self.advance(); // consume IDENT

        // Optional `: TypeRef`.
        self.skip_trivia();
        if self.at(COLON) {
            self.advance();
            self.skip_trivia();
            self.parse_param_type_annotation();
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
    pub(super) fn parse_type_ref(&mut self) {
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
            //
            // Phase 5 (nullability-soundness): `NOT NULL` qualifiers are
            // NOT consumed inside `parse_type_ref`; they are left for the
            // enclosing `PARAM` or `ROW_FIELD` context to consume as a
            // sibling of the `TYPE_REF` node. This keeps `type_ref.text()`
            // clean (no trailing "NOT NULL") and the string-level
            // `parse_smelt_type` unaffected.
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
    pub(super) fn emit_expr_kind_marker(&mut self, kind: SyntaxKind) {
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
    ///
    /// Also stops at a depth-0 `NOT_KW` so that `NOT NULL` qualifiers
    /// (which appear after the flat type name) are not consumed into the
    /// TYPE_REF text. The caller is responsible for consuming any trailing
    /// `NOT NULL` qualifier (or rejecting it for struct fields).
    pub(super) fn parse_flat_type_ref_stopping_on_row_field_boundary(&mut self) {
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
                // Stop before `NOT NULL` so the caller can handle it.
                if k == NOT_KW && matches!(self.peek_next_non_trivia(), Some(NULL_KW)) {
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

    /// Emit a zero-width `NOT_NULL_QUALIFIER` marker node (Phase 5,
    /// nullability-soundness). The marker contains no tokens — it serves
    /// only as a structured presence indicator that `TypeRef::not_null()`
    /// can find without reading token text.
    ///
    /// Used inside `parse_expr_tail` to mark `Expr<T NOT NULL>` before
    /// consuming the actual `NOT` / `NULL` tokens.
    pub(super) fn emit_not_null_qualifier_marker(&mut self) {
        self.start_node(NOT_NULL_QUALIFIER);
        self.finish_node();
    }

    /// If the current token is `NOT` and the next non-trivia token is `NULL`,
    /// consume both and emit a `NOT_NULL_QUALIFIER` child node (Phase 5
    /// nullability-soundness). Returns `true` when the qualifier was consumed.
    ///
    /// Used outside `TYPE_REF` (in `PARAM` or `ROW_FIELD` context) to place
    /// the qualifier as a sibling of the `TYPE_REF` node. This variant puts
    /// the actual `NOT` and `NULL` tokens inside the qualifier node.
    pub(super) fn try_consume_not_null_qualifier(&mut self) -> bool {
        self.skip_trivia();
        if self.at(NOT_KW) && matches!(self.peek_next_non_trivia(), Some(NULL_KW)) {
            self.start_node(NOT_NULL_QUALIFIER);
            self.advance(); // NOT_KW
            self.skip_trivia();
            self.advance(); // NULL_KW
            self.finish_node();
            true
        } else {
            false
        }
    }

    /// Flat-consume tokens up to a depth-0 boundary. `<` and `(` open
    /// angle/paren depth so commas inside don't end the TypeRef.
    pub(super) fn consume_type_ref_tail(&mut self) {
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
    pub(super) fn parse_tableexpr_tail(&mut self) {
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
            //
            // Phase 5 (nullability-soundness): after the flat type ref,
            // optionally consume `NOT NULL` into a NOT_NULL_QUALIFIER child
            // of the ROW_FIELD node (e.g. `id: Integer NOT NULL`).
            if self.at(IDENT) {
                self.start_node(ROW_FIELD);
                self.advance(); // IDENT name
                self.skip_trivia();
                if self.at(COLON) {
                    self.advance();
                    self.skip_trivia();
                    self.parse_flat_type_ref_stopping_on_row_field_boundary();
                    // Optionally consume `NOT NULL` qualifier on the row field.
                    self.try_consume_not_null_qualifier();
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
    pub(super) fn parse_selectitems_tail(&mut self) {
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
    pub(super) fn parse_expr_tail(&mut self) {
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
        //
        // Phase 5 (nullability-soundness): `Expr<T NOT NULL>` is the surface
        // syntax for a non-nullable expression parameter. When `NOT NULL`
        // appears before the closing `>` at depth 0, we emit a **zero-width**
        // `NOT_NULL_QUALIFIER` marker (no child tokens) BEFORE consuming the
        // `NOT` / `NULL` / `>` tokens. This lets `TypeRef::not_null()` detect
        // the qualifier as a child of `TYPE_REF` without needing to parse the
        // raw text. The tokens still become children of the TYPE_REF (so the
        // raw text includes "NOT NULL"), but the CST marker is the authoritative
        // detection signal used by `extract_param_spec`.
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
                // Phase 5: `NOT NULL` inside `Expr<T NOT NULL>` — emit a
                // zero-width NOT_NULL_QUALIFIER marker (no tokens), then
                // consume the NOT / NULL / closing `>` tokens.
                if k == NOT_KW && matches!(self.peek_next_non_trivia(), Some(NULL_KW)) {
                    self.emit_not_null_qualifier_marker();
                    // Consume NOT, NULL, and the closing >.
                    self.advance(); // NOT_KW
                    self.skip_trivia();
                    self.advance(); // NULL_KW
                    self.skip_trivia();
                    if self.at(GT) {
                        self.advance();
                    } else {
                        self.skip_to_matching_gt();
                    }
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
    pub(super) fn peek_has_second_selectitems_arg(&self) -> bool {
        matches!(self.peek_nth_non_trivia(1), Some(k) if k == COMMA)
    }

    /// Peek the kind of the Nth non-trivia token after the current one
    /// (0 == current non-trivia; 1 == the one after, etc.).
    pub(super) fn peek_nth_non_trivia(&self, n: usize) -> Option<SyntaxKind> {
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
    pub(super) fn peek_next_non_trivia(&self) -> Option<SyntaxKind> {
        self.peek_nth_non_trivia(1)
    }

    /// Consume tokens until a `>` at angle-depth 0 is found (and
    /// consumed), or a type-ref boundary / EOF is reached.
    pub(super) fn skip_to_matching_gt(&mut self) {
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
    pub(super) fn is_struct_type_start(&self) -> bool {
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
    pub(super) fn parse_struct_type(&mut self) {
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
                    // Phase 5 (nullability-soundness): `NOT NULL` is not accepted
                    // on struct field types — emit a parse error.
                    self.skip_trivia();
                    if self.at(NOT_KW) && matches!(self.peek_next_non_trivia(), Some(NULL_KW)) {
                        self.error(
                            "`NOT NULL` is not accepted on struct field types; nullability is only tracked at the top-level Expr/TableExpr parameter position".to_string(),
                        );
                        self.advance(); // NOT_KW
                        self.skip_trivia();
                        self.advance(); // NULL_KW
                    }
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
}

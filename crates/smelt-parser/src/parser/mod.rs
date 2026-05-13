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

    // Domain-specific parsing methods live in submodules:
    //   - parser::smelt_ext (smelt.* extensions)
    //   - parser::types     (type refs, records, struct types)
    //   - parser::select    (SELECT, FROM, WHERE, ...)
    //   - parser::expr      (expression precedence, primaries, ...)
    //   - parser::meta      (meta-language lambdas)
}

mod expr;
mod meta;
mod select;
mod smelt_ext;
mod types;

#[cfg(test)]
mod tests;

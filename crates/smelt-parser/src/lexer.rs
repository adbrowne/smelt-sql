/// Lexer for smelt SQL files with template expressions
use crate::syntax_kind::SyntaxKind;
use crate::SyntaxKind::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: SyntaxKind,
    pub len: usize,
}

/// Tokenize input text into a stream of tokens
pub fn tokenize(input: &str) -> Vec<Token> {
    let mut lexer = Lexer::new(input);
    let mut tokens = Vec::new();

    loop {
        let token = lexer.next_token();
        if token.kind == EOF {
            break;
        }
        tokens.push(token);
    }

    tokens
}

struct Lexer<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn next_token(&mut self) -> Token {
        if self.pos >= self.input.len() {
            return Token { kind: EOF, len: 0 };
        }

        let start = self.pos;
        let c = self.current_char();

        let kind = match c {
            // Whitespace
            c if c.is_whitespace() => self.consume_whitespace(),

            // Comments
            '-' if self.peek_char() == Some('-') => self.consume_comment(),

            // Operators & punctuation
            '(' => {
                self.advance();
                LPAREN
            }
            ')' => {
                self.advance();
                RPAREN
            }
            ',' => {
                self.advance();
                COMMA
            }
            '.' if self.peek_two_chars() == Some(('.', '.')) => {
                // `...` — list-spread operator (meta-language)
                self.advance();
                self.advance();
                self.advance();
                DOT_DOT_DOT
            }
            '.' if self.peek_char() == Some('.') => {
                // `..` — struct/row spread (two-dot)
                self.advance();
                self.advance();
                DOT_DOT
            }
            '.' => {
                self.advance();
                DOT
            }
            '*' => {
                self.advance();
                STAR
            }
            '+' => {
                self.advance();
                PLUS
            }
            '-' if self.peek_char() == Some('>') => {
                self.advance();
                self.advance();
                if self.current_char() == '>' {
                    self.advance();
                    JSON_ARROW_TEXT // ->>
                } else {
                    JSON_ARROW // ->
                }
            }
            '-' => {
                self.advance();
                MINUS
            }
            '%' => {
                self.advance();
                PERCENT
            }
            '/' if self.peek_char() == Some('*') => self.consume_block_comment(),
            '/' => {
                self.advance();
                DIVIDE
            }
            '=' if self.peek_char() == Some('>') => {
                self.advance();
                self.advance();
                ARROW
            }
            '=' => {
                self.advance();
                EQ
            }
            '!' if self.peek_char() == Some('~') => {
                self.advance(); // !
                self.advance(); // ~
                if self.current_char() == '*' {
                    self.advance();
                    NOT_TILDE_STAR // !~*
                } else {
                    NOT_TILDE // !~
                }
            }
            '!' if self.peek_char() == Some('=') => {
                self.advance();
                self.advance();
                NE
            }
            '<' if self.peek_char() == Some('@') => {
                self.advance();
                self.advance();
                LT_AT
            }
            '<' if self.peek_char() == Some('>') => {
                self.advance();
                self.advance();
                NE // <> is not-equal, same as !=
            }
            '<' if self.peek_char() == Some('=') => {
                self.advance();
                self.advance();
                LE
            }
            '<' => {
                self.advance();
                LT
            }
            '>' if self.peek_char() == Some('=') => {
                self.advance();
                self.advance();
                GE
            }
            '>' => {
                self.advance();
                GT
            }
            ':' if self.peek_char() == Some(':') => {
                self.advance();
                self.advance();
                DOUBLE_COLON
            }
            // Lex `||` (SQL string concat) before `|>` (meta-language pipe).
            // The order here is load-bearing: `||` must be checked first so
            // SQL string concatenation is never mis-tokenised as `|` `>`.
            '|' if self.peek_char() == Some('|') => {
                self.advance();
                self.advance();
                CONCAT
            }
            '|' if self.peek_char() == Some('>') => {
                self.advance();
                self.advance();
                PIPE_ARROW
            }
            '|' => {
                self.advance();
                ERROR
            }
            '[' => {
                self.advance();
                LBRACKET
            }
            ']' => {
                self.advance();
                RBRACKET
            }
            '{' => {
                self.advance();
                LBRACE
            }
            '}' => {
                self.advance();
                RBRACE
            }
            // DuckDB "walrus" named-argument operator: `name := value`.
            // Must be checked before the bare-COLON fallback below.
            ':' if self.peek_char() == Some('=') => {
                self.advance();
                self.advance();
                WALRUS
            }
            ':' if self.peek_char() != Some(':') => {
                self.advance();
                COLON
            }

            // Hash operators (#>, #>>) and bare `#` (CTE-reference operator).
            '#' if self.peek_char() == Some('>') => {
                self.advance(); // #
                self.advance(); // >
                if self.current_char() == '>' {
                    self.advance();
                    HASH_ARROW_TEXT // #>>
                } else {
                    HASH_ARROW // #>
                }
            }
            // Bare `#` — used in `smelt.<path>#<cte>` CTE references inside
            // smelt.test bodies.  This arm fires only when the next char is NOT
            // `>` (the #> / #>> arm above catches that case first).
            '#' => {
                self.advance();
                HASH
            }
            // Containment operator (@>)
            '@' if self.peek_char() == Some('>') => {
                self.advance();
                self.advance();
                AT_GT
            }
            // Regex match operators (~, ~*)
            '~' => {
                self.advance();
                if self.current_char() == '*' {
                    self.advance();
                    TILDE_STAR // ~*
                } else {
                    TILDE // ~
                }
            }
            // Strings
            '\'' | '"' => self.consume_string(c),

            // Prefixed string literals: `E'...'` (escape string) and
            // `B'...'` (bit-string-shaped literal). DuckDB accepts both as
            // string literals with no parse error (the `E` prefix additionally
            // triggers backslash-escape processing of the body on DuckDB's
            // side; smelt does not need to replicate that semantics here,
            // only avoid silently splitting the prefix letter from the string
            // that follows it into an orphan IDENT + STRING pair). This arm
            // must come before the generic identifier arm below, and only
            // fires when the quote immediately follows the single prefix
            // letter (no space) — `Extra'foo'` still lexes as IDENT `Extra`
            // followed by STRING, since `peek_char` there is `x`, not `'`.
            c if (c == 'E' || c == 'e' || c == 'B' || c == 'b')
                && self.peek_char() == Some('\'') =>
            {
                self.advance(); // consume the E/B prefix letter
                self.consume_string('\'')
            }

            // Dollar-quoted string literals: `$$...$$` or `$tag$...$tag$`
            // (PostgreSQL/DuckDB syntax). This arm only fires when `$` actually
            // opens a valid dollar-quote delimiter (`$$` or `$tag$` where tag is
            // `[A-Za-z_][A-Za-z0-9_]*`); otherwise it falls through to the
            // "unknown character" fallback below, preserving prior behaviour for
            // other `$`-prefixed text (e.g. `$1`, `$name` as prepared-statement
            // parameter markers — not implemented in smelt today; a bare `$`
            // there still tokenizes as a single ERROR followed by the trailing
            // text lexed normally, exactly as before this change).
            '$' => match self.try_dollar_quote() {
                Some(kind) => kind,
                None => {
                    self.advance();
                    ERROR
                }
            },

            // Numbers
            c if c.is_ascii_digit() => self.consume_number(),

            // Identifiers and keywords
            c if c.is_alphabetic() || c == '_' => self.consume_ident_or_keyword(),

            // Unknown character
            _ => {
                self.advance();
                ERROR
            }
        };

        Token {
            kind,
            len: self.pos - start,
        }
    }

    fn current_char(&self) -> char {
        self.input[self.pos..].chars().next().unwrap_or('\0')
    }

    fn peek_char(&self) -> Option<char> {
        self.input[self.pos..].chars().nth(1)
    }

    /// Peek at the two characters *after* the current one (chars 1 and 2
    /// relative to `self.pos`, i.e. what follows `peek_char`).
    fn peek_two_chars(&self) -> Option<(char, char)> {
        let mut chars = self.input[self.pos..].chars();
        let _current = chars.next()?;
        let second = chars.next()?;
        let third = chars.next()?;
        Some((second, third))
    }

    fn advance(&mut self) {
        if self.pos < self.input.len() {
            self.pos += self.current_char().len_utf8();
        }
    }

    fn consume_whitespace(&mut self) -> SyntaxKind {
        while self.current_char().is_whitespace() {
            self.advance();
        }
        WHITESPACE
    }

    fn consume_comment(&mut self) -> SyntaxKind {
        // Consume --
        self.advance();
        self.advance();

        // Consume until newline or EOF
        while self.current_char() != '\n' && self.current_char() != '\0' {
            self.advance();
        }

        COMMENT
    }

    fn consume_block_comment(&mut self) -> SyntaxKind {
        // Consume /*
        self.advance();
        self.advance();

        let mut depth = 1;
        while depth > 0 && self.current_char() != '\0' {
            if self.current_char() == '/' && self.peek_char() == Some('*') {
                self.advance();
                self.advance();
                depth += 1;
            } else if self.current_char() == '*' && self.peek_char() == Some('/') {
                self.advance();
                self.advance();
                depth -= 1;
            } else {
                self.advance();
            }
        }

        COMMENT
    }

    fn consume_string(&mut self, quote: char) -> SyntaxKind {
        // Consume opening quote
        self.advance();

        // Consume until an unmatched closing quote or EOF.
        //
        // Standard SQL: a doubled quote inside the string (`''` inside a
        // single-quoted string, or `""` inside a double-quoted identifier) is
        // an escape for a literal quote character — both characters are
        // consumed as part of the string and scanning continues. Only a
        // single, unmatched quote terminates.
        loop {
            let c = self.current_char();
            if c == '\0' {
                // EOF without closing quote — terminate; the parser will
                // emit a diagnostic for the unterminated string later.
                break;
            }
            if c == quote {
                if self.peek_char() == Some(quote) {
                    // Doubled quote: consume both as a literal quote and
                    // keep scanning the same string.
                    self.advance();
                    self.advance();
                    continue;
                }
                // Single closing quote — consume it and stop.
                self.advance();
                break;
            }
            if c == '\\' {
                // Backslash escape: skip the backslash and one more
                // character if present. Preserves prior behaviour.
                self.advance();
                if self.current_char() != '\0' {
                    self.advance();
                }
                continue;
            }
            self.advance();
        }

        STRING
    }

    /// Attempt to lex a dollar-quoted string starting at the current `$`.
    ///
    /// Returns `Some(STRING)` on a well-formed, terminated dollar-quote (the
    /// lexer position is advanced past the closing delimiter), `Some(ERROR)`
    /// if a valid *opening* delimiter is found but no matching closing
    /// delimiter exists before EOF (position advanced to EOF — the whole
    /// unterminated tail becomes one ERROR token, never a silent split), or
    /// `None` if `$` does not open a valid delimiter at all (position left
    /// unchanged; the caller falls back to prior single-char ERROR
    /// behaviour).
    ///
    /// A delimiter is `$$` (empty tag) or `$tag$` where `tag` matches
    /// `[A-Za-z_][A-Za-z0-9_]*` — PostgreSQL/DuckDB dollar-quote grammar.
    /// The closing delimiter is the exact same literal string as the
    /// opening one, so `$$ ... $$` and `$tag$ ... $tag$` don't interfere:
    /// a `$$` occurring inside a `$tag$...$tag$` body is ordinary content
    /// (verified against DuckDB v1.5.4: `$tag$ x $$ y $tag$` yields the
    /// string ` x $$ y `).
    fn try_dollar_quote(&mut self) -> Option<SyntaxKind> {
        let bytes_len = self.input.len();
        let start = self.pos; // position of the opening '$'
        let mut i = start + 1;
        if i >= bytes_len {
            return None;
        }

        let tag_start = i;
        let first_char = self.input[i..].chars().next()?;
        if first_char == '$' {
            // Bare `$$` — empty tag; tag_end == tag_start (zero-length tag).
        } else if first_char.is_alphabetic() || first_char == '_' {
            i += first_char.len_utf8();
            loop {
                if i >= bytes_len {
                    // Ran off the end of input while still inside the tag —
                    // never a valid opening delimiter (no closing '$' for
                    // the tag itself, let alone the string body).
                    return None;
                }
                let ch = self.input[i..].chars().next()?;
                if ch == '$' {
                    break;
                } else if ch.is_alphanumeric() || ch == '_' {
                    i += ch.len_utf8();
                } else {
                    // Non-identifier character before the tag's closing
                    // '$' — not a valid dollar-quote delimiter (e.g. a
                    // prepared-statement-style `$name` with no second `$`).
                    return None;
                }
            }
        } else {
            return None;
        }

        let tag_end = i; // index of the '$' that closes the opening tag
        let tag = &self.input[tag_start..tag_end];
        let open_delim_end = tag_end + 1; // consume the tag's closing '$'
        let delimiter = format!("${tag}$");

        match self.input[open_delim_end..].find(delimiter.as_str()) {
            Some(rel_idx) => {
                let close_end = open_delim_end + rel_idx + delimiter.len();
                self.pos = close_end;
                Some(STRING)
            }
            None => {
                // Unterminated dollar-quote: consume through EOF as a single
                // ERROR token rather than silently splitting the input.
                self.pos = bytes_len;
                Some(ERROR)
            }
        }
    }

    fn consume_number(&mut self) -> SyntaxKind {
        self.consume_digits_with_separators();

        // Handle decimal point
        if self.current_char() == '.' && self.peek_char().is_some_and(|c| c.is_ascii_digit()) {
            self.advance(); // consume '.'
            self.consume_digits_with_separators();
        }

        // Handle scientific notation: e/E followed by optional +/- and then digits.
        // e.g. `1e8`, `1e-8`, `1.5e+3`, `2.3E-4`.
        // We only enter the exponent branch when the character immediately after
        // the mantissa is `e`/`E` and the character(s) that follow confirm this
        // is a numeric exponent, not the start of an identifier (e.g. `enum`).
        let cur = self.current_char();
        if cur == 'e' || cur == 'E' {
            let next = self.peek_char();
            let is_sci = match next {
                // e/E followed directly by a digit: `1e8`
                Some(c) if c.is_ascii_digit() => true,
                // e/E followed by sign then digit: `1e-8`, `1e+3`
                Some('+') | Some('-') => self
                    .peek_two_chars()
                    .is_some_and(|(_, after_sign)| after_sign.is_ascii_digit()),
                _ => false,
            };
            if is_sci {
                self.advance(); // consume 'e'/'E'
                if self.current_char() == '+' || self.current_char() == '-' {
                    self.advance(); // consume optional sign
                }
                self.consume_digits_with_separators(); // consume exponent digits
            }
        }

        // Fail-loud: a numeric literal immediately followed by an identifier
        // continuation character (letter or `_`) must never be silently
        // split into a shorter NUMBER token plus a following IDENT/alias —
        // e.g. `0x1F` must not read as `0` implicitly aliased to `x1F`.
        // Verified empirically against DuckDB v1.5.4: `SELECT 0x1F` (no
        // whitespace before an alias) reads as `0` implicitly aliased to
        // `x1F` — DuckDB has no true hex-integer-literal grammar in this
        // position, so a following `AS a` or any binary operator fails as a
        // syntax error (the alias slot is already filled, or the trailing
        // ident isn't a valid operator continuation). smelt refuses to
        // reproduce that silent split: the whole malformed blob becomes a
        // single ERROR token instead of being read as `0` aliased to `x1F`.
        //
        // Underscore digit separators (`1_000_000`, `1_000.000_1`,
        // `1_000e1_0`) are handled by `consume_digits_with_separators`
        // above/below and never reach this fallback — DuckDB accepts an
        // underscore only strictly *between* two digits within a single
        // digit run (integer part, fractional part, or exponent digits);
        // a leading/trailing underscore or a doubled `__` is rejected by
        // DuckDB too (verified empirically), and stays a single ERROR token
        // here rather than being silently split.
        if self.current_char().is_alphabetic() || self.current_char() == '_' {
            while self.current_char().is_alphanumeric() || self.current_char() == '_' {
                self.advance();
            }
            return ERROR;
        }

        NUMBER
    }

    /// Consume a run of ASCII digits, allowing DuckDB-style underscore digit
    /// separators strictly between two digits (never leading, trailing, or
    /// doubled). E.g. `1_000_000` and `1_0_0` are consumed in full; `1__0`
    /// and `1_` stop before the invalid underscore, leaving it for the
    /// fail-loud fallback in `consume_number` to fold into a single ERROR
    /// token.
    fn consume_digits_with_separators(&mut self) {
        while self.current_char().is_ascii_digit() {
            self.advance();
            if self.current_char() == '_' && self.peek_char().is_some_and(|c| c.is_ascii_digit()) {
                self.advance(); // consume the separator; loop consumes the following digit
            }
        }
    }

    fn consume_ident_or_keyword(&mut self) -> SyntaxKind {
        let start = self.pos;

        while self.current_char().is_alphanumeric() || self.current_char() == '_' {
            self.advance();
        }

        let text = &self.input[start..self.pos];
        keyword_or_ident(text)
    }
}

fn keyword_or_ident(text: &str) -> SyntaxKind {
    match text.to_uppercase().as_str() {
        "SELECT" => SELECT_KW,
        "FROM" => FROM_KW,
        "WHERE" => WHERE_KW,
        "GROUP" => GROUP_KW,
        "BY" => BY_KW,
        "AS" => AS_KW,
        "AND" => AND_KW,
        "OR" => OR_KW,
        "NOT" => NOT_KW,
        "IS" => IS_KW,
        "NULL" => NULL_KW,
        "JOIN" => JOIN_KW,
        "INNER" => INNER_KW,
        "LEFT" => LEFT_KW,
        "RIGHT" => RIGHT_KW,
        "FULL" => FULL_KW,
        "OUTER" => OUTER_KW,
        "CROSS" => CROSS_KW,
        "ON" => ON_KW,
        "USING" => USING_KW,
        // Phase 10: Expression keywords
        "CASE" => CASE_KW,
        "WHEN" => WHEN_KW,
        "THEN" => THEN_KW,
        "ELSE" => ELSE_KW,
        "END" => END_KW,
        "CAST" => CAST_KW,
        "TRY_CAST" => TRY_CAST_KW,
        "EXTRACT" => EXTRACT_KW,
        "BETWEEN" => BETWEEN_KW,
        "IN" => IN_KW,
        "EXISTS" => EXISTS_KW,
        "ANY" => ANY_KW,
        "SOME" => SOME_KW,
        // Phase 11: SQL clause keywords
        "ORDER" => ORDER_KW,
        "LIMIT" => LIMIT_KW,
        "OFFSET" => OFFSET_KW,
        "HAVING" => HAVING_KW,
        "DISTINCT" => DISTINCT_KW,
        "ALL" => ALL_KW,
        "ASC" => ASC_KW,
        "DESC" => DESC_KW,
        "NULLS" => NULLS_KW,
        "FIRST" => FIRST_KW,
        "LAST" => LAST_KW,
        // Phase 12: Window function keywords
        "OVER" => OVER_KW,
        "PARTITION" => PARTITION_KW,
        "WINDOW" => WINDOW_KW,
        "ROWS" => ROWS_KW,
        "RANGE" => RANGE_KW,
        "GROUPS" => GROUPS_KW,
        "UNBOUNDED" => UNBOUNDED_KW,
        "PRECEDING" => PRECEDING_KW,
        "FOLLOWING" => FOLLOWING_KW,
        "CURRENT" => CURRENT_KW,
        "ROW" => ROW_KW,
        // Phase 13: CTE keywords
        "WITH" => WITH_KW,
        "RECURSIVE" => RECURSIVE_KW,
        "UNION" => UNION_KW,
        "INTERSECT" => INTERSECT_KW,
        "EXCEPT" => EXCEPT_KW,
        // Phase 14: PostgreSQL-specific keywords
        "LATERAL" => LATERAL_KW,
        "TABLESAMPLE" => TABLESAMPLE_KW,
        "BERNOULLI" => BERNOULLI_KW,
        "SYSTEM" => SYSTEM_KW,
        "REPEATABLE" => REPEATABLE_KW,
        // Phase 15: Aggregate function keywords
        "FILTER" => FILTER_KW,
        // SQL data type/constructor keywords
        "ARRAY" => ARRAY_KW,
        "VALUES" => VALUES_KW,
        "STRUCT" => STRUCT_KW,
        // Note: WITHIN, EXCLUDE, TIES, OTHERS, NO, FETCH, NEXT, ONLY are handled
        // contextually in the parser (via IDENT text matching) to avoid conflicts
        // with these common words used as identifiers.
        // Multi-dialect superset keywords
        "QUALIFY" => QUALIFY_KW,
        "PIVOT" => PIVOT_KW,
        "UNPIVOT" => UNPIVOT_KW,
        "LIKE" => LIKE_KW,
        "ILIKE" => ILIKE_KW,
        "GLOB" => GLOB_KW,
        "COLLATE" => COLLATE_KW,
        // Phase B (meta-language): `fn` is a reserved keyword; any SQL that
        // previously used `fn` as a column, table, or alias name must now quote it
        // (e.g. `"fn"` or backtick-quoted `\`fn\``).
        "FN" => FN_KW,
        // Phase F (meta-language): `if` is a reserved keyword for the meta-world ternary.
        // `then` and `else` were already reserved as SQL CASE keywords; they keep that meaning
        // inside CASE expressions and gain ternary meaning outside CASE contexts.
        "IF" => IF_KW,
        _ => IDENT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== Phase 1 (meta-language): DOT_DOT_DOT token tests =====

    #[test]
    fn tokenize_triple_dot() {
        // `...` must lex as a single DOT_DOT_DOT token (length 3).
        let tokens = tokenize("...");
        assert_eq!(
            tokens.len(),
            1,
            "expected exactly one token, got: {:?}",
            tokens
        );
        assert_eq!(tokens[0].kind, DOT_DOT_DOT);
        assert_eq!(tokens[0].len, 3);
    }

    #[test]
    fn triple_dot_disambiguates_from_double_dot() {
        // `..foo` → DOT_DOT IDENT(foo)  (existing struct-spread, now a single DOT_DOT token)
        let tokens_double = tokenize("..foo");
        let non_ws: Vec<_> = tokens_double
            .iter()
            .filter(|t| t.kind != WHITESPACE)
            .collect();
        assert_eq!(
            non_ws[0].kind, DOT_DOT,
            "..foo should start with DOT_DOT, got {:?}",
            non_ws[0].kind
        );
        assert_eq!(non_ws[0].len, 2);
        assert_eq!(
            non_ws[1].kind, IDENT,
            "..foo second token should be IDENT, got {:?}",
            non_ws[1].kind
        );

        // `...foo` → DOT_DOT_DOT IDENT(foo)  (new list-spread)
        let tokens_triple = tokenize("...foo");
        let non_ws: Vec<_> = tokens_triple
            .iter()
            .filter(|t| t.kind != WHITESPACE)
            .collect();
        assert_eq!(
            non_ws[0].kind, DOT_DOT_DOT,
            "...foo should start with DOT_DOT_DOT, got {:?}",
            non_ws[0].kind
        );
        assert_eq!(non_ws[0].len, 3);
        assert_eq!(
            non_ws[1].kind, IDENT,
            "...foo second token should be IDENT, got {:?}",
            non_ws[1].kind
        );
    }

    #[test]
    fn test_not_equal_operators() {
        // Test != operator
        let tokens = tokenize("a != b");
        assert_eq!(tokens[2].kind, NE);

        // Test <> operator (PostgreSQL-style)
        let tokens = tokenize("a <> b");
        assert_eq!(tokens[2].kind, NE);
    }

    #[test]
    fn test_concat_operator() {
        // Test || string concatenation operator
        let tokens = tokenize("'a' || 'b'");
        assert_eq!(tokens[2].kind, CONCAT);
    }

    #[test]
    fn test_basic_sql() {
        let input = "SELECT user_id FROM events";
        let tokens = tokenize(input);

        assert_eq!(tokens[0].kind, SELECT_KW);
        assert_eq!(tokens[1].kind, WHITESPACE);
        assert_eq!(tokens[2].kind, IDENT); // user_id
        assert_eq!(tokens[3].kind, WHITESPACE);
        assert_eq!(tokens[4].kind, FROM_KW);
        assert_eq!(tokens[5].kind, WHITESPACE);
        assert_eq!(tokens[6].kind, IDENT); // events
    }

    #[test]
    fn test_ref_function() {
        let input = "ref('raw_events')";
        let tokens = tokenize(input);

        assert_eq!(tokens[0].kind, IDENT); // ref
        assert_eq!(tokens[1].kind, LPAREN);
        assert_eq!(tokens[2].kind, STRING);
        assert_eq!(tokens[3].kind, RPAREN);
    }

    #[test]
    fn test_comment() {
        let input = "-- This is a comment\nSELECT";
        let tokens = tokenize(input);

        assert_eq!(tokens[0].kind, COMMENT);
        assert_eq!(tokens[1].kind, WHITESPACE); // newline
        assert_eq!(tokens[2].kind, SELECT_KW);
    }

    // ===== Phase B (meta-language): `fn` keyword + `|>` pipe-arrow token =====

    #[test]
    fn tokenize_fn_keyword() {
        // `fn` is a reserved keyword: it lexes as FN_KW (not IDENT).
        // SQL that previously used `fn` as a column or table name must now quote it.
        let tokens = tokenize("fn");
        let non_ws: Vec<_> = tokens.iter().filter(|t| t.kind != WHITESPACE).collect();
        assert_eq!(
            non_ws.len(),
            1,
            "expected exactly one token, got: {:?}",
            non_ws
        );
        assert_eq!(
            non_ws[0].kind, FN_KW,
            "`fn` must lex as FN_KW (reserved keyword), got {:?}",
            non_ws[0].kind
        );
        assert_eq!(non_ws[0].len, 2);

        // `fnord` must still lex as a single IDENT (no over-eager keyword match).
        let tokens_fnord = tokenize("fnord");
        let non_ws_fnord: Vec<_> = tokens_fnord
            .iter()
            .filter(|t| t.kind != WHITESPACE)
            .collect();
        assert_eq!(
            non_ws_fnord.len(),
            1,
            "fnord should be a single IDENT token, got: {:?}",
            non_ws_fnord
        );
        assert_eq!(
            non_ws_fnord[0].kind, IDENT,
            "fnord should lex as IDENT, got {:?}",
            non_ws_fnord[0].kind
        );
        assert_eq!(non_ws_fnord[0].len, 5);
    }

    #[test]
    fn tokenize_pipe_arrow() {
        // `|>` must lex as a single PIPE_ARROW token (length 2).
        let tokens = tokenize("|>");
        let non_ws: Vec<_> = tokens.iter().filter(|t| t.kind != WHITESPACE).collect();
        assert_eq!(
            non_ws.len(),
            1,
            "expected exactly one token, got: {:?}",
            non_ws
        );
        assert_eq!(
            non_ws[0].kind, PIPE_ARROW,
            "expected PIPE_ARROW, got {:?}",
            non_ws[0].kind
        );
        assert_eq!(non_ws[0].len, 2);
    }

    #[test]
    fn pipe_arrow_disambiguates_from_double_pipe() {
        // `||` must lex as CONCAT (SQL string concatenation).
        let tokens_double = tokenize("||");
        let non_ws_double: Vec<_> = tokens_double
            .iter()
            .filter(|t| t.kind != WHITESPACE)
            .collect();
        assert_eq!(
            non_ws_double.len(),
            1,
            "|| should be one CONCAT token, got: {:?}",
            non_ws_double
        );
        assert_eq!(
            non_ws_double[0].kind, CONCAT,
            "|| should lex as CONCAT, got {:?}",
            non_ws_double[0].kind
        );

        // `|>` must lex as PIPE_ARROW.
        let tokens_pipe = tokenize("|>");
        let non_ws_pipe: Vec<_> = tokens_pipe
            .iter()
            .filter(|t| t.kind != WHITESPACE)
            .collect();
        assert_eq!(
            non_ws_pipe.len(),
            1,
            "|> should be one PIPE_ARROW token, got: {:?}",
            non_ws_pipe
        );
        assert_eq!(
            non_ws_pipe[0].kind, PIPE_ARROW,
            "|> should lex as PIPE_ARROW, got {:?}",
            non_ws_pipe[0].kind
        );

        // `|||>` must lex as CONCAT then PIPE_ARROW.
        let tokens_both = tokenize("|||>");
        let non_ws_both: Vec<_> = tokens_both
            .iter()
            .filter(|t| t.kind != WHITESPACE)
            .collect();
        assert_eq!(
            non_ws_both.len(),
            2,
            "|||> should be CONCAT + PIPE_ARROW, got: {:?}",
            non_ws_both
        );
        assert_eq!(non_ws_both[0].kind, CONCAT, "first token should be CONCAT");
        assert_eq!(
            non_ws_both[1].kind, PIPE_ARROW,
            "second token should be PIPE_ARROW"
        );
    }

    #[test]
    fn pipe_arrow_does_not_collide_with_named_arg() {
        // `=>` (named-arg arrow) must lex as ARROW; `|>` is PIPE_ARROW.
        let tokens_arrow = tokenize("=>");
        let non_ws_arrow: Vec<_> = tokens_arrow
            .iter()
            .filter(|t| t.kind != WHITESPACE)
            .collect();
        assert_eq!(
            non_ws_arrow.len(),
            1,
            "=> should be one ARROW token, got: {:?}",
            non_ws_arrow
        );
        assert_eq!(
            non_ws_arrow[0].kind, ARROW,
            "=> should lex as ARROW (named-arg), got {:?}",
            non_ws_arrow[0].kind
        );

        // `|>` is PIPE_ARROW, not a sequence ending in `>` that could be confused
        // with named-arg arrows.
        let tokens_pipe = tokenize("|>");
        let non_ws_pipe: Vec<_> = tokens_pipe
            .iter()
            .filter(|t| t.kind != WHITESPACE)
            .collect();
        assert_eq!(
            non_ws_pipe.len(),
            1,
            "|> should be one PIPE_ARROW token, got: {:?}",
            non_ws_pipe
        );
        assert_eq!(
            non_ws_pipe[0].kind, PIPE_ARROW,
            "|> should lex as PIPE_ARROW"
        );
    }

    // ===== Dollar-quoted string literals =====

    #[test]
    fn tokenize_bare_dollar_quote() {
        // `$$hello$$` lexes as a single STRING token spanning the whole thing.
        let tokens = tokenize("$$hello$$");
        let non_ws: Vec<_> = tokens.iter().filter(|t| t.kind != WHITESPACE).collect();
        assert_eq!(non_ws.len(), 1, "expected one token, got: {:?}", non_ws);
        assert_eq!(non_ws[0].kind, STRING);
        assert_eq!(non_ws[0].len, "$$hello$$".len());
    }

    #[test]
    fn tokenize_dollar_quote_with_single_quote_body() {
        // `$$a'b$$` — an embedded single quote is ordinary content, no escaping
        // needed (unlike `'...'` strings).
        let input = "$$a'b$$";
        let tokens = tokenize(input);
        let non_ws: Vec<_> = tokens.iter().filter(|t| t.kind != WHITESPACE).collect();
        assert_eq!(non_ws.len(), 1, "expected one token, got: {:?}", non_ws);
        assert_eq!(non_ws[0].kind, STRING);
        assert_eq!(non_ws[0].len, input.len());
    }

    #[test]
    fn tokenize_tagged_dollar_quote() {
        // `$tag$...$tag$` — a tagged delimiter.
        let input = "$tag$ x $$ y $tag$";
        let tokens = tokenize(input);
        let non_ws: Vec<_> = tokens.iter().filter(|t| t.kind != WHITESPACE).collect();
        assert_eq!(non_ws.len(), 1, "expected one token, got: {:?}", non_ws);
        assert_eq!(non_ws[0].kind, STRING);
        assert_eq!(non_ws[0].len, input.len());
    }

    #[test]
    fn tokenize_tagged_dollar_quote_nested_bare_dollar() {
        // A bare `$$` inside a `$tag$...$tag$` body is ordinary content — the
        // closing delimiter must match the *same* tag, not any `$...$` run.
        // Verified against DuckDB v1.5.4: `$tag$ x $$ y $tag$` -> " x $$ y ".
        let input = "SELECT $tag$ x $$ y $tag$ AS s";
        let tokens = tokenize(input);
        let non_ws: Vec<_> = tokens.iter().filter(|t| t.kind != WHITESPACE).collect();
        // SELECT, STRING, AS, s
        assert_eq!(
            non_ws.len(),
            4,
            "expected SELECT STRING AS IDENT, got: {:?}",
            non_ws
        );
        assert_eq!(non_ws[0].kind, SELECT_KW);
        assert_eq!(non_ws[1].kind, STRING);
        assert_eq!(non_ws[1].len, "$tag$ x $$ y $tag$".len());
        assert_eq!(non_ws[2].kind, AS_KW);
        assert_eq!(non_ws[3].kind, IDENT);
    }

    #[test]
    fn tokenize_unterminated_dollar_quote_is_single_error_token() {
        // `$$abc` with no closing delimiter must become one ERROR token
        // spanning the rest of the input — never a silent split into
        // partial tokens.
        let input = "SELECT $$abc";
        let tokens = tokenize(input);
        let non_ws: Vec<_> = tokens.iter().filter(|t| t.kind != WHITESPACE).collect();
        assert_eq!(non_ws.len(), 2, "expected SELECT ERROR, got: {:?}", non_ws);
        assert_eq!(non_ws[0].kind, SELECT_KW);
        assert_eq!(non_ws[1].kind, ERROR);
        assert_eq!(non_ws[1].len, "$$abc".len());
    }

    #[test]
    fn tokenize_dollar_not_a_delimiter_keeps_prior_behavior() {
        // `$notatag` has no closing `$`, so it is not a valid dollar-quote
        // delimiter at all: `$` lexes as a single ERROR token (unchanged
        // "unknown character" fallback) and `notatag` lexes separately as an
        // IDENT, exactly as before this change.
        let tokens = tokenize("$notatag");
        let non_ws: Vec<_> = tokens.iter().filter(|t| t.kind != WHITESPACE).collect();
        assert_eq!(non_ws.len(), 2, "expected ERROR IDENT, got: {:?}", non_ws);
        assert_eq!(non_ws[0].kind, ERROR);
        assert_eq!(non_ws[0].len, 1);
        assert_eq!(non_ws[1].kind, IDENT);
        assert_eq!(non_ws[1].len, "notatag".len());
    }

    #[test]
    fn tokenize_dollar_digit_keeps_prior_behavior() {
        // `$1` — a digit immediately after `$` never forms a valid tag
        // (tags must start with a letter or underscore), so `$` falls back
        // to the single-char ERROR token and `1` lexes as NUMBER.
        let tokens = tokenize("$1");
        let non_ws: Vec<_> = tokens.iter().filter(|t| t.kind != WHITESPACE).collect();
        assert_eq!(non_ws.len(), 2, "expected ERROR NUMBER, got: {:?}", non_ws);
        assert_eq!(non_ws[0].kind, ERROR);
        assert_eq!(non_ws[0].len, 1);
        assert_eq!(non_ws[1].kind, NUMBER);
    }

    #[test]
    fn tokenize_lone_dollar_at_eof_keeps_prior_behavior() {
        // A single trailing `$` with nothing after it is not a delimiter.
        let tokens = tokenize("$");
        let non_ws: Vec<_> = tokens.iter().filter(|t| t.kind != WHITESPACE).collect();
        assert_eq!(
            non_ws.len(),
            1,
            "expected one ERROR token, got: {:?}",
            non_ws
        );
        assert_eq!(non_ws[0].kind, ERROR);
        assert_eq!(non_ws[0].len, 1);
    }
}

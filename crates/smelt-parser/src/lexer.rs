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
            '|' if self.peek_char() == Some('|') => {
                self.advance();
                self.advance();
                CONCAT
            }
            '[' => {
                self.advance();
                LBRACKET
            }
            ']' => {
                self.advance();
                RBRACKET
            }
            ':' if self.peek_char() != Some(':') => {
                self.advance();
                COLON
            }

            // Hash operators (#>, #>>)
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

        // Consume until closing quote or EOF
        while self.current_char() != quote && self.current_char() != '\0' {
            if self.current_char() == '\\' {
                // Skip escaped character
                self.advance();
                if self.current_char() != '\0' {
                    self.advance();
                }
            } else {
                self.advance();
            }
        }

        // Consume closing quote if present
        if self.current_char() == quote {
            self.advance();
        }

        STRING
    }

    fn consume_number(&mut self) -> SyntaxKind {
        while self.current_char().is_ascii_digit() {
            self.advance();
        }

        // Handle decimal point
        if self.current_char() == '.' && self.peek_char().is_some_and(|c| c.is_ascii_digit()) {
            self.advance(); // consume '.'
            while self.current_char().is_ascii_digit() {
                self.advance();
            }
        }

        NUMBER
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
        _ => IDENT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

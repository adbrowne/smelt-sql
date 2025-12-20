/// Lexer for sqt SQL files with template expressions

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
            '/' => {
                self.advance();
                DIVIDE
            }
            '=' => {
                self.advance();
                EQ
            }
            '!' if self.peek_char() == Some('=') => {
                self.advance();
                self.advance();
                NE
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
        if self.current_char() == '.' && self.peek_char().map_or(false, |c| c.is_ascii_digit()) {
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
        _ => IDENT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

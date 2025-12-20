/// Token and node types for the sqt language

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum SyntaxKind {
    // Keywords
    SELECT_KW,
    FROM_KW,
    WHERE_KW,
    GROUP_KW,
    BY_KW,
    AS_KW,
    AND_KW,
    OR_KW,
    NOT_KW,

    // Operators & punctuation
    LPAREN,   // (
    RPAREN,   // )
    COMMA,    // ,
    DOT,      // .
    STAR,     // *
    EQ,       // =
    NE,       // !=
    LT,       // <
    GT,       // >
    LE,       // <=
    GE,       // >=
    PLUS,     // +
    MINUS,    // -
    MULTIPLY, // * (same as STAR, but in expression context)
    DIVIDE,   // /

    // Literals & identifiers
    STRING,     // 'value' or "value"
    NUMBER,     // 123, 3.14
    IDENT,      // column_name, table_name
    WHITESPACE, // spaces, tabs, newlines
    COMMENT,    // -- comment

    // Composite nodes
    FILE,            // Root node
    SELECT_STMT,     // SELECT ... FROM ... WHERE ...
    SELECT_LIST,     // column1, column2, *
    SELECT_ITEM,     // column or expression with optional alias
    FROM_CLAUSE,     // FROM table or ref() function
    TABLE_REF,       // Table reference (identifier or function call)
    WHERE_CLAUSE,    // WHERE expression
    GROUP_BY_CLAUSE, // GROUP BY column1, column2
    EXPRESSION,      // Generic expression
    BINARY_EXPR,     // left op right
    FUNCTION_CALL,   // COUNT(*), SUM(col), ref('model')
    ARG_LIST,        // (arg1, arg2)

    // Error handling
    ERROR, // Invalid syntax

    // Special
    EOF, // End of file
}

use SyntaxKind::*;

impl SyntaxKind {
    pub fn is_keyword(&self) -> bool {
        matches!(
            self,
            SELECT_KW | FROM_KW | WHERE_KW | GROUP_KW | BY_KW | AS_KW | AND_KW | OR_KW | NOT_KW
        )
    }

    pub fn is_trivia(&self) -> bool {
        matches!(self, WHITESPACE | COMMENT)
    }

    pub fn is_literal(&self) -> bool {
        matches!(self, STRING | NUMBER)
    }
}

impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> Self {
        Self(kind as u16)
    }
}

/// The language type for Rowan
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SqtLanguage {}

impl rowan::Language for SqtLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
        assert!(raw.0 <= SyntaxKind::EOF as u16);
        unsafe { std::mem::transmute::<u16, SyntaxKind>(raw.0) }
    }

    fn kind_to_raw(kind: Self::Kind) -> rowan::SyntaxKind {
        kind.into()
    }
}

/// Convenient type aliases
pub type SyntaxNode = rowan::SyntaxNode<SqtLanguage>;
pub type SyntaxToken = rowan::SyntaxToken<SqtLanguage>;
pub type SyntaxElement = rowan::SyntaxElement<SqtLanguage>;

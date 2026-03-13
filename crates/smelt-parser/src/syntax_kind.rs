/// Token and node types for the smelt language
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
#[allow(non_camel_case_types)]
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
    IS_KW,
    NULL_KW,
    JOIN_KW,
    INNER_KW,
    LEFT_KW,
    RIGHT_KW,
    FULL_KW,
    OUTER_KW,
    CROSS_KW,
    ON_KW,
    USING_KW,
    // Phase 10: Expression keywords
    CASE_KW,
    WHEN_KW,
    THEN_KW,
    ELSE_KW,
    END_KW,
    CAST_KW,
    BETWEEN_KW,
    IN_KW,
    EXISTS_KW,
    ANY_KW,
    SOME_KW,
    // Phase 11: SQL clause keywords
    ORDER_KW,
    LIMIT_KW,
    OFFSET_KW,
    HAVING_KW,
    DISTINCT_KW,
    ALL_KW,
    ASC_KW,
    DESC_KW,
    NULLS_KW,
    FIRST_KW,
    LAST_KW,
    // Phase 12: Window function keywords
    OVER_KW,
    PARTITION_KW,
    WINDOW_KW,
    ROWS_KW,
    RANGE_KW,
    GROUPS_KW,
    UNBOUNDED_KW,
    PRECEDING_KW,
    FOLLOWING_KW,
    CURRENT_KW,
    ROW_KW,
    // Phase 13: CTE keywords
    WITH_KW,
    RECURSIVE_KW,
    UNION_KW,
    INTERSECT_KW,
    EXCEPT_KW,
    // Phase 14: PostgreSQL-specific keywords
    LATERAL_KW,
    TABLESAMPLE_KW,
    BERNOULLI_KW,
    SYSTEM_KW,
    REPEATABLE_KW,
    // Phase 15: Aggregate function keywords
    FILTER_KW,
    // SQL data type/constructor keywords
    ARRAY_KW,
    VALUES_KW,
    STRUCT_KW,
    // Multi-dialect superset keywords
    QUALIFY_KW,
    PIVOT_KW,
    UNPIVOT_KW,

    // Contextual keywords (lexed as IDENT, recognized by parser contextually)
    // These variants exist for the enum but are never produced by the lexer.
    // They are kept for potential future use in AST node type discrimination.
    WITHIN_KW,
    EXCLUDE_KW,
    TIES_KW,
    OTHERS_KW,
    NO_KW,
    FETCH_KW,
    NEXT_KW,
    ONLY_KW,

    // Operators & punctuation
    LPAREN,       // (
    RPAREN,       // )
    COMMA,        // ,
    DOT,          // .
    STAR,         // *
    EQ,           // =
    NE,           // !=
    LT,           // <
    GT,           // >
    LE,           // <=
    GE,           // >=
    PLUS,         // +
    MINUS,        // -
    MULTIPLY,     // * (same as STAR, but in expression context)
    DIVIDE,       // /
    ARROW,        // => (named parameter)
    DOUBLE_COLON, // :: (PostgreSQL cast operator)
    CONCAT,       // || (string concatenation)
    LBRACKET,     // [
    RBRACKET,     // ]
    COLON,        // : (used in array slices)
    // JSON operators
    JSON_ARROW,      // ->
    JSON_ARROW_TEXT, // ->>
    HASH_ARROW,      // #>
    HASH_ARROW_TEXT, // #>>
    AT_GT,           // @>
    LT_AT,           // <@
    // Regex operators
    TILDE,          // ~
    TILDE_STAR,     // ~*
    NOT_TILDE,      // !~
    NOT_TILDE_STAR, // !~*

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
    JOIN_CLAUSE,     // Complete JOIN clause with type and condition
    JOIN_CONDITION,  // ON expr or USING (cols)
    WHERE_CLAUSE,    // WHERE expression
    GROUP_BY_CLAUSE, // GROUP BY column1, column2
    EXPRESSION,      // Generic expression
    BINARY_EXPR,     // left op right
    FUNCTION_CALL,   // COUNT(*), SUM(col), ref('model')
    ARG_LIST,        // (arg1, arg2)
    NAMED_PARAM,     // param_name => value
    // Phase 10: Expression nodes
    CASE_EXPR,    // CASE WHEN ... THEN ... END
    WHEN_CLAUSE,  // WHEN condition THEN result
    CAST_EXPR,    // CAST(expr AS type) or expr::type
    TYPE_SPEC,    // Type specification (INT, VARCHAR(255), etc.)
    SUBQUERY,     // (SELECT ...)
    BETWEEN_EXPR, // expr BETWEEN low AND high
    IN_EXPR,      // expr IN (values...)
    EXISTS_EXPR,  // EXISTS (subquery)
    // Phase 11: SQL clause nodes
    HAVING_CLAUSE,   // HAVING expression
    ORDER_BY_CLAUSE, // ORDER BY column1, column2
    ORDER_BY_ITEM,   // Single ORDER BY item with direction and null ordering
    LIMIT_CLAUSE,    // LIMIT n OFFSET m
    // Phase 12: Window function nodes
    WINDOW_SPEC,         // OVER clause specification
    PARTITION_BY_CLAUSE, // PARTITION BY expr, expr
    WINDOW_FRAME,        // ROWS/RANGE/GROUPS specification
    FRAME_BOUND,         // bound specification like UNBOUNDED PRECEDING
    // Phase 13: CTE nodes
    WITH_CLAUSE, // WITH clause (entire WITH statement)
    CTE,         // Single common table expression
    // Phase 14: PostgreSQL-specific nodes
    DISTINCT_ON_CLAUSE, // DISTINCT ON (expr, expr)
    TABLESAMPLE_CLAUSE, // TABLESAMPLE method (percentage) REPEATABLE (seed)
    // Phase 15: Aggregate function nodes
    FILTER_CLAUSE, // FILTER (WHERE condition)
    // Multi-dialect superset nodes
    QUALIFY_CLAUSE,      // QUALIFY expression
    LAMBDA_EXPR,         // x -> expr or (x, y) -> expr
    LAMBDA_PARAM_LIST,   // Parameter list in lambda
    PIVOT_CLAUSE,        // PIVOT (agg FOR col IN (...))
    UNPIVOT_CLAUSE,      // UNPIVOT (val FOR name IN (...))
    PIVOT_IN_LIST,       // IN list for PIVOT/UNPIVOT
    ARRAY_SUBSCRIPT,     // expr[expr]
    ARRAY_SLICE,         // expr[expr:expr]
    ARRAY_LITERAL,       // ARRAY[1, 2, 3]
    VALUES_CLAUSE,       // VALUES (1, 2), (3, 4)
    VALUES_ROW,          // Single row in VALUES: (1, 2)
    STRUCT_LITERAL,      // STRUCT(a, b, c)
    ROW_CONSTRUCTOR,     // ROW(1, 2, 3)
    ANY_EXPR,            // ANY(expr) / ALL(expr) / SOME(expr)
    WITHIN_GROUP_CLAUSE, // WITHIN GROUP (ORDER BY ...)
    FRAME_EXCLUDE,       // EXCLUDE CURRENT ROW / GROUP / TIES / NO OTHERS
    FETCH_CLAUSE,        // FETCH FIRST N ROWS ONLY

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
            SELECT_KW
                | FROM_KW
                | WHERE_KW
                | GROUP_KW
                | BY_KW
                | AS_KW
                | AND_KW
                | OR_KW
                | NOT_KW
                | IS_KW
                | NULL_KW
                | JOIN_KW
                | INNER_KW
                | LEFT_KW
                | RIGHT_KW
                | FULL_KW
                | OUTER_KW
                | CROSS_KW
                | ON_KW
                | USING_KW
                | CASE_KW
                | WHEN_KW
                | THEN_KW
                | ELSE_KW
                | END_KW
                | CAST_KW
                | BETWEEN_KW
                | IN_KW
                | EXISTS_KW
                | ANY_KW
                | SOME_KW
                | ORDER_KW
                | LIMIT_KW
                | OFFSET_KW
                | HAVING_KW
                | DISTINCT_KW
                | ALL_KW
                | ASC_KW
                | DESC_KW
                | NULLS_KW
                | FIRST_KW
                | LAST_KW
                | OVER_KW
                | PARTITION_KW
                | WINDOW_KW
                | ROWS_KW
                | RANGE_KW
                | GROUPS_KW
                | UNBOUNDED_KW
                | PRECEDING_KW
                | FOLLOWING_KW
                | CURRENT_KW
                | ROW_KW
                | WITH_KW
                | RECURSIVE_KW
                | UNION_KW
                | INTERSECT_KW
                | EXCEPT_KW
                | LATERAL_KW
                | TABLESAMPLE_KW
                | BERNOULLI_KW
                | SYSTEM_KW
                | REPEATABLE_KW
                | FILTER_KW
                | ARRAY_KW
                | VALUES_KW
                | STRUCT_KW
                | QUALIFY_KW
                | PIVOT_KW
                | UNPIVOT_KW
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
pub enum SmeltLanguage {}

impl rowan::Language for SmeltLanguage {
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
pub type SyntaxNode = rowan::SyntaxNode<SmeltLanguage>;
pub type SyntaxToken = rowan::SyntaxToken<SmeltLanguage>;
pub type SyntaxElement = rowan::SyntaxElement<SmeltLanguage>;

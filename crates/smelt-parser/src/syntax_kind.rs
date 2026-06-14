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
    // Phase B (meta-language): `fn` reserved keyword
    FN_KW,
    // Phase F (meta-language): `if` reserved keyword for meta-world ternary
    // (`then` and `else` already exist as THEN_KW / ELSE_KW — SQL CASE keywords)
    IF_KW,
    // SQL data type/constructor keywords
    ARRAY_KW,
    VALUES_KW,
    STRUCT_KW,
    // Multi-dialect superset keywords
    QUALIFY_KW,
    PIVOT_KW,
    UNPIVOT_KW,
    LIKE_KW,
    ILIKE_KW,
    EXTRACT_KW,
    COLLATE_KW,

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
    DOT_DOT,      // .. (struct/row spread)
    DOT_DOT_DOT,  // ... (list spread)
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
    PERCENT,      // %
    ARROW,        // => (named parameter)
    DOUBLE_COLON, // :: (PostgreSQL cast operator)
    CONCAT,       // || (string concatenation)
    LBRACKET,     // [
    RBRACKET,     // ]
    LBRACE,       // { (row-requirement opener in TableExpr<{...}>, Phase 13)
    RBRACE,       // } (row-requirement closer in TableExpr<{...}>, Phase 13)
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
    ALIAS_COLUMN_LIST,   // Column list after table alias: AS t(c1, c2, …) or CTE name(c1, c2, …)
    STRUCT_LITERAL,      // STRUCT(a, b, c)
    ROW_CONSTRUCTOR,     // ROW(1, 2, 3)
    EXTRACT_EXPR,        // EXTRACT(field FROM expr)
    COLLATE_EXPR,        // expr COLLATE collation_name
    ANY_EXPR,            // ANY(expr) / ALL(expr) / SOME(expr)
    WITHIN_GROUP_CLAUSE, // WITHIN GROUP (ORDER BY ...)
    FRAME_EXCLUDE,       // EXCLUDE CURRENT ROW / GROUP / TIES / NO OTHERS
    FETCH_CLAUSE,        // FETCH FIRST N ROWS ONLY

    // smelt.define top-level declaration
    SMELT_DEFINE,  // smelt.define name(params) [-> TypeRef] AS (body)
    DEFINE_NAME,   // Wraps the function's identifier in a smelt.define
    PARAM_LIST,    // Parameter list of a smelt.define: ( param, param, ... )
    PARAM,         // Single parameter: name [: TypeRef] [= DEFAULT_VALUE]
    TYPE_REF,      // Type reference (flat in Phase 1: e.g. tokens `Expr < Numeric >`)
    DEFINE_BODY,   // Parenthesized body expression of a smelt.define
    RETURN_ARROW,  // Return type arrow: -> <TypeRef>
    DEFAULT_VALUE, // Default value of a parameter: = <expression>

    // Unified `smelt.<path>` value reference and call form (smelt.<path> migration, Phase 1)
    //
    // SMELT_PATH_REF: `smelt.<seg>(.<seg>)*` with no trailing `(`. Used in
    //   FROM/argument position to refer to any project-defined entity by its
    //   workspace-relative path (model, function, seed, source, test).
    // SMELT_PATH_CALL: same path followed by `(args)` and optional PASSING
    //   clauses. Used to call parameterised entities (functions, parameterised
    //   models).
    // SMELT_PATH_CALL_STAR: a SMELT_PATH_CALL immediately followed by `.*` in a
    //   SELECT-list position. The child SMELT_PATH_CALL is the inner call; the
    //   DOT and STAR tokens are also children of this node. The printer lowers
    //   this to per-field projections when the function's body is a brace-struct
    //   literal, or falls back to verbatim emission otherwise.
    SMELT_PATH_REF,
    SMELT_PATH_CALL,
    SMELT_PATH_CALL_STAR,
    SMELT_PATH, // Dotted path inside a SMELT_PATH_REF / SMELT_PATH_CALL — the
    // tokens AFTER the leading `smelt.` prefix (which is also captured here).

    // PASSING clauses (Phase 28)
    PASSING_CLAUSE, // One clause: PASSING <name> AS ( <body> )
    PASSING_NAME,   // The identifier after PASSING
    PASSING_BODY,   // The expression inside (...)

    // smelt.extern top-level declaration (Phase 10)
    SMELT_EXTERN, // smelt.extern name(params) -> TypeRef

    // Phase 35: Struct type references and brace-struct literals
    STRUCT_TYPE,          // Struct<{field: Type, ..tail}> in a type reference
    STRUCT_FIELD,         // Single `name: Type` pair inside a STRUCT_TYPE
    ROW_TAIL,             // The trailing `..r` or `..` inside a STRUCT_TYPE
    BRACE_STRUCT_LITERAL, // {expr AS name, ..spread} struct-literal expression
    STRUCT_FIELD_ITEM,    // Single `expr AS name` item inside a BRACE_STRUCT_LITERAL
    SPREAD_ITEM,          // `..name` spread item inside a BRACE_STRUCT_LITERAL

    // Phase 38: smelt.as_struct() call
    SMELT_AS_STRUCT_CALL, // smelt.as_struct(alias [EXCEPT col1, col2, ...])
    EXCEPT_COL_LIST,      // EXCEPT col1, col2, ... inside SMELT_AS_STRUCT_CALL

    // Phase 1 (meta-language): list spread operator `...xs` in comma-separated positions
    LIST_SPREAD, // `...expr` — spread a List<T> into a comma-separated position

    // Phase B (meta-language): `|>` pipe-arrow token
    PIPE_ARROW, // `|>` — meta-language pipe operator

    // Phase B (meta-language): lambda and pipe CST nodes
    // LAMBDA: `fn IDENT => EXPR` or `fn (IDENT, ...) => EXPR`
    // PIPE_EXPR: `EXPR |> CALL(args...)` — left-associative; spread (`...`) is outermost, pipe is next
    LAMBDA,
    PIPE_EXPR,
    // Phase F (meta-language): single parameter node inside LAMBDA_PARAM_LIST.
    // Each IDENT in the lambda parameter list is wrapped in a LAMBDA_PARAM node.
    LAMBDA_PARAM,
    // Phase F (meta-language): meta-world ternary `if COND then THEN else ELSE`.
    // TERNARY_EXPR holds three EXPRESSION children: condition, then_branch, else_branch.
    TERNARY_EXPR,
    // Phase F (meta-language): parameterised reducer call in the second-argument position
    // of `reduce`. Shape: IDENT ARG_LIST. Only emitted in reduce's second-arg context.
    REDUCER_CALL,

    // Phase 13: structured TypeRef children for TableExpr / AggExpr /
    // WindowExpr / SelectItems parameter sorts. These are emitted as
    // children of a TYPE_REF node by parse_type_ref when the leading
    // identifier matches one of the recognised sort heads.
    //
    // The EXPR_KIND_* nodes are zero-width markers whose variant
    // classifies the head (Scalar / Agg / Window). They contain no
    // tokens so they don't alter the TYPE_REF's text range — the AST
    // wrapper classifies via SyntaxKind rather than a stored string.
    EXPR_KIND_SCALAR, // Marker: `Expr<T>` has `Scalar` kind.
    EXPR_KIND_AGG,    // Marker: `AggExpr<T>` has `Agg` kind.
    EXPR_KIND_WINDOW, // Marker: `WindowExpr<T>` has `Window` kind.
    ROW_REQUIREMENT,  // { field: Type [, ..r] } under a TableExpr<...>
    ROW_FIELD,        // Single field inside ROW_REQUIREMENT: `name: TypeRef`
    ROW_TAIL_NAMED,   // `..<ident>` tail in a ROW_REQUIREMENT
    ROW_TAIL_ANON,    // bare `..` tail in a ROW_REQUIREMENT
    SELECTITEMS_KIND, // Capital-Kind argument of SelectItems<K, ctx>
    SELECTITEMS_CTX,  // lowercase context argument of SelectItems<K, ctx>
    EXPR_CTX,         // lowercase context argument of Expr<T, ctx>

    // Phase 2 (meta-language): record declarations, literals, inline types, map methods
    //
    // SMELT_RECORD_DECL: top-level `smelt.record Name = { field: Type, ... }` declaration.
    // RECORD_FIELD: shared `IDENT COLON …` triple inside a record decl body, literal, or
    //   inline-record type.
    // RECORD_LITERAL: `{f1: v1, f2: v2}` in a value position — fields have expression values.
    // RECORD_TYPE_INLINE: `{f1: T1, f2: T2}` in a type-annotation position — fields have
    //   type-ref values.
    // MAP_METHOD_CALL: `expr.method(args)` where the method is a Map API method
    //   (entries, get, keys, values, contains_key, etc.). Emitted as a distinct node kind so
    //   Phase 4 type inference can dispatch the closed Map API without ambiguity against
    //   field-projection on records/structs.
    SMELT_RECORD_DECL,
    RECORD_FIELD,
    RECORD_LITERAL,
    RECORD_TYPE_INLINE,
    MAP_METHOD_CALL,

    // Error handling
    ERROR, // Invalid syntax

    // Phase 5 (nullability-soundness): NOT NULL qualifier on type references.
    // Emitted as a structured child node inside a TYPE_REF to mark the presence
    // of the `NOT NULL` suffix (e.g. `Expr<Integer NOT NULL>`).
    NOT_NULL_QUALIFIER,

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
                | FN_KW
                | IF_KW
                | ARRAY_KW
                | VALUES_KW
                | STRUCT_KW
                | QUALIFY_KW
                | PIVOT_KW
                | UNPIVOT_KW
                | LIKE_KW
                | ILIKE_KW
                | EXTRACT_KW
                | COLLATE_KW
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

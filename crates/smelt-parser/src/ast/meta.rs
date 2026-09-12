use super::*;
use crate::syntax_kind::SyntaxNode;

// ===== Phase 1 (meta-language): List literals and spread =====

/// A bracket list literal `[a, b, c]` (Phase 1 meta-language).
///
/// The same CST kind (`ARRAY_LITERAL`) is shared with `ARRAY[...]` Data-World
/// array literals. The type checker disambiguates between meta `List<T>` and
/// Data-World `Array<U>` in a later phase.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ListLiteral(SyntaxNode);

impl ListLiteral {
    /// Cast from a raw `SyntaxNode`. Returns `Some` only for `ARRAY_LITERAL` nodes.
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == ARRAY_LITERAL {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Iterate over element expressions in the list literal.
    pub fn elements(&self) -> impl Iterator<Item = Expr> + '_ {
        self.0.children().filter_map(Expr::cast)
    }

    /// Iterate over spread elements inside the list literal.
    pub fn spread_elements(&self) -> impl Iterator<Item = ListSpread> + '_ {
        self.0.children().filter_map(ListSpread::cast)
    }
}

/// A list spread expression `...expr` (Phase 1 meta-language).
///
/// Valid in any comma-separated grammar position: SELECT lists, GROUP BY,
/// ORDER BY, function arguments, IN-lists, VALUES rows, and list-literal
/// elements. Forbidden-position validation is the type-checker's job (Phase 3).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ListSpread(SyntaxNode);

impl ListSpread {
    /// Cast from a raw `SyntaxNode`. Returns `Some` only for `LIST_SPREAD` nodes.
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == LIST_SPREAD {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The operand expression being spread (e.g. `metric_exprs` for `...metric_exprs`).
    pub fn operand(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }
}

// ===== Phase B + Phase F (meta-language): Lambda, TernaryExpr, ReducerCall typed wrappers =====

/// A single parameter in a `fn` lambda parameter list (Phase F).
///
/// CST shape:
/// ```text
/// LAMBDA_PARAM
///   IDENT   (the parameter name)
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LambdaParam(SyntaxNode);

impl LambdaParam {
    /// Cast from a raw `SyntaxNode`. Returns `Some` only for `LAMBDA_PARAM` nodes.
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == LAMBDA_PARAM {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The parameter name (the text of the IDENT token inside this node).
    pub fn name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }
}

/// A `LAMBDA_PARAM_LIST` node — the parameter list of a `fn` lambda.
///
/// Phase F shape (canonical):
/// ```text
/// LAMBDA_PARAM_LIST
///   LAMBDA_PARAM  (one per parameter)
///     IDENT
///   LAMBDA_PARAM
///     IDENT
///   ...
/// ```
///
/// For a single-arg lambda (`fn x => body`) this contains one `LAMBDA_PARAM` child.
/// For a multi-arg lambda (`fn (a, b) => body`) it contains two or more.
/// For a zero-arg lambda (`fn () => body`) it contains zero (flagged downstream).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LambdaParamList(SyntaxNode);

impl LambdaParamList {
    /// Cast from a raw `SyntaxNode`. Returns `Some` only for `LAMBDA_PARAM_LIST` nodes.
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == LAMBDA_PARAM_LIST {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The typed parameter nodes, in declaration order.
    pub fn lambda_params(&self) -> Vec<LambdaParam> {
        self.0.children().filter_map(LambdaParam::cast).collect()
    }

    /// The parameter names, in order. For `fn x => body` this is `["x"]`.
    /// For `fn (a, b) => body` this is `["a", "b"]`.
    pub fn params(&self) -> Vec<String> {
        self.lambda_params()
            .into_iter()
            .filter_map(|p| p.name())
            .collect()
    }

    /// Returns `true` if this is a multi-arg parameter list (more than one LAMBDA_PARAM child).
    pub fn is_multi_arg(&self) -> bool {
        self.lambda_params().len() > 1
    }
}

/// A Phase B/F meta-language lambda: `fn IDENT => EXPR` or `fn (IDENT, ...) => EXPR`.
///
/// CST shape:
/// ```text
/// LAMBDA
///   FN_KW    (reserved `fn` keyword)
///   LAMBDA_PARAM_LIST
///     LAMBDA_PARAM  (one per parameter)
///       IDENT
///   ARROW (=>)
///   EXPRESSION
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Lambda(SyntaxNode);

impl Lambda {
    /// Cast from a raw `SyntaxNode`. Returns `Some` only for `LAMBDA` nodes.
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == LAMBDA {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The parameter list of this lambda.
    pub fn param_list(&self) -> Option<LambdaParamList> {
        self.0.children().find_map(LambdaParamList::cast)
    }

    /// The typed parameter nodes (via `param_list()`).
    pub fn lambda_params(&self) -> Vec<LambdaParam> {
        self.param_list()
            .map(|p| p.lambda_params())
            .unwrap_or_default()
    }

    /// Convenience: the parameter names (from `param_list()`).
    pub fn params(&self) -> Vec<String> {
        self.param_list().map(|p| p.params()).unwrap_or_default()
    }

    /// The body expression of this lambda.
    pub fn body(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    /// `true` if this lambda has more than one parameter.
    pub fn is_multi_arg(&self) -> bool {
        self.param_list().map(|p| p.is_multi_arg()).unwrap_or(false)
    }
}

/// A Phase F meta-world ternary expression: `if COND then THEN_EXPR else ELSE_EXPR`.
///
/// CST shape:
/// ```text
/// TERNARY_EXPR
///   IF_KW
///   EXPRESSION  (condition)
///   THEN_KW
///   EXPRESSION  (then-branch)
///   ELSE_KW
///   EXPRESSION  (else-branch, absent when dangling)
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TernaryExpr(SyntaxNode);

impl TernaryExpr {
    /// Cast from a raw `SyntaxNode`. Returns `Some` only for `TERNARY_EXPR` nodes.
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == TERNARY_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Helper: return the Nth `EXPRESSION` child (0-indexed).
    fn nth_expr(&self, n: usize) -> Option<Expr> {
        self.0.children().filter_map(Expr::cast).nth(n)
    }

    /// The condition expression (first `EXPRESSION` child).
    pub fn condition(&self) -> Option<Expr> {
        self.nth_expr(0)
    }

    /// The then-branch expression (second `EXPRESSION` child).
    pub fn then_branch(&self) -> Option<Expr> {
        self.nth_expr(1)
    }

    /// The else-branch expression (third `EXPRESSION` child).
    /// Returns `None` for incomplete ternaries (`TernaryDanglingElse`).
    pub fn else_branch(&self) -> Option<Expr> {
        self.nth_expr(2)
    }
}

/// A Phase F parameterised reducer call: `IDENT(args...)` in the second-argument
/// position of a `reduce` call.
///
/// CST shape:
/// ```text
/// REDUCER_CALL
///   IDENT   (reducer name, e.g. `concat_with`)
///   ARG_LIST
///     EXPRESSION  (first argument, e.g. `' OR '`)
/// ```
///
/// Only emitted in `reduce`'s second-argument context; everywhere else the same
/// syntax produces a generic `FUNCTION_CALL` node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReducerCall(SyntaxNode);

impl ReducerCall {
    /// Cast from a raw `SyntaxNode`. Returns `Some` only for `REDUCER_CALL` nodes.
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == REDUCER_CALL {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The reducer name (the leading IDENT token).
    pub fn name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// The argument list of the parameterised reducer call.
    pub fn args(&self) -> Option<SyntaxNode> {
        self.0.children().find(|n| n.kind() == ARG_LIST)
    }
}

/// A Phase B meta-language pipe expression: `EXPR |> EXPR`.
///
/// CST shape:
/// ```text
/// PIPE_EXPR
///   <LHS content>...   (tokens/nodes from the LHS expression)
///   PIPE_ARROW
///   EXPRESSION         (the RHS)
/// ```
///
/// Pipe is left-associative and lowest-precedence among meta-language operators.
/// `a |> b(p) |> c(q)` parses as `((a |> b(p)) |> c(q))`:
/// the outer PIPE_EXPR's first non-trivia children form the LHS (which is itself
/// a PIPE_EXPR), followed by `|>`, followed by an EXPRESSION child for the RHS.
///
/// The RHS must syntactically be a call expression; Phase 3 emits `PipeRhsNotCall`
/// for non-call RHS. The parser does not gate.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PipeExpr(SyntaxNode);

impl PipeExpr {
    /// Cast from a raw `SyntaxNode`. Returns `Some` only for `PIPE_EXPR` nodes.
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == PIPE_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The left-hand side of the pipe: the expression before the `|>` token.
    ///
    /// For `a |> b(p) |> c(q)`, the outer `PipeExpr::lhs()` returns the inner
    /// `PipeExpr` node (`a |> b(p)`) as an `Expr::PipeExpr`. For a simple
    /// `a |> b()`, `lhs()` returns the `a` reference expression.
    ///
    /// Implemented by iterating `children_with_tokens()` and returning the last
    /// `Expr`-castable node encountered before the `PIPE_ARROW` token.
    pub fn lhs(&self) -> Option<Expr> {
        let mut last_expr: Option<Expr> = None;
        for child in self.0.children_with_tokens() {
            match child {
                rowan::NodeOrToken::Token(t) if t.kind() == PIPE_ARROW => break,
                rowan::NodeOrToken::Node(n) => {
                    if let Some(e) = Expr::cast(n) {
                        last_expr = Some(e);
                    }
                }
                _ => {}
            }
        }
        last_expr
    }

    /// The right-hand side of the pipe: the `EXPRESSION` child that follows
    /// the `PIPE_ARROW` token.
    ///
    /// For chained pipes like `a |> b(p) |> c(q)`, the outer `PipeExpr::rhs()`
    /// must return the expression containing `c(q)`, NOT the inner pipe.
    /// Using `find_map(Expr::cast)` on `children()` would incorrectly return
    /// the inner `PIPE_EXPR` child first, since `Expr::cast` accepts `PIPE_EXPR`.
    /// This implementation skips past the `PIPE_ARROW` token before casting.
    pub fn rhs(&self) -> Option<Expr> {
        // Iterate children_with_tokens; after the PIPE_ARROW, the next node is RHS.
        let mut past_arrow = false;
        for child in self.0.children_with_tokens() {
            if !past_arrow {
                if let rowan::NodeOrToken::Token(t) = &child {
                    if t.kind() == PIPE_ARROW {
                        past_arrow = true;
                    }
                }
            } else if let rowan::NodeOrToken::Node(n) = child {
                if let Some(e) = Expr::cast(n) {
                    return Some(e);
                }
            }
        }
        None
    }

    /// True when the RHS is a function call expression (the expected case).
    /// Phase 3 emits `PipeRhsNotCall` when this returns false.
    pub fn rhs_is_call(&self) -> bool {
        self.rhs()
            .map(|e| {
                e.syntax()
                    .descendants()
                    .any(|n| n.kind() == FUNCTION_CALL || n.kind() == SMELT_PATH_CALL)
            })
            .unwrap_or(false)
    }
}

// ===== Phase 2 (meta-language): record types, literals, map methods =====

/// A top-level `smelt.record Name = { field: Type, ... }` declaration.
///
/// The body is wrapped in a `RECORD_TYPE_INLINE` child so that the declaration
/// body shares the same node kind as an inline-record type annotation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SmeltRecordDecl(SyntaxNode);

impl SmeltRecordDecl {
    /// Cast from a raw `SyntaxNode`. Returns `Some` only for `SMELT_RECORD_DECL` nodes.
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SMELT_RECORD_DECL {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The declared record type name (the `IDENT` token after `smelt . record`).
    ///
    /// Skips the leading `smelt` and `record` tokens to find the type name.
    pub fn name(&self) -> Option<String> {
        // The token sequence is: IDENT("smelt") DOT IDENT("record") [trivia] IDENT(Name) ...
        // We skip the first two IDENTs ("smelt", "record") and return the third.
        let idents: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == IDENT)
            .collect();
        // index 0: "smelt", index 1: "record", index 2: TypeName
        idents.get(2).map(|t| t.text().to_string())
    }

    /// The body as a `RECORD_TYPE_INLINE` node.
    pub fn body(&self) -> Option<RecordTypeInline> {
        self.0.children().find_map(RecordTypeInline::cast)
    }

    /// Iterate over the declared fields (direct children of the body).
    pub fn fields(&self) -> impl Iterator<Item = RecordField> + '_ {
        self.body().into_iter().flat_map(|b| {
            b.syntax()
                .children()
                .filter_map(RecordField::cast)
                .collect::<Vec<_>>()
        })
    }
}

/// A `{f1: v1, f2: v2, ...}` record literal expression at a value position.
///
/// Each field is a `RECORD_FIELD` child with an expression value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecordLiteral(SyntaxNode);

impl RecordLiteral {
    /// Cast from a raw `SyntaxNode`. Returns `Some` only for `RECORD_LITERAL` nodes.
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == RECORD_LITERAL {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Iterate over `RECORD_FIELD` children in declaration order.
    pub fn fields(&self) -> impl Iterator<Item = RecordField> + '_ {
        self.0.children().filter_map(RecordField::cast)
    }

    /// The text of this record literal as written in the source.
    pub fn text(&self) -> String {
        self.0.text().to_string()
    }
}

/// A `{f1: T1, f2: T2, ...}` inline-record type at a type-annotation position.
///
/// Produced by the parser when `{` appears in a type-annotation context (e.g.
/// `smelt.record Name = { ... }` body or `smelt.define foo(x: { ... }) AS ...`
/// parameter annotation).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecordTypeInline(SyntaxNode);

impl RecordTypeInline {
    /// Cast from a raw `SyntaxNode`. Returns `Some` only for `RECORD_TYPE_INLINE` nodes.
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == RECORD_TYPE_INLINE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Iterate over direct `RECORD_FIELD` children in declaration order.
    pub fn fields(&self) -> impl Iterator<Item = RecordField> + '_ {
        self.0.children().filter_map(RecordField::cast)
    }

    /// The text of this inline record type as written in the source.
    pub fn text(&self) -> String {
        self.0.text().to_string()
    }
}

/// A single `IDENT : ...` field inside a `RECORD_LITERAL`, `RECORD_TYPE_INLINE`,
/// or `SMELT_RECORD_DECL` body. The trailing value is either an expression (in
/// a literal) or a type reference (in a type context).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecordField(SyntaxNode);

impl RecordField {
    /// Cast from a raw `SyntaxNode`. Returns `Some` only for `RECORD_FIELD` nodes.
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == RECORD_FIELD {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The field name (the `IDENT` token before the `:`).
    pub fn name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// The value expression (only valid in a `RECORD_LITERAL` context).
    pub fn value_expr(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    /// The type reference (only valid in a type-annotation context).
    pub fn type_ref(&self) -> Option<TypeRef> {
        self.0.children().find_map(TypeRef::cast)
    }

    /// The inline-record type (only valid when the field's type is `{ ... }`).
    pub fn inline_record_type(&self) -> Option<RecordTypeInline> {
        self.0.children().find_map(RecordTypeInline::cast)
    }
}

/// A `expr.method(args)` call on a `Map<K, V>` typed expression.
///
/// Produced by the parser whenever it encounters `IDENT DOT IDENT (` and the
/// method name is one of the recognised Map API methods (entries, get, keys,
/// values, contains_key, insert, remove, len, is_empty, merge). Phase 4 type
/// inference validates that the LHS is actually `Map<K, V>` and emits
/// `MapApiUnknown` otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MapMethodCall(SyntaxNode);

impl MapMethodCall {
    /// Cast from a raw `SyntaxNode`. Returns `Some` only for `MAP_METHOD_CALL` nodes.
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == MAP_METHOD_CALL {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The name of the method (e.g. `"entries"`, `"get"`).
    ///
    /// Works for both receiver shapes:
    ///   - `IDENT(receiver) DOT IDENT(method) ARG_LIST`  — bare-identifier receiver
    ///   - `SMELT_PATH_CALL(receiver) DOT IDENT(method) ARG_LIST` — call receiver
    ///
    /// Finds the first `IDENT` token that appears after a `DOT` token.
    pub fn method_name(&self) -> Option<String> {
        let mut after_dot = false;
        for child in self.0.children_with_tokens() {
            match child {
                rowan::NodeOrToken::Token(tok) if tok.kind() == DOT => {
                    after_dot = true;
                }
                rowan::NodeOrToken::Token(tok) if after_dot && tok.kind() == IDENT => {
                    return Some(tok.text().to_string());
                }
                rowan::NodeOrToken::Token(tok)
                    if after_dot && !tok.kind().is_trivia() && tok.kind() != DOT =>
                {
                    break; // unexpected non-trivia non-DOT token after DOT
                }
                _ => {}
            }
        }
        None
    }

    /// The receiver expression (the expression before the dot).
    ///
    /// For bare-identifier receivers (`m.keys()`) this is the `IDENT`-wrapped
    /// `EXPRESSION` node.  For call-expression receivers
    /// (`smelt.config.load_yaml(...).keys()`) this is the `SMELT_PATH_CALL` node.
    pub fn receiver_expr(&self) -> Option<Expr> {
        // The receiver is the first child *node* of MAP_METHOD_CALL (before the DOT).
        self.0.children().next().and_then(Expr::cast)
    }

    /// The argument list, if present.
    pub fn arg_list(&self) -> Option<ArgList> {
        self.0.children().find_map(ArgList::cast)
    }

    /// The text of the whole call as written in the source.
    pub fn text(&self) -> String {
        self.0.text().to_string()
    }
}

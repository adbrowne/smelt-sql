/// Typed AST wrappers over Rowan CST
use crate::syntax_kind::SyntaxNode;
use crate::SyntaxKind::*;
use rowan::TextRange;

/// Root file node
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct File(SyntaxNode);

impl File {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == FILE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    pub fn select_stmt(&self) -> Option<SelectStmt> {
        self.0.children().find_map(SelectStmt::cast)
    }

    /// Find all ref('model') function calls in the file
    pub fn refs(&self) -> impl Iterator<Item = RefCall> + '_ {
        self.0
            .descendants()
            .filter_map(FunctionCall::cast)
            .filter_map(RefCall::from_function_call)
    }

    /// Find all source('source.table') function calls in the file
    pub fn sources(&self) -> impl Iterator<Item = SourceCall> + '_ {
        self.0
            .descendants()
            .filter_map(FunctionCall::cast)
            .filter_map(SourceCall::from_function_call)
    }

    /// Iterate over top-level `smelt.define` declarations in this file.
    pub fn defines(&self) -> impl Iterator<Item = SmeltDefine> + '_ {
        self.0.children().filter_map(SmeltDefine::cast)
    }

    /// Iterate over top-level `smelt.extern` declarations in this file.
    pub fn externs(&self) -> impl Iterator<Item = SmeltExtern> + '_ {
        self.0.children().filter_map(SmeltExtern::cast)
    }
}

// ===== smelt.define (Step 1, Phase 1) =====

/// Top-level `smelt.define name(params) [-> Type] AS (body)` declaration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SmeltDefine(SyntaxNode);

impl SmeltDefine {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SMELT_DEFINE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The declared function name (text of the single IDENT inside DEFINE_NAME).
    pub fn name(&self) -> Option<String> {
        let name_node = self.0.children().find(|n| n.kind() == DEFINE_NAME)?;
        name_node
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// The text range of the DEFINE_NAME node (the function name identifier).
    pub fn name_range(&self) -> Option<TextRange> {
        let name_node = self.0.children().find(|n| n.kind() == DEFINE_NAME)?;
        let ident = name_node
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)?;
        Some(ident.text_range())
    }

    /// The parameter list, if parsed successfully.
    pub fn param_list(&self) -> Option<ParamList> {
        self.0.children().find_map(ParamList::cast)
    }

    /// The declared return type, if any (the TypeRef inside a RETURN_ARROW node).
    pub fn return_type(&self) -> Option<TypeRef> {
        self.0
            .children()
            .find(|n| n.kind() == RETURN_ARROW)?
            .children()
            .find_map(TypeRef::cast)
    }

    /// The body expression block.
    pub fn body(&self) -> Option<DefineBody> {
        self.0.children().find_map(DefineBody::cast)
    }

    /// Byte offset at which this declaration starts in the source text.
    ///
    /// Since `strip_frontmatter` preserves byte offsets (each stripped
    /// line becomes `-- <spaces>` of the same byte length), offsets into
    /// the stripped text are identical to offsets into the raw text.
    /// Callers can therefore use this offset to look up a
    /// per-declaration frontmatter block in the raw source.
    pub fn source_offset(&self) -> usize {
        usize::from(self.0.text_range().start())
    }

    /// Text of the frontmatter block that immediately precedes this
    /// declaration in `raw_text`, if any.
    ///
    /// Returns `None` when there is no `---`/`---` block directly
    /// before the declaration, or when the gap between the block and
    /// the declaration contains SQL / another declaration.
    ///
    /// Introduced in Phase 11 (per-declaration frontmatter).
    pub fn frontmatter(&self, raw_text: &str) -> Option<String> {
        let off = self.source_offset();
        let attached = crate::attach_frontmatter_to_decls(raw_text, &[off]);
        attached.into_iter().next().flatten().map(|b| b.inner_text)
    }
}

/// Top-level `smelt.extern name(params) -> Type` declaration (Phase 10).
///
/// Shape mirrors [`SmeltDefine`] but without a body — externs bind a
/// user-chosen name to a backend-provided function. The return type is
/// mandatory (the extern signature is the only information the checker has
/// about the imported function).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SmeltExtern(SyntaxNode);

impl SmeltExtern {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SMELT_EXTERN {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The declared extern's function name.
    ///
    /// For the legacy single-IDENT form (`smelt.extern foo(...)`) this is
    /// just `foo`. For the dotted backend-namespace form
    /// (`smelt.extern duckdb.read_parquet(...)`), this returns
    /// `read_parquet` — the backend prefix is available separately via
    /// [`SmeltExtern::backend_namespace`].
    pub fn name(&self) -> Option<String> {
        let name_node = self.0.children().find(|n| n.kind() == DEFINE_NAME)?;
        let idents: Vec<_> = name_node
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == IDENT)
            .collect();
        idents.last().map(|t| t.text().to_string())
    }

    /// The text range of the extern's function-name identifier. For the
    /// dotted form this points at the second IDENT (the function name,
    /// not the backend prefix); diagnostics anchored here therefore
    /// underline the part users will care about.
    pub fn name_range(&self) -> Option<TextRange> {
        let name_node = self.0.children().find(|n| n.kind() == DEFINE_NAME)?;
        let idents: Vec<_> = name_node
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == IDENT)
            .collect();
        idents.last().map(|t| t.text_range())
    }

    /// The parameter list, if parsed successfully.
    pub fn param_list(&self) -> Option<ParamList> {
        self.0.children().find_map(ParamList::cast)
    }

    /// The declared return type, if any (the TypeRef inside a RETURN_ARROW node).
    pub fn return_type(&self) -> Option<TypeRef> {
        self.0
            .children()
            .find(|n| n.kind() == RETURN_ARROW)?
            .children()
            .find_map(TypeRef::cast)
    }

    /// Byte offset at which this declaration starts in the source text.
    /// See `SmeltDefine::source_offset` for the offset-stability rationale.
    pub fn source_offset(&self) -> usize {
        usize::from(self.0.text_range().start())
    }

    /// Optional backend namespace captured from a dotted extern name,
    /// e.g. `smelt.extern duckdb.read_parquet(...)` — returns
    /// `Some("duckdb")`. Single-segment extern names return `None`.
    ///
    /// Introduced in Phase 11 (backend namespace sugar for externs).
    pub fn backend_namespace(&self) -> Option<String> {
        // We stored the backend prefix as the *first* IDENT inside the
        // DEFINE_NAME node, followed by a DOT and a second IDENT whose
        // text is returned by `name()`. If the node only contains a
        // single IDENT (the legacy form), return None.
        let name_node = self.0.children().find(|n| n.kind() == DEFINE_NAME)?;
        let tokens: Vec<_> = name_node
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == IDENT || t.kind() == DOT)
            .collect();
        if tokens.len() >= 3
            && tokens[0].kind() == IDENT
            && tokens[1].kind() == DOT
            && tokens[2].kind() == IDENT
        {
            Some(tokens[0].text().to_string())
        } else {
            None
        }
    }

    /// Text of the frontmatter block that immediately precedes this
    /// declaration. See `SmeltDefine::frontmatter` for semantics.
    pub fn frontmatter(&self, raw_text: &str) -> Option<String> {
        let off = self.source_offset();
        let attached = crate::attach_frontmatter_to_decls(raw_text, &[off]);
        attached.into_iter().next().flatten().map(|b| b.inner_text)
    }
}

/// Parameter list of a `smelt.define`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParamList(SyntaxNode);

impl ParamList {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == PARAM_LIST {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Iterate over declared parameters.
    pub fn params(&self) -> impl Iterator<Item = Param> + '_ {
        self.0.children().filter_map(Param::cast)
    }
}

/// A single parameter inside a `smelt.define` parameter list.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Param(SyntaxNode);

impl Param {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == PARAM {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The parameter name (the first IDENT token of the PARAM node).
    pub fn name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// The text range of the parameter-name identifier token, if present.
    /// Used by Phase 5 to anchor duplicate-parameter-name diagnostics on the
    /// second occurrence's name span.
    pub fn name_range(&self) -> Option<TextRange> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text_range())
    }

    /// The parameter's declared type, if any.
    pub fn type_ref(&self) -> Option<TypeRef> {
        self.0.children().find_map(TypeRef::cast)
    }

    /// The parameter's default-value node, if any. Structured access will come
    /// in a later phase; for now callers can inspect the SyntaxNode directly.
    pub fn default_value(&self) -> Option<SyntaxNode> {
        self.0.children().find(|n| n.kind() == DEFAULT_VALUE)
    }

    /// The expression inside the DEFAULT_VALUE node, if any. Phase 6 uses this
    /// so the `fill-missing-arg` check can infer the default's type at the
    /// call site.
    pub fn default_value_expr(&self) -> Option<Expr> {
        self.default_value()
            .and_then(|dv| dv.children().find_map(Expr::cast))
    }
}

/// Flat type reference. Phase 4 parses the text into a structured
/// [`smelt_types::SmeltType`]; Phase 13 adds structured CST children for
/// the non-`Expr` sorts (`TableExpr`, `AggExpr`, `WindowExpr`,
/// `SelectItems`) so downstream phases can walk them without re-parsing
/// the raw text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeRef(SyntaxNode);

/// Classification of a `TYPE_REF`'s leading sort keyword.
///
/// Phase 13 recognises these heads inside `parse_type_ref` and emits
/// structured CST children where appropriate. `Other` captures sort
/// keywords that are not one of the recognised heads — in practice an
/// unknown head is reported as a parse error, but a post-parse walker
/// may still see the unknown name here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeRefHead {
    /// `Expr<...>` — scalar-expression fragment sort.
    Expr,
    /// `AggExpr<...>` — aggregate-expression fragment sort.
    AggExpr,
    /// `WindowExpr<...>` — window-expression fragment sort.
    WindowExpr,
    /// `TableExpr` / `TableExpr<{...}>` — table-expression fragment sort.
    TableExpr,
    /// `SelectItems<...>` — select-list fragment sort.
    SelectItems,
    /// Unknown or missing sort keyword; the raw identifier text is carried
    /// for diagnostic callers. `None` means the `TYPE_REF` had no leading
    /// identifier (error-recovery shape).
    Other(Option<String>),
}

/// Kind tag emitted on `Expr<T>` / `AggExpr<T>` / `WindowExpr<T>` type refs.
///
/// Phase 14 attaches this kind to every typed AST node during inference;
/// Phase 13 only records it on the signature's parameter type refs so the
/// signature extractor can thread the kind through without re-parsing the
/// raw text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExprKindTag {
    /// `Expr<T>` — scalar expression.
    Scalar,
    /// `AggExpr<T>` — aggregate expression.
    Agg,
    /// `WindowExpr<T>` — window expression.
    Window,
}

/// Trailing row-polymorphism marker inside a `ROW_REQUIREMENT`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowTail {
    /// No tail was written.
    None,
    /// `..` — anonymous fresh row variable, cannot be referenced.
    Anon,
    /// `..name` — named row variable.
    Named(String),
}

/// Structured view over a `ROW_REQUIREMENT` child of a `TableExpr<{...}>`
/// type reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RowRequirement(SyntaxNode);

/// A single `name: TypeRef` field inside a `ROW_REQUIREMENT`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RowField(SyntaxNode);

impl TypeRef {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == TYPE_REF {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Raw text of the type reference (including internal whitespace).
    pub fn text(&self) -> String {
        self.0.text().to_string()
    }

    /// Classify the `TYPE_REF`'s leading sort keyword.
    ///
    /// Looks at the first `IDENT` token in document order (skipping any
    /// error / trivia tokens). Returns [`TypeRefHead::Other(None)`] if
    /// no leading identifier is present.
    pub fn kind(&self) -> TypeRefHead {
        let head = leading_ident_text(&self.0);
        match head.as_deref() {
            Some("Expr") => TypeRefHead::Expr,
            Some("AggExpr") => TypeRefHead::AggExpr,
            Some("WindowExpr") => TypeRefHead::WindowExpr,
            Some("TableExpr") => TypeRefHead::TableExpr,
            Some("SelectItems") => TypeRefHead::SelectItems,
            Some(other) => TypeRefHead::Other(Some(other.to_string())),
            None => TypeRefHead::Other(None),
        }
    }

    /// Return the [`ExprKindTag`] attached to this type ref, if any.
    ///
    /// Populated only for `Expr<T>`, `AggExpr<T>`, and `WindowExpr<T>`
    /// heads. Other heads (TableExpr, SelectItems, etc.) return `None`.
    pub fn expr_kind(&self) -> Option<ExprKindTag> {
        for child in self.0.children() {
            match child.kind() {
                EXPR_KIND_SCALAR => return Some(ExprKindTag::Scalar),
                EXPR_KIND_AGG => return Some(ExprKindTag::Agg),
                EXPR_KIND_WINDOW => return Some(ExprKindTag::Window),
                _ => {}
            }
        }
        None
    }

    /// Return the structured row requirement of a `TableExpr<{...}>`, if
    /// present. Only `TableExpr` heads carry a `ROW_REQUIREMENT` child.
    pub fn row_requirement(&self) -> Option<RowRequirement> {
        self.0
            .children()
            .find(|n| n.kind() == ROW_REQUIREMENT)
            .map(RowRequirement)
    }

    /// The `SELECTITEMS_KIND` argument text (e.g. `"Agg"`), if any.
    pub fn selectitems_kind(&self) -> Option<String> {
        let node = self.0.children().find(|n| n.kind() == SELECTITEMS_KIND)?;
        node.children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// The `SELECTITEMS_CTX` argument text (e.g. `"sessionized"`), if any.
    pub fn selectitems_ctx(&self) -> Option<String> {
        let node = self.0.children().find(|n| n.kind() == SELECTITEMS_CTX)?;
        node.children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// The `EXPR_CTX` context-binding identifier (e.g. `"source"`) from
    /// `Expr<Boolean, source>`, if present (Phase 19).
    pub fn expr_ctx(&self) -> Option<String> {
        let node = self.0.children().find(|n| n.kind() == EXPR_CTX)?;
        node.children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }
}

/// First IDENT token in the sub-tree's document order, if any.
fn leading_ident_text(node: &SyntaxNode) -> Option<String> {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == IDENT)
        .map(|t| t.text().to_string())
}

impl RowRequirement {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == ROW_REQUIREMENT {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// All `ROW_FIELD` children in declared order.
    pub fn fields(&self) -> Vec<RowField> {
        self.0.children().filter_map(RowField::cast).collect()
    }

    /// The trailing row variable marker, if any.
    pub fn tail(&self) -> RowTail {
        if self.0.children().any(|n| n.kind() == ROW_TAIL_ANON) {
            return RowTail::Anon;
        }
        if let Some(named) = self.0.children().find(|n| n.kind() == ROW_TAIL_NAMED) {
            if let Some(name) = named
                .children_with_tokens()
                .filter_map(|e| e.into_token())
                .find(|t| t.kind() == IDENT)
                .map(|t| t.text().to_string())
            {
                return RowTail::Named(name);
            }
        }
        RowTail::None
    }
}

impl RowField {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == ROW_FIELD {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The field's declared name, if present.
    pub fn name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// The field's declared `TYPE_REF`, if present.
    pub fn type_ref(&self) -> Option<TypeRef> {
        self.0.children().find_map(TypeRef::cast)
    }
}

/// Parenthesized body expression of a `smelt.define`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DefineBody(SyntaxNode);

impl DefineBody {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == DEFINE_BODY {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The first expression-like child of the body, if any.
    pub fn expression(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    /// The body's SELECT statement, if the body is shaped as a bare
    /// top-level SELECT (e.g. a `TableExpr`-returning define whose
    /// body is `(SELECT ... FROM source)`). Distinct from
    /// [`DefineBody::expression`] which returns `None` for SELECT-shaped
    /// bodies because `Expr::cast(SELECT_STMT)` does not recognise
    /// SELECT as an expression in this grammar.
    pub fn select_stmt(&self) -> Option<SelectStmt> {
        self.0.children().find_map(SelectStmt::cast)
    }
}

// ===== smelt.fn.* user-declared function call (Phase 2) =====

/// `smelt.fn.<path>(args)` call node. Distinct from `FUNCTION_CALL` — this
/// node is only produced for calls that start with the literal `smelt.fn.`
/// prefix, which names user-declared functions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SmeltFnCall(SyntaxNode);

impl SmeltFnCall {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SMELT_FN_CALL {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The dotted call path node (`smelt.fn.<...>.name`), if present.
    pub fn call_path(&self) -> Option<CallPath> {
        self.0.children().find_map(CallPath::cast)
    }

    /// The argument list node (`(args)`), if present.
    pub fn arg_list(&self) -> Option<ArgList> {
        self.0.children().find_map(ArgList::cast)
    }

    /// The full dotted path text including the `smelt.fn.` prefix, joined
    /// with `.` and with whitespace/trivia stripped — e.g.
    /// `smelt.fn.core.safe_divide`.
    pub fn path_text(&self) -> String {
        match self.call_path() {
            Some(p) => {
                p.0.children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .filter(|t| t.kind() == IDENT)
                    .map(|t| t.text().to_string())
                    .collect::<Vec<_>>()
                    .join(".")
            }
            None => String::new(),
        }
    }

    /// The text range of the `CALL_PATH` node (the dotted path including the
    /// `smelt.fn.` prefix). Phase 6 anchors `UnknownSmeltFn` diagnostics here.
    pub fn call_path_range(&self) -> Option<TextRange> {
        self.call_path().map(|p| p.0.text_range())
    }

    /// Text range of the whole SMELT_FN_CALL node (path + args). Used as a
    /// fallback anchor when the call path is missing.
    pub fn text_range(&self) -> TextRange {
        self.0.text_range()
    }
}

/// The dotted path inside a `SMELT_FN_CALL` — includes the literal
/// `smelt.fn.` prefix tokens as well as all subsequent namespace / name
/// segments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CallPath(SyntaxNode);

impl CallPath {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == CALL_PATH {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The logical path segments AFTER the `smelt.fn.` prefix. For
    /// `smelt.fn.core.math.safe_divide` this returns
    /// `["core", "math", "safe_divide"]`.
    pub fn segments(&self) -> Vec<String> {
        let idents: Vec<String> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
            .collect();
        // Drop the leading `smelt` and `fn` tokens (the prefix). If for some
        // reason the path is shorter than expected (e.g. an error-recovery
        // case) we just return whatever remains.
        idents.into_iter().skip(2).collect()
    }
}

/// Argument list node (`(arg, arg, ...)`) used by both `FUNCTION_CALL` and
/// `SMELT_FN_CALL`. Minimal wrapper in this phase — callers that need the
/// richer `FunctionCall::named_params` helper can continue to use that
/// wrapper directly.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArgList(SyntaxNode);

impl ArgList {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == ARG_LIST {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Iterate over the direct `NAMED_PARAM` children of this argument list.
    pub fn named_params(&self) -> impl Iterator<Item = NamedParam> + '_ {
        self.0.children().filter_map(NamedParam::cast)
    }

    /// Iterate over positional (non-named) expression arguments in this arg
    /// list, in source order. NAMED_PARAM children are skipped — callers that
    /// want both should iterate `named_params()` separately.
    pub fn positional_args(&self) -> Vec<Expr> {
        self.0
            .children()
            .filter(|n| n.kind() != NAMED_PARAM)
            .filter_map(Expr::cast)
            .collect()
    }
}

/// SELECT statement
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SelectStmt(SyntaxNode);

impl SelectStmt {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SELECT_STMT {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn with_clause(&self) -> Option<WithClause> {
        self.0.children().find_map(WithClause::cast)
    }

    pub fn select_list(&self) -> Option<SelectList> {
        self.0.children().find_map(SelectList::cast)
    }

    pub fn from_clause(&self) -> Option<FromClause> {
        self.0.children().find_map(FromClause::cast)
    }

    pub fn where_clause(&self) -> Option<WhereClause> {
        self.0.children().find_map(WhereClause::cast)
    }

    pub fn group_by_clause(&self) -> Option<GroupByClause> {
        self.0.children().find_map(GroupByClause::cast)
    }

    pub fn having_clause(&self) -> Option<HavingClause> {
        self.0.children().find_map(HavingClause::cast)
    }

    pub fn qualify_clause(&self) -> Option<QualifyClause> {
        self.0.children().find_map(QualifyClause::cast)
    }

    pub fn order_by_clause(&self) -> Option<OrderByClause> {
        self.0.children().find_map(OrderByClause::cast)
    }

    pub fn limit_clause(&self) -> Option<LimitClause> {
        self.0.children().find_map(LimitClause::cast)
    }

    pub fn is_distinct(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == DISTINCT_KW)
    }

    /// Get the underlying syntax node.
    ///
    /// Originally `pub(crate)` and named for the printer module; made
    /// `pub` in Phase 15 so the SELECT-body walker in smelt-db can
    /// descendants-walk a SELECT_STMT body to dispatch nested
    /// `smelt.fn.*` calls.
    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Check if this SELECT has a UNION clause
    pub fn has_union(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == UNION_KW)
    }

    /// Check if the UNION is UNION ALL (vs regular UNION which removes duplicates)
    pub fn is_union_all(&self) -> bool {
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .collect();

        for (i, token) in tokens.iter().enumerate() {
            if token.kind() == UNION_KW {
                // Skip whitespace to find next meaningful token
                for next_token in &tokens[i + 1..] {
                    match next_token.kind() {
                        WHITESPACE | COMMENT => continue,
                        ALL_KW => return true,
                        _ => break,
                    }
                }
            }
        }
        false
    }

    /// Get the SELECT statement after UNION (if any)
    pub fn union_select(&self) -> Option<SelectStmt> {
        let mut found_union = false;

        for child in self.0.children_with_tokens() {
            if let Some(token) = child.as_token() {
                if token.kind() == UNION_KW {
                    found_union = true;
                }
            } else if found_union {
                if let Some(n) = child.as_node() {
                    if n.kind() == SELECT_STMT {
                        return SelectStmt::cast(n.clone());
                    }
                }
            }
        }
        None
    }

    /// Check if this SELECT has any set operation (UNION, INTERSECT, or EXCEPT)
    pub fn has_set_operation(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| matches!(t.kind(), UNION_KW | INTERSECT_KW | EXCEPT_KW))
    }

    /// Get the SELECT statement after any set operation (UNION, INTERSECT, or EXCEPT)
    pub fn set_operation_select(&self) -> Option<SelectStmt> {
        let mut found_set_op = false;

        for child in self.0.children_with_tokens() {
            if let Some(token) = child.as_token() {
                if matches!(token.kind(), UNION_KW | INTERSECT_KW | EXCEPT_KW) {
                    found_set_op = true;
                }
            } else if found_set_op {
                if let Some(n) = child.as_node() {
                    if n.kind() == SELECT_STMT {
                        return SelectStmt::cast(n.clone());
                    }
                }
            }
        }
        None
    }
}

/// SELECT list (columns)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SelectList(SyntaxNode);

impl SelectList {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SELECT_LIST {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn items(&self) -> impl Iterator<Item = SelectItem> + '_ {
        self.0.children().filter_map(SelectItem::cast)
    }
}

/// SELECT item (column or expression with optional alias)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SelectItem(SyntaxNode);

impl SelectItem {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SELECT_ITEM {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the expression node for this select item
    pub fn expression(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    /// Get the alias if present (explicit `AS alias` or implicit `expr alias`)
    pub fn alias(&self) -> Option<String> {
        let mut found_as = false;
        let mut found_expr = false;

        for child in self.0.children_with_tokens() {
            match &child {
                rowan::NodeOrToken::Token(token) => {
                    if token.kind() == AS_KW {
                        found_as = true;
                    } else if token.kind() == IDENT {
                        if found_as {
                            // Explicit alias: `expr AS alias`
                            return Some(token.text().to_string());
                        } else if found_expr {
                            // Implicit alias: `expr alias` (IDENT after expression node)
                            return Some(token.text().to_string());
                        }
                    }
                }
                rowan::NodeOrToken::Node(_) => {
                    found_expr = true;
                }
            }
        }
        None
    }

    /// Get the text range of the alias token, if present
    pub fn alias_range(&self) -> Option<TextRange> {
        let mut found_as = false;
        let mut found_expr = false;

        for child in self.0.children_with_tokens() {
            match &child {
                rowan::NodeOrToken::Token(token) => {
                    if token.kind() == AS_KW {
                        found_as = true;
                    } else if token.kind() == IDENT && (found_as || found_expr) {
                        return Some(token.text_range());
                    }
                }
                rowan::NodeOrToken::Node(_) => {
                    found_expr = true;
                }
            }
        }
        None
    }

    /// Get the effective column name (alias if present, otherwise inferred from expression)
    pub fn column_name(&self) -> Option<String> {
        // If there's an alias, use it
        if let Some(alias) = self.alias() {
            return Some(alias);
        }

        // Otherwise, try to infer from expression
        if let Some(expr) = self.expression() {
            expr.infer_name()
        } else {
            None
        }
    }

    /// Check if this select item is a wildcard (*)
    pub fn is_wildcard(&self) -> bool {
        self.0.children_with_tokens().any(|child| {
            child
                .as_token()
                .is_some_and(|t| t.kind() == STAR || t.kind() == MULTIPLY)
        }) && self.expression().is_none()
    }

    /// If this select item is a qualified wildcard `<qualifier>.*`,
    /// return the qualifier identifier text. Returns `None` for a bare
    /// `*` (see [`Self::is_wildcard`]) or any non-wildcard item.
    ///
    /// The parser emits a `SELECT_ITEM` whose tokens are `IDENT DOT
    /// STAR` for this shape (no wrapping `EXPRESSION` node). Phase 17
    /// uses this accessor to expand `source.*` inside a
    /// `TableExpr`-returning function body.
    pub fn qualified_wildcard_target(&self) -> Option<String> {
        if !self.is_wildcard() {
            return None;
        }
        // Walk tokens; if we see `IDENT DOT STAR` before the STAR, the
        // IDENT is the qualifier. Bare `*` has no leading IDENT.
        let mut last_ident: Option<String> = None;
        let mut last_was_dot = false;
        for child in self.0.children_with_tokens() {
            if let Some(token) = child.as_token() {
                match token.kind() {
                    IDENT => {
                        last_ident = Some(token.text().to_string());
                        last_was_dot = false;
                    }
                    DOT => {
                        last_was_dot = true;
                    }
                    STAR | MULTIPLY => {
                        if last_was_dot {
                            return last_ident;
                        }
                        return None;
                    }
                    _ => {}
                }
            }
        }
        None
    }

    /// Get the text range of this select item
    pub fn range(&self) -> TextRange {
        self.0.text_range()
    }

    /// Get the underlying syntax node (for printer)
    #[allow(dead_code)] // Used by printer module
    pub(crate) fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

/// FROM clause
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FromClause(SyntaxNode);

impl FromClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == FROM_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn table_refs(&self) -> impl Iterator<Item = TableRef> + '_ {
        self.0.children().filter_map(TableRef::cast)
    }

    pub fn joins(&self) -> impl Iterator<Item = JoinClause> + '_ {
        self.0.children().filter_map(JoinClause::cast)
    }

    /// Get the text range of this FROM clause
    pub fn text_range(&self) -> TextRange {
        self.0.text_range()
    }

    /// Get the full text of this FROM clause
    pub fn text(&self) -> String {
        self.0.text().to_string()
    }
}

/// JOIN clause (JOIN type + table + condition)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JoinClause(SyntaxNode);

impl JoinClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == JOIN_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the JOIN type (INNER, LEFT, RIGHT, FULL, CROSS)
    /// Returns None for bare JOIN (defaults to INNER)
    pub fn join_type(&self) -> Option<JoinType> {
        for token in self.0.children_with_tokens().filter_map(|e| e.into_token()) {
            match token.kind() {
                INNER_KW => return Some(JoinType::Inner),
                LEFT_KW => return Some(JoinType::Left),
                RIGHT_KW => return Some(JoinType::Right),
                FULL_KW => return Some(JoinType::Full),
                CROSS_KW => return Some(JoinType::Cross),
                _ => continue,
            }
        }
        None // Bare JOIN, defaults to INNER
    }

    /// Get the table reference being joined
    pub fn table_ref(&self) -> Option<TableRef> {
        self.0.children().find_map(TableRef::cast)
    }

    /// Get the join condition (ON or USING clause)
    pub fn condition(&self) -> Option<JoinCondition> {
        self.0.children().find_map(JoinCondition::cast)
    }
}

/// JOIN type enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Full,
    Cross,
}

/// JOIN condition (ON expr or USING cols)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JoinCondition(SyntaxNode);

impl JoinCondition {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == JOIN_CONDITION {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Check if this is an ON condition (vs USING)
    pub fn is_on(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == ON_KW)
    }

    /// Check if this is a USING condition
    pub fn is_using(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == USING_KW)
    }

    /// Get the ON expression (if this is an ON condition)
    pub fn on_expression(&self) -> Option<Expr> {
        if self.is_on() {
            self.0.children().find_map(Expr::cast)
        } else {
            None
        }
    }

    /// Get the USING column list as strings
    pub fn using_columns(&self) -> Vec<String> {
        if !self.is_using() {
            return Vec::new();
        }

        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
            .collect()
    }
}

/// Table reference (identifier or template expression)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TableRef(SyntaxNode);

impl TableRef {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == TABLE_REF {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Check if this is a function call reference (like ref('model'))
    pub fn is_function_call(&self) -> bool {
        self.0.children().any(|n| n.kind() == FUNCTION_CALL)
    }

    /// Get the function call if this table ref is a function (like ref('model'))
    pub fn function_call(&self) -> Option<FunctionCall> {
        self.0.children().find_map(FunctionCall::cast)
    }

    /// Get the `smelt.fn.*` call if this table ref is one (Phase 15+).
    ///
    /// `SMELT_FN_CALL` nodes are distinct from `FUNCTION_CALL`; the
    /// parser emits them for calls starting with the `smelt.fn.`
    /// prefix. Phase 17 uses this accessor to resolve a
    /// `TableExpr`-returning call in FROM position to its inferred
    /// output schema.
    pub fn smelt_fn_call(&self) -> Option<SmeltFnCall> {
        self.0.children().find_map(SmeltFnCall::cast)
    }

    pub fn identifier(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// Get the alias if present (explicit AS alias or implicit alias after table ref)
    pub fn alias(&self) -> Option<String> {
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .collect();

        // Look for AS keyword followed by IDENT
        let mut found_as = false;
        let mut last_ident: Option<String> = None;
        let mut ident_count = 0;

        for token in &tokens {
            match token.kind() {
                AS_KW => found_as = true,
                IDENT => {
                    ident_count += 1;
                    if found_as {
                        // This is the explicit alias after AS
                        return Some(token.text().to_string());
                    }
                    last_ident = Some(token.text().to_string());
                }
                _ => {}
            }
        }

        // If we have more than one IDENT and no AS keyword, the last IDENT is an implicit alias
        // But we need to be careful: for function calls like smelt.source('raw.users'),
        // we don't want to return 'smelt' or 'source' as aliases
        if !found_as && ident_count > 1 && !self.is_function_call() {
            return last_ident;
        }

        // For function calls with implicit alias (smelt.source('raw.users') t),
        // check if the last token is an IDENT that's not part of the function call
        if self.is_function_call() {
            // Get the function call's text range
            if let Some(func) = self.function_call() {
                let func_range = func.syntax().text_range();
                // Check if last_ident is after the function call
                for token in tokens.iter().rev() {
                    if token.kind() == IDENT {
                        let token_start = token.text_range().start();
                        if token_start >= func_range.end() {
                            return Some(token.text().to_string());
                        }
                        break;
                    }
                }
            }
        }

        // For subqueries with implicit alias (LATERAL (...) alias_name),
        // check if the last token is an IDENT that's after the subquery
        if self.subquery().is_some() {
            if let Some(subquery) = self.subquery() {
                let subquery_range = subquery.0.text_range();
                // Check if last_ident is after the subquery
                for token in tokens.iter().rev() {
                    if token.kind() == IDENT {
                        let token_start = token.text_range().start();
                        if token_start > subquery_range.end() {
                            return Some(token.text().to_string());
                        }
                        break;
                    }
                }
            }
        }

        None
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Check if this table reference is LATERAL (allows correlated subquery)
    pub fn is_lateral(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == LATERAL_KW)
    }

    /// Get the subquery if this table reference contains one
    pub fn subquery(&self) -> Option<Subquery> {
        self.0.children().find_map(Subquery::cast)
    }
}

/// WHERE clause
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WhereClause(SyntaxNode);

impl WhereClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == WHERE_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the expression in this WHERE clause
    pub fn expression(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    /// Get the text range of this WHERE clause
    pub fn text_range(&self) -> TextRange {
        self.0.text_range()
    }

    /// Get the full text of this WHERE clause
    #[allow(dead_code)] // Keep for debugging, but prefer expression()
    pub fn text(&self) -> String {
        self.0.text().to_string()
    }
}

/// Expression node (represents any SQL expression)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Expr(SyntaxNode);

impl Expr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        // Accept any node that looks like an expression
        match node.kind() {
            EXPRESSION => {
                // Unwrap nested EXPRESSION wrappers to the innermost one.
                // parse_expression() wraps in EXPRESSION, and parse_primary_expr()
                // also wraps bare atoms in EXPRESSION — this avoids double-wrapping
                // issues in accessors that look for direct-child tokens.
                let mut inner = node;
                loop {
                    let children: Vec<_> = inner.children().collect();
                    if children.len() == 1 && children[0].kind() == EXPRESSION {
                        inner = children.into_iter().next().unwrap();
                    } else {
                        break;
                    }
                }
                Some(Self(inner))
            }
            BINARY_EXPR | FUNCTION_CALL | CASE_EXPR | CAST_EXPR | EXTRACT_EXPR | SUBQUERY
            | BETWEEN_EXPR | IN_EXPR | EXISTS_EXPR | SMELT_FN_CALL => Some(Self(node)),
            _ => {
                // Also try to wrap the node if it contains expression-like children
                if node.children().any(|n| {
                    matches!(
                        n.kind(),
                        EXPRESSION
                            | BINARY_EXPR
                            | FUNCTION_CALL
                            | CASE_EXPR
                            | CAST_EXPR
                            | EXTRACT_EXPR
                            | SUBQUERY
                            | BETWEEN_EXPR
                            | IN_EXPR
                            | EXISTS_EXPR
                            | SMELT_FN_CALL
                    )
                }) {
                    Some(Self(node))
                } else {
                    None
                }
            }
        }
    }

    /// Try to infer a column name from this expression
    /// Used when there's no explicit alias
    pub fn infer_name(&self) -> Option<String> {
        // Check for wildcard (*)
        if self.text().trim() == "*" {
            return Some("*".to_string());
        }

        // CAST(<inner> AS <type>) — propagate the inner expression's name so
        // an aliased SELECT like `SUM(x)::DOUBLE AS revenue` keeps `revenue`,
        // and a bare `CAST(line_gross AS DOUBLE)` keeps `line_gross`. Without
        // this branch, callers fall through to the IDENT fallback below and
        // pick up the type-spec identifier ("DOUBLE"), or to the SELECT-list
        // numeric placeholder (`_col1`) — both wrong.
        if let Some(cast) = self.as_cast() {
            if let Some(inner) = cast.expression() {
                if let Some(name) = inner.infer_name() {
                    return Some(name);
                }
            }
        }

        // Check if this is a function call
        if let Some(_func) = self.as_function_call() {
            // For function calls without alias, use the full function text
            return Some(self.text());
        }

        // Check if this is a simple column reference
        if let Some(col_ref) = self.as_column_ref() {
            // For qualified names (table.column), use just the column part
            return Some(col_ref.name().to_string());
        }

        // For other complex expressions, try to find the first identifier
        for child in self.0.children_with_tokens() {
            if let Some(token) = child.as_token() {
                if token.kind() == IDENT {
                    return Some(token.text().to_string());
                }
            }
        }

        None
    }

    /// Get the underlying syntax node
    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Get the full text of this expression
    pub fn text(&self) -> String {
        self.0.text().to_string()
    }

    /// Get the text range of this expression
    pub fn text_range(&self) -> TextRange {
        self.0.text_range()
    }

    /// Check if this is a simple column reference (identifier possibly qualified)
    pub fn as_column_ref(&self) -> Option<ColumnRef> {
        ColumnRef::from_expr(self)
    }

    /// Check if this is a function call
    pub fn as_function_call(&self) -> Option<FunctionCall> {
        self.0.children().find_map(FunctionCall::cast).or_else(|| {
            // Check if this node itself is a function call
            FunctionCall::cast(self.0.clone())
        })
    }

    /// Check if this is a `smelt.fn.*` user-declared function call. Distinct
    /// from `as_function_call()` — SQL built-in function calls produce a
    /// `FUNCTION_CALL` node; `smelt.fn.*` calls produce a `SMELT_FN_CALL`.
    pub fn as_smelt_fn_call(&self) -> Option<SmeltFnCall> {
        self.0
            .children()
            .find_map(SmeltFnCall::cast)
            .or_else(|| SmeltFnCall::cast(self.0.clone()))
    }

    /// Check if this is a CASE expression
    pub fn as_case(&self) -> Option<CaseExpr> {
        self.0
            .children()
            .find_map(CaseExpr::cast)
            .or_else(|| CaseExpr::cast(self.0.clone()))
    }

    /// Check if this is an EXTRACT expression
    pub fn as_extract(&self) -> Option<ExtractExpr> {
        self.0
            .children()
            .find_map(ExtractExpr::cast)
            .or_else(|| ExtractExpr::cast(self.0.clone()))
    }

    /// Check if this is a CAST expression
    pub fn as_cast(&self) -> Option<CastExpr> {
        self.0
            .children()
            .find_map(CastExpr::cast)
            .or_else(|| CastExpr::cast(self.0.clone()))
    }

    /// Check if this is a subquery
    pub fn as_subquery(&self) -> Option<Subquery> {
        self.0
            .children()
            .find_map(Subquery::cast)
            .or_else(|| Subquery::cast(self.0.clone()))
    }

    /// Check if this is a BETWEEN expression
    pub fn as_between(&self) -> Option<BetweenExpr> {
        self.0
            .children()
            .find_map(BetweenExpr::cast)
            .or_else(|| BetweenExpr::cast(self.0.clone()))
    }

    /// Check if this is an IN expression
    pub fn as_in(&self) -> Option<InExpr> {
        self.0
            .children()
            .find_map(InExpr::cast)
            .or_else(|| InExpr::cast(self.0.clone()))
    }

    /// Check if this is an EXISTS expression
    pub fn as_exists(&self) -> Option<ExistsExpr> {
        self.0
            .children()
            .find_map(ExistsExpr::cast)
            .or_else(|| ExistsExpr::cast(self.0.clone()))
    }

    /// Check if this is a binary expression
    pub fn as_binary(&self) -> Option<BinaryExpr> {
        self.0
            .children()
            .find_map(BinaryExpr::cast)
            .or_else(|| BinaryExpr::cast(self.0.clone()))
    }

    /// Check if this is an array literal (ARRAY[1, 2, 3])
    pub fn as_array_literal(&self) -> Option<ArrayLiteral> {
        self.0
            .children()
            .find_map(ArrayLiteral::cast)
            .or_else(|| ArrayLiteral::cast(self.0.clone()))
    }

    /// Check if this contains an array subscript (expr[index])
    pub fn as_array_subscript(&self) -> Option<ArraySubscript> {
        self.0
            .children()
            .find_map(ArraySubscript::cast)
            .or_else(|| ArraySubscript::cast(self.0.clone()))
    }

    /// Check if this contains an array slice (expr[start:end])
    pub fn as_array_slice(&self) -> Option<ArraySlice> {
        self.0
            .children()
            .find_map(ArraySlice::cast)
            .or_else(|| ArraySlice::cast(self.0.clone()))
    }

    /// Check if this is a ROW constructor (ROW(1, 2, 3))
    pub fn as_row_constructor(&self) -> Option<RowConstructor> {
        self.0
            .children()
            .find_map(RowConstructor::cast)
            .or_else(|| RowConstructor::cast(self.0.clone()))
    }

    /// Check if this is a struct literal (STRUCT(1 AS a, 'hello' AS b))
    pub fn as_struct_literal(&self) -> Option<StructLiteral> {
        self.0
            .children()
            .find_map(StructLiteral::cast)
            .or_else(|| StructLiteral::cast(self.0.clone()))
    }

    /// Check if this expression has a window specification (OVER clause)
    pub fn window_spec(&self) -> Option<WindowSpec> {
        self.0.children().find_map(WindowSpec::cast)
    }
}

/// Column reference (identifier, possibly qualified like "table.column")
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ColumnRef {
    qualifier: Option<String>,
    name: String,
}

impl ColumnRef {
    /// Try to parse a column reference from an expression
    pub fn from_expr(expr: &Expr) -> Option<Self> {
        let tokens: Vec<_> = expr
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == IDENT || t.kind() == DOT)
            .collect();

        if tokens.is_empty() {
            return None;
        }

        // Simple identifier
        if tokens.len() == 1 && tokens[0].kind() == IDENT {
            return Some(ColumnRef {
                qualifier: None,
                name: tokens[0].text().to_string(),
            });
        }

        // Qualified identifier: table.column
        if tokens.len() >= 3
            && tokens[0].kind() == IDENT
            && tokens[1].kind() == DOT
            && tokens[2].kind() == IDENT
        {
            return Some(ColumnRef {
                qualifier: Some(tokens[0].text().to_string()),
                name: tokens[2].text().to_string(),
            });
        }

        None
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn qualifier(&self) -> Option<&str> {
        self.qualifier.as_deref()
    }
}

/// Binary expression (e.g., a + b, x AND y, col = 'value')
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BinaryExpr(SyntaxNode);

impl BinaryExpr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == BINARY_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the underlying syntax node
    pub fn node(&self) -> &SyntaxNode {
        &self.0
    }

    /// Get the left operand expression
    pub fn left(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    /// Get the right operand expression
    pub fn right(&self) -> Option<Expr> {
        self.0.children().filter_map(Expr::cast).nth(1)
    }

    /// Get the operator as a string
    pub fn operator(&self) -> Option<String> {
        for child in self.0.children_with_tokens() {
            if let Some(token) = child.as_token() {
                match token.kind() {
                    PLUS => return Some("+".to_string()),
                    MINUS => return Some("-".to_string()),
                    STAR | MULTIPLY => return Some("*".to_string()),
                    DIVIDE => return Some("/".to_string()),
                    PERCENT => return Some("%".to_string()),
                    EQ => return Some("=".to_string()),
                    NE => return Some("<>".to_string()),
                    LT => return Some("<".to_string()),
                    GT => return Some(">".to_string()),
                    LE => return Some("<=".to_string()),
                    GE => return Some(">=".to_string()),
                    CONCAT => return Some("||".to_string()),
                    AND_KW => return Some("AND".to_string()),
                    OR_KW => return Some("OR".to_string()),
                    IS_KW => return Some("IS".to_string()),
                    NOT_KW => return Some("NOT".to_string()),
                    LIKE_KW => return Some("LIKE".to_string()),
                    ILIKE_KW => return Some("ILIKE".to_string()),
                    TILDE => return Some("~".to_string()),
                    TILDE_STAR => return Some("~*".to_string()),
                    NOT_TILDE => return Some("!~".to_string()),
                    NOT_TILDE_STAR => return Some("!~*".to_string()),
                    JSON_ARROW => return Some("->".to_string()),
                    JSON_ARROW_TEXT => return Some("->>".to_string()),
                    HASH_ARROW => return Some("#>".to_string()),
                    HASH_ARROW_TEXT => return Some("#>>".to_string()),
                    _ => {}
                }
            }
        }
        None
    }

    /// Check if this is a unary expression (e.g., -x, NOT y)
    /// Unary expressions have no right operand
    pub fn is_unary(&self) -> bool {
        self.right().is_none()
    }

    /// Get the unary operand as a column reference
    /// For unary expressions where the operand is a simple identifier (not wrapped in a node),
    /// this extracts the column reference from tokens.
    pub fn unary_operand_column(&self) -> Option<ColumnRef> {
        // First try getting as a normal expression
        if let Some(expr) = self.left() {
            return expr.as_column_ref();
        }

        // For unary expressions, the operand might be a bare identifier token
        // not wrapped in an expression node. Extract column ref from tokens.
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == IDENT || t.kind() == DOT)
            .collect();

        if tokens.is_empty() {
            return None;
        }

        // Simple identifier
        if tokens.len() == 1 && tokens[0].kind() == IDENT {
            return Some(ColumnRef {
                qualifier: None,
                name: tokens[0].text().to_string(),
            });
        }

        // Qualified identifier: table.column
        if tokens.len() >= 3
            && tokens[0].kind() == IDENT
            && tokens[1].kind() == DOT
            && tokens[2].kind() == IDENT
        {
            return Some(ColumnRef {
                qualifier: Some(tokens[0].text().to_string()),
                name: tokens[2].text().to_string(),
            });
        }

        None
    }
}

/// Function call expression
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionCall(SyntaxNode);

impl FunctionCall {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == FUNCTION_CALL {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the function name (e.g., "COUNT", "SUM", "ref")
    /// For namespaced calls like smelt.ref(), returns just "ref"
    pub fn name(&self) -> Option<String> {
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .collect();

        // Check for namespaced call: IDENT DOT IDENT
        if tokens.len() >= 3
            && tokens[0].kind() == IDENT
            && tokens[1].kind() == DOT
            && tokens[2].kind() == IDENT
        {
            return Some(tokens[2].text().to_string());
        }

        // Simple call: just IDENT
        tokens
            .iter()
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// Get the namespace prefix if this is a namespaced call (e.g., "smelt" from smelt.ref())
    pub fn namespace(&self) -> Option<String> {
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .collect();

        // Check for namespaced call: IDENT DOT IDENT
        if tokens.len() >= 3
            && tokens[0].kind() == IDENT
            && tokens[1].kind() == DOT
            && tokens[2].kind() == IDENT
        {
            Some(tokens[0].text().to_string())
        } else {
            None
        }
    }

    /// Get the text of the full function call
    pub fn text(&self) -> String {
        self.0.text().to_string()
    }

    /// Get all named parameters from this function call
    pub fn named_params(&self) -> impl Iterator<Item = NamedParam> + '_ {
        self.0.descendants().filter_map(NamedParam::cast)
    }

    /// Get the underlying syntax node
    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Get the arguments from this function call
    pub fn arguments(&self) -> Vec<Expr> {
        // Find ARG_LIST child and return EXPRESSION children
        self.0
            .children()
            .filter(|n| n.kind() == ARG_LIST)
            .flat_map(|arg_list| arg_list.children().filter_map(Expr::cast))
            .collect()
    }

    /// Get the FILTER clause if present (PostgreSQL aggregate filter)
    pub fn filter_clause(&self) -> Option<FilterClause> {
        self.0.children().find_map(FilterClause::cast)
    }
}

/// FILTER clause for aggregate functions (e.g., FILTER (WHERE status = 'active'))
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FilterClause(SyntaxNode);

impl FilterClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == FILTER_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the filter condition expression
    pub fn expression(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }
}

/// Array literal: ARRAY[1, 2, 3]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArrayLiteral(SyntaxNode);

impl ArrayLiteral {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == ARRAY_LITERAL {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get all element expressions in the array literal
    pub fn elements(&self) -> Vec<Expr> {
        self.0.children().filter_map(Expr::cast).collect()
    }
}

/// Array subscript: expr[index]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArraySubscript(SyntaxNode);

impl ArraySubscript {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == ARRAY_SUBSCRIPT {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn index(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }
}

/// Array slice: expr[start:end]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArraySlice(SyntaxNode);

impl ArraySlice {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == ARRAY_SLICE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn start(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    pub fn end(&self) -> Option<Expr> {
        // Get the second expression (after the colon)
        self.0.children().filter_map(Expr::cast).nth(1)
    }
}

/// ROW constructor: ROW(1, 2, 3)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RowConstructor(SyntaxNode);

impl RowConstructor {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == ROW_CONSTRUCTOR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get all element expressions in the ROW constructor
    pub fn elements(&self) -> Vec<Expr> {
        self.0.children().filter_map(Expr::cast).collect()
    }
}

/// Struct literal: STRUCT(1 AS a, 'hello' AS b)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StructLiteral(SyntaxNode);

impl StructLiteral {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == STRUCT_LITERAL {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get field expressions and their optional names.
    /// Returns (expression, optional_name) pairs.
    pub fn fields(&self) -> Vec<(Expr, Option<String>)> {
        let mut result = Vec::new();
        let mut current_expr: Option<Expr> = None;

        for child in self.0.children_with_tokens() {
            match child {
                rowan::NodeOrToken::Node(node) => {
                    if let Some(expr) = Expr::cast(node) {
                        // If we had a previous expression without a name, push it
                        if let Some(prev) = current_expr.take() {
                            result.push((prev, None));
                        }
                        current_expr = Some(expr);
                    }
                }
                rowan::NodeOrToken::Token(token) => {
                    if token.kind() == AS_KW {
                        // Next IDENT token is the field name
                        continue;
                    }
                    if token.kind() == IDENT {
                        if let Some(expr) = current_expr.take() {
                            result.push((expr, Some(token.text().to_string())));
                        }
                    }
                    if token.kind() == COMMA {
                        // Flush any pending unnamed expression
                        if let Some(expr) = current_expr.take() {
                            result.push((expr, None));
                        }
                    }
                }
            }
        }
        // Flush last expression
        if let Some(expr) = current_expr.take() {
            result.push((expr, None));
        }
        result
    }
}

/// PIVOT clause
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PivotClause(SyntaxNode);

impl PivotClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == PIVOT_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

/// UNPIVOT clause
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnpivotClause(SyntaxNode);

impl UnpivotClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == UNPIVOT_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

/// Lambda expression (e.g., x -> x + 1 or (acc, x) -> acc + x)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LambdaExpr(SyntaxNode);

impl LambdaExpr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == LAMBDA_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the parameter names
    pub fn params(&self) -> Vec<String> {
        self.0
            .children()
            .find(|n| n.kind() == LAMBDA_PARAM_LIST)
            .map(|param_list| {
                param_list
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .filter(|t| t.kind() == IDENT)
                    .map(|t| t.text().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get the body expression
    pub fn body(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }
}

/// Named parameter in a function call (e.g., filter => expr)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NamedParam(SyntaxNode);

impl NamedParam {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == NAMED_PARAM {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the parameter name (the identifier before =>)
    pub fn name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// Get the parameter value as text (everything after =>)
    pub fn value_text(&self) -> String {
        // Get the full text and extract everything after the =>
        let full_text = self.0.text().to_string();

        // Find the => and return everything after it, trimmed
        if let Some(arrow_pos) = full_text.find("=>") {
            full_text[arrow_pos + 2..].trim().to_string()
        } else {
            String::new()
        }
    }

    /// The value expression (the sub-expression after `=>`), if one can be
    /// extracted as an `Expr`. Phase 6 uses this to type-check the value
    /// against the declared parameter's type at a call site.
    pub fn value_expr(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    /// Text range of the parameter-name identifier (before `=>`), suitable
    /// for anchoring `MissingArgument` / duplicate-name diagnostics at the
    /// call site.
    pub fn name_range(&self) -> Option<TextRange> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text_range())
    }

    /// Full text range of this NAMED_PARAM node (name + `=>` + value).
    pub fn text_range(&self) -> TextRange {
        self.0.text_range()
    }
}

/// ref('model_name') function call wrapper
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RefCall(FunctionCall);

impl RefCall {
    /// Create a RefCall from a FunctionCall if it's a smelt.ref() call
    pub fn from_function_call(func: FunctionCall) -> Option<Self> {
        let name = func.name()?.to_lowercase();
        let namespace = func.namespace()?; // Require namespace

        // Only accept smelt.ref() - namespace is required
        if name == "ref" && namespace.to_lowercase() == "smelt" {
            Some(Self(func))
        } else {
            None
        }
    }

    /// Get the underlying FunctionCall
    pub fn function_call(&self) -> &FunctionCall {
        &self.0
    }

    /// Get the model name from the ref call (first argument)
    pub fn model_name(&self) -> Option<String> {
        // Look for the first STRING token in the function call arguments
        self.0
             .0
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == STRING)
            .map(|t| {
                let text = t.text();
                // Strip quotes
                text.trim_start_matches('\'')
                    .trim_start_matches('"')
                    .trim_end_matches('\'')
                    .trim_end_matches('"')
                    .to_string()
            })
    }

    /// Get the text range of the entire ref call
    pub fn range(&self) -> TextRange {
        self.0 .0.text_range()
    }

    /// Get the text range of just the model name string (inside quotes)
    pub fn name_range(&self) -> Option<TextRange> {
        self.0
             .0
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == STRING)
            .map(|t| t.text_range())
    }

    /// Get the text range of the string content inside quotes (excluding the quote characters)
    pub fn content_range(&self) -> Option<TextRange> {
        self.0
             .0
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == STRING)
            .map(|t| {
                let range = t.text_range();
                let text = t.text();
                // STRING tokens include quotes — trim them
                if text.len() >= 2 && (text.starts_with('\'') || text.starts_with('"')) {
                    TextRange::new(
                        range.start() + rowan::TextSize::from(1),
                        range.end() - rowan::TextSize::from(1),
                    )
                } else {
                    range
                }
            })
    }

    /// Get all named parameters from this ref call
    pub fn named_params(&self) -> impl Iterator<Item = NamedParam> + '_ {
        self.0.named_params()
    }
}

/// source('source.table') function call wrapper
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceCall(FunctionCall);

impl SourceCall {
    /// Create a SourceCall from a FunctionCall if it's a smelt.source() call
    pub fn from_function_call(func: FunctionCall) -> Option<Self> {
        let name = func.name()?.to_lowercase();
        let namespace = func.namespace()?;

        if name == "source" && namespace.to_lowercase() == "smelt" {
            Some(Self(func))
        } else {
            None
        }
    }

    /// Get the qualified name from the source call (e.g., "raw.users")
    pub fn qualified_name(&self) -> Option<String> {
        self.0
             .0
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == STRING)
            .map(|t| {
                let text = t.text();
                text.trim_start_matches('\'')
                    .trim_start_matches('"')
                    .trim_end_matches('\'')
                    .trim_end_matches('"')
                    .to_string()
            })
    }

    /// Get the source name (first part before the dot)
    pub fn source_name(&self) -> Option<String> {
        let qualified = self.qualified_name()?;
        qualified.split('.').next().map(|s| s.to_string())
    }

    /// Get the table name (second part after the dot)
    pub fn table_name(&self) -> Option<String> {
        let qualified = self.qualified_name()?;
        qualified.split('.').nth(1).map(|s| s.to_string())
    }

    /// Get the text range of the entire source call
    pub fn range(&self) -> TextRange {
        self.0 .0.text_range()
    }

    /// Get the text range of just the qualified name string (including quotes)
    pub fn name_range(&self) -> Option<TextRange> {
        self.0
             .0
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == STRING)
            .map(|t| t.text_range())
    }

    /// Get the text range of just the table name portion inside the quoted string
    /// For `source('raw.users')`, returns the range covering `users`
    pub fn table_name_range(&self) -> Option<TextRange> {
        self.0
             .0
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == STRING)
            .and_then(|t| {
                let range = t.text_range();
                let text = t.text();
                // Strip outer quotes to get the qualified name
                let inner = text
                    .trim_start_matches('\'')
                    .trim_start_matches('"')
                    .trim_end_matches('\'')
                    .trim_end_matches('"');
                // Find the dot position in the inner text
                if let Some(dot_pos) = inner.find('.') {
                    // Table name starts after the dot, plus 1 for the opening quote
                    let quote_offset = if text.starts_with('\'') || text.starts_with('"') {
                        1u32
                    } else {
                        0
                    };
                    let table_start =
                        range.start() + rowan::TextSize::from(quote_offset + dot_pos as u32 + 1);
                    let table_end = range.end() - rowan::TextSize::from(quote_offset);
                    Some(TextRange::new(table_start, table_end))
                } else {
                    None
                }
            })
    }
}

/// Helper to convert TextRange offset to line/column position
pub fn offset_to_position(text: &str, offset: usize) -> Position {
    let mut line = 0u32;
    let mut column = 0u32;

    for (i, ch) in text.chars().enumerate() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 0;
        } else {
            column += 1;
        }
    }

    Position { line, column }
}

/// Helper to convert TextRange to LSP Range
pub fn text_range_to_range(text: &str, range: TextRange) -> Range {
    let start = offset_to_position(text, usize::from(range.start()));
    let end = offset_to_position(text, usize::from(range.end()));
    Range { start, end }
}

/// Position (line, column)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

/// Range (start, end positions)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

// ===== Phase 10: Expression Enhancement AST Wrappers =====

/// CASE expression (CASE WHEN ... THEN ... ELSE ... END)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CaseExpr(SyntaxNode);

impl CaseExpr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == CASE_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the underlying syntax node
    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Get the case value expression (for simple CASE)
    /// Returns None for searched CASE (CASE WHEN ...)
    pub fn case_value(&self) -> Option<Expr> {
        // The case value is the first EXPRESSION-like child, before any WHEN_CLAUSE
        self.0
            .children()
            .take_while(|n| n.kind() != WHEN_CLAUSE)
            .find_map(Expr::cast)
    }

    /// Get all WHEN clauses
    pub fn when_clauses(&self) -> impl Iterator<Item = WhenClause> + '_ {
        self.0.children().filter_map(WhenClause::cast)
    }

    /// Get the ELSE expression if present
    pub fn else_expr(&self) -> Option<Expr> {
        // The ELSE expression is the last EXPRESSION child, after all WHEN clauses
        let mut found_else = false;
        for child in self.0.children_with_tokens() {
            if let Some(token) = child.as_token() {
                if token.kind() == ELSE_KW {
                    found_else = true;
                }
            } else if found_else {
                if let Some(node) = child.as_node() {
                    if let Some(expr) = Expr::cast(node.clone()) {
                        return Some(expr);
                    }
                }
            }
        }
        None
    }
}

/// WHEN clause in a CASE expression
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WhenClause(SyntaxNode);

impl WhenClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == WHEN_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the condition expression (after WHEN)
    pub fn condition(&self) -> Option<Expr> {
        // First EXPRESSION child
        self.0.children().find_map(Expr::cast)
    }

    /// Get the result expression (after THEN)
    pub fn result(&self) -> Option<Expr> {
        // Second EXPRESSION child
        self.0.children().filter_map(Expr::cast).nth(1)
    }
}

/// CAST expression (CAST(expr AS type) or expr::type)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CastExpr(SyntaxNode);

impl CastExpr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == CAST_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the expression being cast
    pub fn expression(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    /// Get the type specification
    pub fn type_spec(&self) -> Option<TypeSpec> {
        self.0.children().find_map(TypeSpec::cast)
    }

    /// Check if this is a PostgreSQL :: cast (vs CAST(...))
    pub fn is_double_colon_cast(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == DOUBLE_COLON)
    }
}

/// EXTRACT expression (EXTRACT(field FROM expr))
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExtractExpr(SyntaxNode);

impl ExtractExpr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == EXTRACT_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the field name (EPOCH, YEAR, MONTH, DAY, HOUR, MINUTE, SECOND)
    pub fn field_name(&self) -> Option<String> {
        // The field is the first IDENT or keyword token after EXTRACT_KW and LPAREN
        let mut after_lparen = false;
        for elem in self.0.children_with_tokens() {
            if let Some(token) = elem.as_token() {
                match token.kind() {
                    LPAREN => after_lparen = true,
                    IDENT if after_lparen => return Some(token.text().to_uppercase()),
                    k if k.is_keyword() && after_lparen && k != FROM_KW => {
                        return Some(token.text().to_uppercase())
                    }
                    _ => {}
                }
            }
        }
        None
    }

    /// Get the source expression (the expression after FROM)
    pub fn expression(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

/// Type specification (e.g., INTEGER, VARCHAR(255), DECIMAL(10,2))
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeSpec(SyntaxNode);

impl TypeSpec {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == TYPE_SPEC {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the type name (e.g., "INTEGER", "VARCHAR")
    pub fn type_name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// Get the full text including parameters (e.g., "VARCHAR(255)")
    pub fn full_text(&self) -> String {
        self.0.text().to_string()
    }
}

/// Subquery (SELECT statement in parentheses)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Subquery(SyntaxNode);

impl Subquery {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SUBQUERY {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the SELECT statement
    pub fn select_stmt(&self) -> Option<SelectStmt> {
        self.0.children().find_map(SelectStmt::cast)
    }
}

/// BETWEEN expression (expr BETWEEN low AND high)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BetweenExpr(SyntaxNode);

impl BetweenExpr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == BETWEEN_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the lower bound expression
    pub fn lower_bound(&self) -> Option<Expr> {
        // First EXPRESSION child
        self.0.children().find_map(Expr::cast)
    }

    /// Get the upper bound expression
    pub fn upper_bound(&self) -> Option<Expr> {
        // Second EXPRESSION child
        self.0.children().filter_map(Expr::cast).nth(1)
    }
}

/// IN expression (expr IN (values...) or expr IN (subquery))
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InExpr(SyntaxNode);

impl InExpr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == IN_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Check if this is a subquery IN (vs value list)
    pub fn is_subquery(&self) -> bool {
        self.0.children().any(|n| n.kind() == SUBQUERY)
    }

    /// Get the subquery (if this is IN (subquery))
    pub fn subquery(&self) -> Option<Subquery> {
        self.0.children().find_map(Subquery::cast)
    }

    /// Get the value expressions (if this is IN (value1, value2, ...))
    pub fn values(&self) -> Vec<Expr> {
        if self.is_subquery() {
            Vec::new()
        } else {
            self.0.children().filter_map(Expr::cast).collect()
        }
    }
}

/// EXISTS expression (EXISTS (subquery))
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExistsExpr(SyntaxNode);

impl ExistsExpr {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == EXISTS_EXPR {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the subquery
    pub fn subquery(&self) -> Option<Subquery> {
        self.0.children().find_map(Subquery::cast)
    }
}

// ===== Phase 11: SQL Clause AST Wrappers =====

/// GROUP BY clause
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GroupByClause(SyntaxNode);

impl GroupByClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == GROUP_BY_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    /// Get the expressions in the GROUP BY clause
    pub fn expressions(&self) -> impl Iterator<Item = Expr> + '_ {
        self.0.children().filter_map(Expr::cast)
    }
}

/// HAVING clause
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HavingClause(SyntaxNode);

impl HavingClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == HAVING_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn expression(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }
}

/// QUALIFY clause (window function filtering)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QualifyClause(SyntaxNode);

impl QualifyClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == QUALIFY_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn expression(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }
}

/// ORDER BY clause
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OrderByClause(SyntaxNode);

impl OrderByClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == ORDER_BY_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn items(&self) -> impl Iterator<Item = OrderByItem> + '_ {
        self.0.children().filter_map(OrderByItem::cast)
    }
}

/// ORDER BY item
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OrderByItem(SyntaxNode);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NullOrdering {
    First,
    Last,
}

impl OrderByItem {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == ORDER_BY_ITEM {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn expression(&self) -> Option<Expr> {
        self.0.children().find_map(Expr::cast)
    }

    pub fn direction(&self) -> Option<SortDirection> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find_map(|t| match t.kind() {
                ASC_KW => Some(SortDirection::Asc),
                DESC_KW => Some(SortDirection::Desc),
                _ => None,
            })
    }

    pub fn null_ordering(&self) -> Option<NullOrdering> {
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .collect();

        for (i, token) in tokens.iter().enumerate() {
            if token.kind() == NULLS_KW {
                // Skip whitespace to find FIRST or LAST
                for next_token in &tokens[i + 1..] {
                    match next_token.kind() {
                        WHITESPACE | COMMENT => continue,
                        FIRST_KW => return Some(NullOrdering::First),
                        LAST_KW => return Some(NullOrdering::Last),
                        _ => break,
                    }
                }
            }
        }
        None
    }
}

/// LIMIT clause
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LimitClause(SyntaxNode);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitValue {
    Number(String),
    All,
}

impl LimitClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == LIMIT_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn limit_value(&self) -> Option<LimitValue> {
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| !matches!(t.kind(), WHITESPACE | COMMENT))
            .collect();

        for i in 0..tokens.len() {
            if tokens[i].kind() == LIMIT_KW && i + 1 < tokens.len() {
                return match tokens[i + 1].kind() {
                    NUMBER => Some(LimitValue::Number(tokens[i + 1].text().to_string())),
                    ALL_KW => Some(LimitValue::All),
                    _ => None,
                };
            }
        }
        None
    }

    pub fn offset_value(&self) -> Option<String> {
        let tokens: Vec<_> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| !matches!(t.kind(), WHITESPACE | COMMENT))
            .collect();

        for i in 0..tokens.len() {
            if tokens[i].kind() == OFFSET_KW
                && i + 1 < tokens.len()
                && tokens[i + 1].kind() == NUMBER
            {
                return Some(tokens[i + 1].text().to_string());
            }
        }
        None
    }

    /// Get the underlying syntax node (for printer)
    #[allow(dead_code)] // Used by printer module
    pub(crate) fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

// ===== Phase 12: Window Function AST Wrappers =====

/// Window specification (OVER clause)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WindowSpec(SyntaxNode);

impl WindowSpec {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == WINDOW_SPEC {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn partition_by(&self) -> Option<PartitionByClause> {
        self.0.children().find_map(PartitionByClause::cast)
    }

    pub fn order_by(&self) -> Option<OrderByClause> {
        self.0.children().find_map(OrderByClause::cast)
    }

    pub fn frame(&self) -> Option<WindowFrame> {
        self.0.children().find_map(WindowFrame::cast)
    }

    /// Get named window reference if this is OVER window_name
    pub fn window_name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }
}

/// PARTITION BY clause
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PartitionByClause(SyntaxNode);

impl PartitionByClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == PARTITION_BY_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn expressions(&self) -> impl Iterator<Item = Expr> + '_ {
        self.0.children().filter_map(Expr::cast)
    }
}

/// Window frame specification
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WindowFrame(SyntaxNode);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameUnit {
    Rows,
    Range,
    Groups,
}

impl WindowFrame {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == WINDOW_FRAME {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn unit(&self) -> Option<FrameUnit> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find_map(|t| match t.kind() {
                ROWS_KW => Some(FrameUnit::Rows),
                RANGE_KW => Some(FrameUnit::Range),
                GROUPS_KW => Some(FrameUnit::Groups),
                _ => None,
            })
    }

    pub fn bounds(&self) -> Vec<FrameBound> {
        self.0.children().filter_map(FrameBound::cast).collect()
    }
}

/// Frame bound
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FrameBound(SyntaxNode);

impl FrameBound {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == FRAME_BOUND {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn text(&self) -> String {
        self.0.text().to_string()
    }
}

// ===== Phase 13: Common Table Expressions (CTEs) =====

/// WITH clause (CTEs)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WithClause(SyntaxNode);

impl WithClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == WITH_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Check if this is a RECURSIVE CTE
    pub fn is_recursive(&self) -> bool {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .any(|t| t.kind() == RECURSIVE_KW)
    }

    /// Get all CTEs in this WITH clause
    pub fn ctes(&self) -> impl Iterator<Item = Cte> + '_ {
        self.0.children().filter_map(Cte::cast)
    }
}

/// Common Table Expression (single CTE in a WITH clause)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Cte(SyntaxNode);

impl Cte {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == CTE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Get the CTE name
    pub fn name(&self) -> Option<String> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// Get the text range of just the CTE name identifier
    pub fn name_range(&self) -> Option<TextRange> {
        self.0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text_range())
    }

    /// Get the query (SELECT statement)
    pub fn query(&self) -> Option<Subquery> {
        self.0.children().find_map(Subquery::cast)
    }

    /// Get the column names from the optional column list
    pub fn column_names(&self) -> Vec<String> {
        // Extract column names between first LPAREN and RPAREN (before AS)
        let mut in_column_list = false;
        let mut found_as = false;
        let mut columns = Vec::new();

        for child in self.0.children_with_tokens() {
            if let Some(token) = child.as_token() {
                match token.kind() {
                    LPAREN if !found_as => in_column_list = true,
                    RPAREN if in_column_list && !found_as => in_column_list = false,
                    AS_KW => {
                        found_as = true;
                        in_column_list = false;
                    }
                    IDENT if in_column_list => {
                        columns.push(token.text().to_string());
                    }
                    _ => {}
                }
            }
        }

        columns
    }
}

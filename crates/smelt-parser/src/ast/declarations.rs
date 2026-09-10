use super::*;
use crate::syntax_kind::SyntaxNode;
use rowan::TextRange;

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

    /// The declared function name (text of the name token inside DEFINE_NAME).
    ///
    /// Accepts both `IDENT` tokens (normal names) and the ternary keyword tokens
    /// (`IF_KW`, `THEN_KW`, `ELSE_KW`) — the latter are wrapped in `DEFINE_NAME` by
    /// the parser for error-recovery purposes so that `check_define_name_shadowing`
    /// can emit `TernaryKeywordShadowed` rather than a cryptic parse error.
    pub fn name(&self) -> Option<String> {
        let name_node = self.0.children().find(|n| n.kind() == DEFINE_NAME)?;
        name_node
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| matches!(t.kind(), IDENT | IF_KW | THEN_KW | ELSE_KW))
            .map(|t| t.text().to_string())
    }

    /// The text range of the DEFINE_NAME node (the function name identifier).
    pub fn name_range(&self) -> Option<TextRange> {
        let name_node = self.0.children().find(|n| n.kind() == DEFINE_NAME)?;
        let ident = name_node
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| matches!(t.kind(), IDENT | IF_KW | THEN_KW | ELSE_KW))?;
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

// ===== smelt.test (Phase 3: parser-only declaration) =====

/// Top-level `smelt.test <name> AS (<select>) [PASSING <dep> AS (<rows>)]... EXPECT (<rows>)`
/// declaration.
///
/// A test is a peer of `smelt.define`/`smelt.extern` on the kind axis. The
/// body `<select>` is an assertion query; the `PASSING` clauses supply inline
/// table data; the `EXPECT` clause lists expected result rows. Semantic wiring
/// (Phase 5) classifies the kind and wires the runner.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SmeltTest(SyntaxNode);

impl SmeltTest {
    /// Cast from a raw `SyntaxNode`. Returns `Some` only for `SMELT_TEST` nodes.
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SMELT_TEST {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The test name — text of the IDENT token inside the `TEST_NAME` child.
    pub fn name(&self) -> Option<String> {
        self.0
            .children()
            .find(|n| n.kind() == TEST_NAME)?
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// The text range of the `TEST_NAME` child — the name identifier span.
    pub fn name_range(&self) -> Option<TextRange> {
        let name_node = self.0.children().find(|n| n.kind() == TEST_NAME)?;
        let ident = name_node
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)?;
        Some(ident.text_range())
    }

    /// The `SELECT` (or `WITH ... SELECT`) statement body of the test assertion.
    pub fn body_select(&self) -> Option<SelectStmt> {
        self.0.children().find_map(SelectStmt::cast)
    }

    /// Iterate over `PASSING` clauses in source order.
    pub fn passing_clauses(&self) -> impl Iterator<Item = PassingClause> + '_ {
        self.0.children().filter_map(PassingClause::cast)
    }

    /// The required `EXPECT` clause (the expected result rows).
    pub fn expect_clause(&self) -> Option<ExpectClause> {
        self.0.children().find_map(ExpectClause::cast)
    }

    /// Byte offset at which this declaration starts in the source text.
    /// See `SmeltDefine::source_offset` for the offset-stability rationale.
    pub fn source_offset(&self) -> usize {
        usize::from(self.0.text_range().start())
    }
}

// ===== smelt.check (Phase 1, data-checks) =====

/// Top-level `smelt.check <name> AS ( <select> )` declaration.
///
/// A check is a data-quality assertion query: rows returned by `<select>` are
/// considered failures. A check has **no** `PASSING` or `EXPECT` clauses —
/// those are `smelt.test`-only surface. Any stray `PASSING`/`EXPECT` clause
/// captured on the node is diagnosable in Phase 2 as `CheckHasTestClause`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SmeltCheck(SyntaxNode);

impl SmeltCheck {
    /// Cast from a raw `SyntaxNode`. Returns `Some` only for `SMELT_CHECK` nodes.
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SMELT_CHECK {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The check name — text of the IDENT token inside the `CHECK_NAME` child.
    pub fn name(&self) -> Option<String> {
        self.0
            .children()
            .find(|n| n.kind() == CHECK_NAME)?
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// The text range of the `CHECK_NAME` child — the name identifier span.
    pub fn name_range(&self) -> Option<TextRange> {
        let name_node = self.0.children().find(|n| n.kind() == CHECK_NAME)?;
        let ident = name_node
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)?;
        Some(ident.text_range())
    }

    /// The `SELECT` (or `WITH ... SELECT`) statement body of the check assertion.
    pub fn body_select(&self) -> Option<SelectStmt> {
        self.0.children().find_map(SelectStmt::cast)
    }

    /// Iterate over any stray `PASSING` clauses captured on the node.
    ///
    /// For well-formed checks this iterator is empty. Phase 2 uses this to
    /// emit `CheckHasTestClause` diagnostics when it is non-empty.
    pub fn passing_clauses(&self) -> impl Iterator<Item = PassingClause> + '_ {
        self.0.children().filter_map(PassingClause::cast)
    }

    /// The stray `EXPECT` clause captured on the node, if present.
    ///
    /// For well-formed checks this returns `None`. Phase 2 uses this to
    /// emit `CheckHasTestClause` diagnostics when it is `Some`.
    pub fn expect_clause(&self) -> Option<ExpectClause> {
        self.0.children().find_map(ExpectClause::cast)
    }

    /// Byte offset at which this declaration starts in the source text.
    pub fn source_offset(&self) -> usize {
        usize::from(self.0.text_range().start())
    }
}

/// The `EXPECT ( <rows> )` clause inside a `smelt.test` declaration.
///
/// `<rows>` is a comma-separated list of record literals `{key: value, ...}`.
/// Omitted keys are allowed (partial match shape for property tests).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExpectClause(SyntaxNode);

impl ExpectClause {
    /// Cast from a raw `SyntaxNode`. Returns `Some` only for `EXPECT_CLAUSE` nodes.
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == EXPECT_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// Iterate over the expected result rows (record literals) in source order.
    ///
    /// Each row is a `RECORD_LITERAL` node produced by parsing a `{k: v, ...}`
    /// expression. Because `parse_expression()` wraps each row in an `EXPRESSION`
    /// node, we use `descendants()` to surface the inner `RECORD_LITERAL` nodes.
    pub fn rows(&self) -> impl Iterator<Item = RecordLiteral> + '_ {
        self.0.descendants().filter_map(RecordLiteral::cast)
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

    /// Returns `true` when this type reference carries a `NOT NULL`
    /// qualifier (Phase 5, nullability-soundness). A `NOT_NULL_QUALIFIER`
    /// child node is emitted by the parser for `Expr<T NOT NULL>`,
    /// `AggExpr<T NOT NULL>`, `WindowExpr<T NOT NULL>`, and
    /// `TableExpr<{…} NOT NULL>` annotations.
    pub fn not_null(&self) -> bool {
        self.0.children().any(|n| n.kind() == NOT_NULL_QUALIFIER)
    }
}

/// First IDENT token in the sub-tree's document order, if any.
fn leading_ident_text(node: &SyntaxNode) -> Option<String> {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == IDENT)
        .map(|t| t.text().to_string())
}

/// Strip a leading/trailing matching quote (`"` or `'`) from raw token text,
/// used to turn a double-quoted identifier lexed as `STRING` (e.g.
/// `"median_delay"`) back into its bare name. Text with no matching quote
/// pair is returned unchanged.
pub(crate) fn strip_ident_quotes(raw: &str) -> &str {
    raw.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(raw)
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

    /// Returns `true` when this row field carries a `NOT NULL` qualifier
    /// (Phase 5, nullability-soundness). The `NOT_NULL_QUALIFIER` node is
    /// emitted by the parser as a direct child of `ROW_FIELD` for
    /// `TableExpr<{id: Integer NOT NULL}>` fields.
    pub fn not_null(&self) -> bool {
        self.0.children().any(|n| n.kind() == NOT_NULL_QUALIFIER)
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
/// A `smelt.as_struct(alias [EXCEPT col1, col2, ...])` expression (Phase 38).
///
/// The alias is the table/parameter qualifier whose columns are collected
/// into a struct. The optional `EXCEPT` list excludes specific column names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SmeltAsStructCall(SyntaxNode);

impl SmeltAsStructCall {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SMELT_AS_STRUCT_CALL {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The alias identifier — the first `IDENT` token after the opening `(`.
    pub fn alias(&self) -> Option<String> {
        let mut after_paren = false;
        for tok in self.0.children_with_tokens().filter_map(|e| e.into_token()) {
            if tok.kind() == LPAREN {
                after_paren = true;
            } else if after_paren && tok.kind() == IDENT {
                return Some(tok.text().to_string());
            }
        }
        None
    }

    /// Column names that appear after `EXCEPT`, if any.
    pub fn except_columns(&self) -> Vec<String> {
        self.0
            .children()
            .find(|n| n.kind() == EXCEPT_COL_LIST)
            .map(|except| {
                except
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .filter(|t| t.kind() == IDENT)
                    .map(|t| t.text().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Text range of the whole `SMELT_AS_STRUCT_CALL` node.
    pub fn text_range(&self) -> rowan::TextRange {
        self.0.text_range()
    }
}

// ===== Unified `smelt.<path>` value-ref and call form (Phase 1 of the
// smelt.<path> migration). Coexists with the legacy `SMELT_FN_CALL` /
// `FUNCTION_CALL` paths until Phase 4. =====

/// `smelt.<path>` value-form reference (no trailing `(args)`).
///
/// Produced by the parser for any `smelt.` prefix followed by a dotted path
/// when no call-list follows. Used in FROM/argument position to address any
/// project-defined entity (model, seed, source, function, test) by its
/// workspace-relative path.
///
/// Phase 1 produces this node uniformly; kind dispatch (model vs. function
/// vs. seed vs. source vs. test) is the data plane's job in Phase 2a.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SmeltPathRef(SyntaxNode);

impl SmeltPathRef {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SMELT_PATH_REF {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The `SMELT_PATH` child holding the dotted path tokens (including the
    /// leading `smelt` IDENT). Returns `None` only on error-recovery paths.
    pub fn path(&self) -> Option<SmeltPath> {
        self.0.children().find_map(SmeltPath::cast)
    }

    /// Path segments AFTER the leading `smelt` token. For `smelt.models.users`
    /// this returns `["models", "users"]`. Empty if the path is malformed.
    pub fn segments(&self) -> Vec<String> {
        self.path().map(|p| p.segments()).unwrap_or_default()
    }

    /// Text range of the entire `SMELT_PATH_REF` node.
    pub fn text_range(&self) -> TextRange {
        self.0.text_range()
    }

    /// The optional `CTE_SEGMENT` child node (present when `#<cte>` suffix was
    /// parsed). Returns `None` when this path ref has no CTE suffix.
    fn cte_segment_node(&self) -> Option<SyntaxNode> {
        self.0.children().find(|n| n.kind() == CTE_SEGMENT)
    }

    /// The CTE name from a trailing `#<cte>` suffix, or `None` if not present.
    ///
    /// For `smelt.daily_revenue#daily_agg` this returns `Some("daily_agg")`.
    pub fn cte_name(&self) -> Option<String> {
        let seg = self.cte_segment_node()?;
        seg.children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
    }

    /// The text range of the `#` token in a trailing `#<cte>` suffix, used for
    /// anchoring `CteRefOutsideTest` diagnostics at the operator itself.
    ///
    /// Returns `None` when this path ref has no CTE suffix.
    pub fn hash_range(&self) -> Option<TextRange> {
        let seg = self.cte_segment_node()?;
        seg.children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == HASH)
            .map(|t| t.text_range())
    }
}

/// `smelt.<path>(<args>)` call form, with optional trailing `PASSING` clauses.
///
/// Produced by the parser for any `smelt.` prefix followed by a dotted path
/// terminated by `(`. Used to call parameterised entities (functions,
/// parameterised models). Coexists with the legacy `SMELT_FN_CALL` until
/// Phase 4.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SmeltPathCall(SyntaxNode);

impl SmeltPathCall {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SMELT_PATH_CALL {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The `SMELT_PATH` child holding the dotted path tokens (including the
    /// leading `smelt` IDENT). Returns `None` only on error-recovery paths.
    pub fn path(&self) -> Option<SmeltPath> {
        self.0.children().find_map(SmeltPath::cast)
    }

    /// Path segments AFTER the leading `smelt` token.
    pub fn segments(&self) -> Vec<String> {
        self.path().map(|p| p.segments()).unwrap_or_default()
    }

    /// The argument list (`(args)`), if present.
    pub fn arg_list(&self) -> Option<ArgList> {
        self.0.children().find_map(ArgList::cast)
    }

    /// Iterate over the trailing `PASSING_CLAUSE` children, in source order.
    pub fn passing_clauses(&self) -> impl Iterator<Item = PassingClause> + '_ {
        self.0.children().filter_map(PassingClause::cast)
    }

    /// Text range of the `SMELT_PATH` child (path only, excluding the args parens).
    /// Returns `None` on error-recovery paths where the path node is absent.
    pub fn call_path_range(&self) -> Option<TextRange> {
        self.path().map(|p| p.syntax().text_range())
    }

    /// Text range of the entire `SMELT_PATH_CALL` node.
    pub fn text_range(&self) -> TextRange {
        self.0.text_range()
    }
}

/// The dotted path inside a `SMELT_PATH_REF` or `SMELT_PATH_CALL`. Includes
/// the leading `smelt` IDENT token as well as all subsequent dotted segments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SmeltPath(SyntaxNode);

impl SmeltPath {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == SMELT_PATH {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The path segments AFTER the leading `smelt` IDENT — e.g. for
    /// `smelt.models.users` returns `["models", "users"]`.
    pub fn segments(&self) -> Vec<String> {
        let idents: Vec<String> = self
            .0
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            // Include IDENT and also ALL_KW: `all` is a reserved SQL keyword
            // but a valid smelt path segment in `smelt.models.all`.
            .filter(|t| t.kind() == IDENT || t.kind() == ALL_KW)
            .map(|t| t.text().to_string())
            .collect();
        // Drop the leading `smelt` token. Error-recovery paths may produce a
        // shorter token list — return whatever remains.
        idents.into_iter().skip(1).collect()
    }
}

/// A single `PASSING <name> AS (<body>)` clause attached to a `SMELT_PATH_CALL`.
/// Phase 28 introduces these as children of `SMELT_FN_CALL` nodes when the
/// call is followed by one or more contextual `PASSING` keywords.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PassingClause(SyntaxNode);

impl PassingClause {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        if node.kind() == PASSING_CLAUSE {
            Some(Self(node))
        } else {
            None
        }
    }

    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    /// The binding name — the dependency's bare address path inside the
    /// `PASSING_NAME` child. May be multi-segment (e.g. `silver.sessions`); the
    /// `IDENT` segments are joined with `.` (DOT tokens are dropped).
    pub fn name(&self) -> Option<String> {
        let name_node = self.0.children().find(|n| n.kind() == PASSING_NAME)?;
        let segments: Vec<String> = name_node
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == IDENT)
            .map(|t| t.text().to_string())
            .collect();
        if segments.is_empty() {
            None
        } else {
            Some(segments.join("."))
        }
    }

    /// Raw text of the expression inside `(...)` in the `PASSING_BODY` child,
    /// with leading/trailing whitespace trimmed. Returns `None` if the body
    /// node is absent (error-recovery case).
    pub fn body_text(&self) -> Option<String> {
        let body_node = self.0.children().find(|n| n.kind() == PASSING_BODY)?;
        Some(trim_source_text(&body_node))
    }

    /// The expression AST node inside the `PASSING_BODY`. Returns `None` when
    /// the body node is absent or contains no parseable expression (error-recovery
    /// path). Use this for type-checking the body at the call site (Phase 29).
    pub fn body_expr(&self) -> Option<Expr> {
        let body_node = self.0.children().find(|n| n.kind() == PASSING_BODY)?;
        body_node.children().find_map(Expr::cast)
    }

    /// The text range of the `PASSING_NAME` child — used to anchor
    /// `UnknownPassingParameter` diagnostics at the name token (Phase 29).
    pub fn name_range(&self) -> Option<TextRange> {
        self.0
            .children()
            .find(|n| n.kind() == PASSING_NAME)
            .map(|n| n.text_range())
    }

    /// Iterate over the body rows (record literals) in source order.
    ///
    /// For `smelt.test` PASSING clauses the body is a comma-separated list of
    /// record literals `{k: v, ...}`.  For function-call PASSING clauses the
    /// body is typically a single expression (SELECT / aggregate), so this
    /// iterator returns exactly one result in that case.
    pub fn rows(&self) -> impl Iterator<Item = RecordLiteral> + '_ {
        self.0.descendants().filter_map(RecordLiteral::cast)
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

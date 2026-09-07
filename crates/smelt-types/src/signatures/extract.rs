use super::*;
use crate::DataType;
use smelt_parser::ast::{File as AstFile, Param as AstParam, SmeltDefine, SmeltExtern, TypeRef};
use smelt_parser::TextRange;

/// The set of backends a function is declared (or inferred) to target.
///
/// `All` is the universal set — it accepts any backend. `Only(names)` is
/// a fixed, alphabetically-sortable list of backend names (e.g.
/// `["duckdb"]`, `["duckdb", "spark"]`). Backend names are case-sensitive
/// to keep string comparison simple; the canonical names are lowercase
/// (`duckdb`, `spark`, `databricks`).
///
/// Introduced in Phase 11 of the smelt-functions plan (§16 #23).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendSet {
    /// Unconstrained — valid on every backend.
    All,
    /// Restricted to the listed backends. The list is sorted & dedup'd
    /// when constructed via [`BackendSet::from_names`].
    Only(Vec<String>),
}

impl BackendSet {
    /// Canonicalise a raw list of backend names into a [`BackendSet::Only`].
    /// Normalises (trim + lowercase), deduplicates, and sorts so that
    /// two equivalent lists compare equal.
    pub fn from_names<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut v: Vec<String> = names
            .into_iter()
            .map(|s| s.as_ref().trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        v.sort();
        v.dedup();
        BackendSet::Only(v)
    }

    /// Is `self` a (non-strict) subset of `other`? `All` is only a
    /// subset of `All`; `Only(a)` is a subset of `All`; `Only(a)` is a
    /// subset of `Only(b)` iff every element of `a` is in `b`.
    pub fn is_subset_of(&self, other: &BackendSet) -> bool {
        match (self, other) {
            (BackendSet::All, BackendSet::All) => true,
            (BackendSet::All, BackendSet::Only(_)) => false,
            (BackendSet::Only(_), BackendSet::All) => true,
            (BackendSet::Only(a), BackendSet::Only(b)) => a.iter().all(|x| b.contains(x)),
        }
    }

    /// Intersection: backends present in both sets.
    pub fn intersect(&self, other: &BackendSet) -> BackendSet {
        match (self, other) {
            (BackendSet::All, other) => other.clone(),
            (self_, BackendSet::All) => self_.clone(),
            (BackendSet::Only(a), BackendSet::Only(b)) => {
                let mut v: Vec<String> = a.iter().filter(|x| b.contains(*x)).cloned().collect();
                v.sort();
                v.dedup();
                BackendSet::Only(v)
            }
        }
    }

    /// Human-readable rendering for diagnostics, e.g. `all`,
    /// `[duckdb]`, `[duckdb, spark]`.
    pub fn render(&self) -> String {
        match self {
            BackendSet::All => "all".to_string(),
            BackendSet::Only(v) => format!("[{}]", v.join(", ")),
        }
    }
}

/// Error produced by [`parse_frontmatter_backends`] when the YAML-ish
/// frontmatter text contains a malformed `backends:` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontmatterParseError {
    pub message: String,
}

impl std::fmt::Display for FrontmatterParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FrontmatterParseError {}

/// Parse a `backends:` declaration out of a frontmatter block body.
///
/// Accepted shapes (Phase 11, intentionally minimal — extensible):
///   * `backends: [duckdb, spark]`      — inline array of backend names
///   * `backends: [duckdb]`             — single-element inline array
///   * no `backends:` key               — returns `Ok(None)` (unconstrained)
///   * `backends: { duckdb: { emit: foo } }` — mapping form; only the
///     backend names are read (emit name is stored alongside). Parsed
///     minimally — a malformed nested shape returns an error.
///
/// The parser is hand-rolled: we don't want a full YAML dependency for
/// what's effectively a single key.
pub fn parse_frontmatter_backends(
    yaml_text: &str,
) -> Result<Option<BackendSet>, FrontmatterParseError> {
    for line in yaml_text.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("backends:") {
            continue;
        }
        let value = trimmed["backends:".len()..].trim();
        if value.is_empty() {
            // `backends:` on its own line — not valid in this minimal parser.
            return Err(FrontmatterParseError {
                message: "backends: expects a value on the same line (e.g. `backends: [duckdb]`)"
                    .to_string(),
            });
        }
        // Inline-array form: `[a, b, c]`.
        if value.starts_with('[') && value.ends_with(']') {
            let inner = &value[1..value.len() - 1];
            let names: Vec<String> = inner
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            return Ok(Some(BackendSet::from_names(names)));
        }
        // Mapping form: `{ duckdb: { emit: foo } }`. We only extract the
        // top-level backend keys — enough for Phase 11 narrow-check.
        if value.starts_with('{') && value.ends_with('}') {
            let inner = &value[1..value.len() - 1];
            // Split on top-level commas. Nesting is shallow so this simple
            // depth-tracker suffices.
            let mut segments: Vec<String> = Vec::new();
            let mut depth = 0i32;
            let mut buf = String::new();
            for ch in inner.chars() {
                match ch {
                    '{' | '[' => {
                        depth += 1;
                        buf.push(ch);
                    }
                    '}' | ']' => {
                        depth -= 1;
                        buf.push(ch);
                    }
                    ',' if depth == 0 => {
                        segments.push(std::mem::take(&mut buf));
                    }
                    _ => buf.push(ch),
                }
            }
            if !buf.trim().is_empty() {
                segments.push(buf);
            }
            let mut names = Vec::new();
            for seg in segments {
                let seg = seg.trim();
                if seg.is_empty() {
                    continue;
                }
                // Take the backend name: everything up to the first `:`.
                let name = seg.split(':').next().unwrap_or("").trim().to_string();
                if name.is_empty() {
                    return Err(FrontmatterParseError {
                        message: "backends: map entry missing key".to_string(),
                    });
                }
                names.push(name);
            }
            return Ok(Some(BackendSet::from_names(names)));
        }
        return Err(FrontmatterParseError {
            message: format!("backends: expects `[...]` or `{{...}}`, got {:?}", value),
        });
    }
    Ok(None)
}

/// Signature of a single `smelt.define` or `smelt.extern` declaration.
///
/// The `name_range` field points at the `DEFINE_NAME` token in the source
/// file; diagnostic-emitting code (duplicate-definition detection, etc.)
/// uses it as the anchor for error spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSig {
    /// The function's declared name.
    pub name: String,
    /// Parameters in declaration order.
    pub params: Vec<ParamSpec>,
    /// Raw text of the declared return type, or `None` if no `-> Type` clause.
    pub return_type_text: Option<String>,
    /// Structured parse of `return_type_text`, or `None` if unannotated.
    /// `Some(Err(...))` indicates a malformed return annotation.
    pub return_type: Option<Result<SmeltType, SmeltTypeParseError>>,
    /// Source range of the return `TypeRef`, suitable for anchoring
    /// diagnostics. `None` when no return type is declared.
    pub return_type_range: Option<TextRange>,
    /// Tier, derived from annotation completeness at parse time.
    pub tier: Tier,
    /// Byte-offset range of the function-name identifier, for diagnostics.
    pub name_range: TextRange,
    /// Whether this signature comes from a `smelt.define` (has a body) or
    /// a `smelt.extern` (signature-only).
    pub origin: SigOrigin,
    /// Backend set declared in frontmatter (`backends: [...]`) OR implied
    /// by a backend-namespace sugar extern (e.g. `smelt.extern duckdb.foo`).
    /// `None` when no declaration is present — the checker then infers
    /// the set from the body (or defaults to [`BackendSet::All`] for
    /// body-less externs).
    ///
    /// Introduced in Phase 11.
    pub declared_backends: Option<BackendSet>,
    /// For externs only: the optional backend-specific emit name. When
    /// the extern is declared with the `duckdb.<name>` sugar, this is
    /// populated with `<name>` so the later SQL emitter knows which
    /// backend symbol to call. For `smelt.define`, always `None`.
    ///
    /// Introduced in Phase 11.
    pub emit_name: Option<String>,
    /// Malformed-frontmatter diagnostic, if any, anchored at the
    /// declaration's name range. `Some(msg)` indicates the frontmatter
    /// block attached to this decl could not be parsed. Checkers should
    /// surface this as a `DiagnosticCode::BackendsWideningNotAllowed`-
    /// adjacent error (for now we reuse the same code since only that
    /// check is wired in Phase 11).
    pub frontmatter_parse_error: Option<String>,
    /// `true` when the declared return type carries a `NOT NULL` qualifier
    /// (Phase 5, nullability-soundness). The body must synthesise a
    /// non-nullable result when this flag is set.
    pub return_not_null: bool,
}

fn type_ref_range(type_ref: &TypeRef, _text: &str) -> TextRange {
    type_ref.syntax().text_range()
}

/// Extract a `ParamSpec` from an AST `Param`.
///
/// Pure: does not consult Salsa. Takes only the AST node and the source text
/// (for the range conversion).
///
/// Phase 16: when the parameter's `TYPE_REF` is a `TableExpr` head
/// with a `ROW_REQUIREMENT` child, we bypass the string-level
/// `parse_smelt_type` (which has no structured row-requirement
/// grammar) and build a [`SmeltType::TableExpr(Some(req))`] directly
/// from the CST via [`row_requirement_from_type_ref`].
fn extract_param_spec(param: &AstParam, text: &str) -> ParamSpec {
    let type_ref_node = param.type_ref();
    let type_ref_text = type_ref_node.as_ref().map(|t| t.text());

    // Phase 5 (nullability-soundness): detect `NOT NULL` qualifier from the
    // CST marker emitted by the parser. When present, strip "NOT NULL" from
    // the raw text before passing to the string-level `parse_smelt_type` so
    // the type parses correctly as if the qualifier were absent.
    let not_null = type_ref_node
        .as_ref()
        .map(|tr| tr.not_null())
        .unwrap_or(false);
    let clean_type_ref_text: Option<String> = if not_null {
        type_ref_text.as_deref().map(strip_not_null_from_type_text)
    } else {
        type_ref_text.clone()
    };

    let mut type_ref: Option<Result<SmeltType, SmeltTypeParseError>> =
        clean_type_ref_text.as_deref().map(parse_smelt_type);

    // Phase 16 override: replace the string-parser's best guess with
    // a CST-derived structured row-requirement when applicable.
    if let Some(tr) = &type_ref_node {
        if let Some(structured) = tableexpr_type_from_cst(tr) {
            type_ref = Some(Ok(structured));
        }
        // Phase 35 override: replace the string-parser error for
        // `Expr<Struct<{…}>>` with a CST-derived SmeltType::Struct.
        if let Some(structured) = struct_expr_type_from_cst(tr) {
            type_ref = Some(Ok(structured));
        }
    }

    let type_ref_range = type_ref_node.as_ref().map(|t| type_ref_range(t, text));
    let name_range = param.name_range();
    // Phase 19: extract the optional context binding from `EXPR_CTX`.
    let context = type_ref_node
        .as_ref()
        .and_then(|tr| tr.expr_ctx())
        .map(ContextRef);
    ParamSpec {
        name: param.name().unwrap_or_default(),
        name_range,
        type_ref_text,
        type_ref,
        type_ref_range,
        has_default: param.default_value().is_some(),
        context,
        not_null,
    }
}

/// Strip the `NOT NULL` qualifier tokens from a raw type-ref text string
/// (Phase 5, nullability-soundness).
///
/// For `Expr<Integer NOT NULL>` the parser emits the tokens "Integer NOT NULL"
/// inside the angle brackets. This helper removes the trailing " NOT NULL"
/// (case-insensitive, with optional leading whitespace) from before the last
/// `>` so that `parse_smelt_type` can parse just the inner type.
///
/// The stripping is done by looking for the rightmost occurrence of " NOT NULL"
/// (case-insensitive) in the string and removing it along with any leading
/// whitespace between the type name and `NOT NULL`.
///
/// Examples:
/// - `"Expr<Integer NOT NULL>"` → `"Expr<Integer>"`
/// - `"Expr<Numeric NOT NULL>"` → `"Expr<Numeric>"`
///
/// Returns the original text unchanged if the pattern is not found.
fn strip_not_null_from_type_text(text: &str) -> String {
    // Find "NOT NULL" (case-insensitive) in the text.
    let upper = text.to_uppercase();
    if let Some(pos) = upper.rfind("NOT NULL") {
        // Strip from the whitespace before NOT back to the type text,
        // then re-attach everything after "NULL".
        let before = text[..pos].trim_end();
        let after = &text[pos + "NOT NULL".len()..];
        format!("{}{}", before, after)
    } else {
        text.to_string()
    }
}

/// Phase 16 CST-aware extractor: if `type_ref` is a `TableExpr` head,
/// build a [`SmeltType::TableExpr`] that carries any structured
/// [`SchemaRequirement`] declared between its angle brackets.
///
/// Returns `None` when the head isn't `TableExpr` (so the string-level
/// `parse_smelt_type` wins), and when the head is `TableExpr` but
/// contains no `ROW_REQUIREMENT` (bare case — the string parser
/// already produces the same `SmeltType::TableExpr(None)`).
///
/// Pure — walks only the CST node.
fn tableexpr_type_from_cst(tr: &TypeRef) -> Option<SmeltType> {
    use smelt_parser::ast::{RowTail as AstRowTail, TypeRefHead};
    if tr.kind() != TypeRefHead::TableExpr {
        return None;
    }
    let Some(req_node) = tr.row_requirement() else {
        // Bare `TableExpr` — let the string-parser's
        // `SmeltType::TableExpr(None)` stand.
        return None;
    };

    // Build the ordered `(name, DataTypeReq, not_null)` list from the CST's
    // ROW_FIELD children.
    let mut required: Vec<(String, DataTypeReq, bool)> = Vec::new();
    for field in req_node.fields() {
        let Some(name) = field.name() else { continue };
        // Inner type is a flat type reference (e.g. `Numeric`,
        // `Integer`, `Text`) — reuse `parse_inner_constraint` so
        // constraint keywords and concrete types go through the same
        // classifier as `Expr<T>`'s inner.
        let inner_text = field.type_ref().map(|t| t.text()).unwrap_or_default();
        let req = match parse_inner_constraint(inner_text.trim()) {
            Some(TypeConstraint::Concrete(dt)) => DataTypeReq::Concrete(dt),
            Some(c) => DataTypeReq::Constraint(c),
            None => {
                // Unknown inner type — fall back to `Any` so
                // downstream code doesn't panic, and the failure
                // surfaces as a match against `Any` (always accepts).
                // A later phase can lift this into a dedicated
                // `RowRequirementUnknownInner` diagnostic.
                DataTypeReq::Constraint(TypeConstraint::Any)
            }
        };
        // Phase 5: read the NOT NULL qualifier from the ROW_FIELD's
        // NOT_NULL_QUALIFIER child (placed as a sibling of TYPE_REF).
        let not_null = field.not_null();
        required.push((name, req, not_null));
    }

    let tail = match req_node.tail() {
        AstRowTail::None => RowTail::None,
        AstRowTail::Anon => RowTail::Anon,
        AstRowTail::Named(n) => RowTail::Named(n),
    };

    Some(SmeltType::TableExpr(Some(SchemaRequirement {
        required,
        tail,
    })))
}

/// Phase 35 CST-aware extractor: if `type_ref` is an `Expr<Struct<{…}>>` head,
/// build a [`SmeltType::Struct`] that carries the declared field list and tail.
///
/// Returns `None` when the head isn't `Expr` wrapping a `STRUCT_TYPE` (so the
/// string-level `parse_smelt_type` wins or already succeeded). When a
/// `STRUCT_TYPE` is found, this always overrides the string-parser's error
/// (which reports `UnknownInner` because the string parser can't handle the
/// nested `Struct<{…}>` form).
///
/// Pure — walks only the CST node.
/// Returns the `TextRange` of each struct field whose type text cannot be
/// parsed as a recognised concrete `DataType`. Used by the diagnostic layer to
/// emit `UnknownStructFieldType` at the individual field's span.
///
/// Returns an empty `Vec` when the `TypeRef` is not an `Expr<Struct<{…}>>` shape,
/// or when all field types are valid.
///
/// Pure — walks only the CST node, no Salsa dependency.
pub fn struct_field_unknown_ranges(tr: &TypeRef) -> Vec<TextRange> {
    use smelt_parser::ast::{StructType, TypeRefHead};
    if tr.kind() != TypeRefHead::Expr {
        return vec![];
    }
    let struct_node = match tr.syntax().descendants().find_map(StructType::cast) {
        Some(n) => n,
        None => return vec![],
    };
    let mut errors = Vec::new();
    for sf in struct_node.fields() {
        let type_ref_node = sf.type_ref();
        let inner_text = type_ref_node.as_ref().map(|t| t.text()).unwrap_or_default();
        if crate::parse_type(inner_text.trim()).is_err() {
            if let Some(field_tr) = type_ref_node {
                errors.push(field_tr.syntax().text_range());
            }
        }
    }
    errors
}

fn struct_expr_type_from_cst(tr: &TypeRef) -> Option<SmeltType> {
    use smelt_parser::ast::{StructType, TypeRefHead};

    if tr.kind() != TypeRefHead::Expr {
        return None;
    }
    // Look for a STRUCT_TYPE descendant inside the TYPE_REF.
    let struct_node = tr.syntax().descendants().find_map(StructType::cast)?;

    let mut fields: Vec<(String, DataType)> = Vec::new();
    for sf in struct_node.fields() {
        let Some(name) = sf.name() else { continue };
        let inner_text = sf.type_ref().map(|t| t.text()).unwrap_or_default();
        // Parse the field's type as a concrete DataType (struct fields must be
        // concrete in v1 — constraints like `Numeric` are not yet supported
        // inside struct field positions). Fall back to Unknown for unrecognised
        // names so diagnostics don't cascade.
        let dt = crate::parse_type(inner_text.trim())
            .unwrap_or(DataType::Unknown(crate::UnknownReason::Dynamic));
        fields.push((name, dt));
    }

    let tail = match struct_node.row_tail() {
        None => StructRowTail::None,
        Some(t) => match t.var_name() {
            None => StructRowTail::Anon,
            Some(n) => StructRowTail::Named(n),
        },
    };

    Some(SmeltType::Struct { fields, tail })
}

fn compute_tier(params: &[ParamSpec], return_type_text: Option<&str>) -> Tier {
    // A parameter counts as "annotated" only when its type_ref parsed
    // successfully (Some(Ok(_))).  A malformed annotation — one that
    // produces Some(Err(_)) — is treated as unannotated, demoting the
    // function to Tier 1 (spec: gradual_typing.md §"Tier dispatch is
    // implicit": "a malformed annotation (one that fails
    // InvalidFunctionTypeRef) is treated as unannotated").
    // Note: unannotated params (type_ref == None) also fall through to
    // Tier 1 via the `_ => false` arm below.
    let all_params_typed = params.iter().all(|p| matches!(&p.type_ref, Some(Ok(_))));
    if !all_params_typed {
        return Tier::One;
    }
    if return_type_text.is_some() {
        Tier::Three
    } else {
        Tier::Two
    }
}

/// Convert a `SmeltDefine` AST node into a `FunctionSig`.
///
/// Returns `None` if the declaration is missing a name (error recovery
/// produced a fragment without an identifier). File text is required so
/// the `name_range` can be rendered as a line/column range — `Range` is
/// the same type returned by other `smelt-db` queries.
///
/// Phase 11: `raw_text` is the pre-strip source text. It's used to
/// attach per-declaration frontmatter blocks. For the common "stripped
/// text == raw text" case (callers without raw access), pass the
/// stripped text — `attach_frontmatter_to_decls` simply finds no blocks
/// and `declared_backends` stays `None`.
pub fn extract_signature(define: &SmeltDefine, text: &str) -> Option<FunctionSig> {
    extract_signature_with_raw(define, text, text)
}

/// `extract_signature` plus raw-text access for per-declaration
/// frontmatter attachment (Phase 11).
pub fn extract_signature_with_raw(
    define: &SmeltDefine,
    stripped_text: &str,
    raw_text: &str,
) -> Option<FunctionSig> {
    let name = define.name()?;
    let name_range = define.name_range()?;

    let params: Vec<ParamSpec> = define
        .param_list()
        .map(|pl| {
            pl.params()
                .map(|p| extract_param_spec(&p, stripped_text))
                .collect()
        })
        .unwrap_or_default();

    let return_type_node = define.return_type();
    let return_type_text = return_type_node.as_ref().map(|t| t.text());

    // Phase 5 (nullability-soundness): detect NOT NULL on return type.
    let return_not_null = return_type_node
        .as_ref()
        .map(|tr| tr.not_null())
        .unwrap_or(false);
    let clean_return_type_text: Option<String> = if return_not_null {
        return_type_text
            .as_deref()
            .map(strip_not_null_from_type_text)
    } else {
        return_type_text.clone()
    };

    let mut return_type: Option<Result<SmeltType, SmeltTypeParseError>> =
        clean_return_type_text.as_deref().map(parse_smelt_type);
    if let Some(tr) = &return_type_node {
        if let Some(structured) = tableexpr_type_from_cst(tr) {
            return_type = Some(Ok(structured));
        } else if let Some(structured) = struct_expr_type_from_cst(tr) {
            return_type = Some(Ok(structured));
        }
    }
    let return_type_range = return_type_node
        .as_ref()
        .map(|t| type_ref_range(t, stripped_text));
    // Pass the cleaned text (without NOT NULL) to compute_tier so the tier is
    // derived from the presence of a return annotation, not the qualifier.
    let tier = compute_tier(&params, clean_return_type_text.as_deref());

    let (declared_backends, frontmatter_parse_error) =
        parse_frontmatter_on_define(define, raw_text);

    Some(FunctionSig {
        name,
        params,
        return_type_text,
        return_type,
        return_type_range,
        tier,
        name_range,
        origin: SigOrigin::Define,
        declared_backends,
        emit_name: None,
        frontmatter_parse_error,
        return_not_null,
    })
}

/// Read the per-decl frontmatter attached to `define` and parse a
/// `backends:` directive if present.
fn parse_frontmatter_on_define(
    define: &SmeltDefine,
    raw_text: &str,
) -> (Option<BackendSet>, Option<String>) {
    let Some(fm) = define.frontmatter(raw_text) else {
        return (None, None);
    };
    match parse_frontmatter_backends(&fm) {
        Ok(backends) => (backends, None),
        Err(e) => (None, Some(e.message)),
    }
}

/// Convert a `SmeltExtern` AST node into a `FunctionSig`.
///
/// Externs have no body but share the same signature surface as defines.
/// Returns `None` when the declaration is missing a name.
pub fn extract_extern_signature(ext: &SmeltExtern, text: &str) -> Option<FunctionSig> {
    extract_extern_signature_with_raw(ext, text, text)
}

/// `extract_extern_signature` plus raw-text access for per-decl
/// frontmatter attachment (Phase 11). Also populates the backend
/// namespace when the extern uses the `duckdb.<name>` sugar.
pub fn extract_extern_signature_with_raw(
    ext: &SmeltExtern,
    stripped_text: &str,
    raw_text: &str,
) -> Option<FunctionSig> {
    let name = ext.name()?;
    let name_range = ext.name_range()?;

    let params: Vec<ParamSpec> = ext
        .param_list()
        .map(|pl| {
            pl.params()
                .map(|p| extract_param_spec(&p, stripped_text))
                .collect()
        })
        .unwrap_or_default();

    let return_type_node = ext.return_type();
    let return_type_text = return_type_node.as_ref().map(|t| t.text());
    let return_not_null = return_type_node
        .as_ref()
        .map(|tr| tr.not_null())
        .unwrap_or(false);
    let clean_return_type_text: Option<String> = if return_not_null {
        return_type_text
            .as_deref()
            .map(strip_not_null_from_type_text)
    } else {
        return_type_text.clone()
    };
    let mut return_type: Option<Result<SmeltType, SmeltTypeParseError>> =
        clean_return_type_text.as_deref().map(parse_smelt_type);
    if let Some(tr) = &return_type_node {
        if let Some(structured) = tableexpr_type_from_cst(tr) {
            return_type = Some(Ok(structured));
        } else if let Some(structured) = struct_expr_type_from_cst(tr) {
            return_type = Some(Ok(structured));
        }
    }
    let return_type_range = return_type_node
        .as_ref()
        .map(|t| type_ref_range(t, stripped_text));
    let tier = compute_tier(&params, clean_return_type_text.as_deref());

    // Phase 11: frontmatter-declared backends take precedence, but the
    // backend-namespace sugar (`smelt.extern duckdb.foo`) implies a
    // [duckdb] constraint when no frontmatter is present. If both are
    // present and agree, they stack; disagreement lets the frontmatter
    // win (it is more explicit).
    let (mut declared_backends, frontmatter_parse_error) = {
        match ext.frontmatter(raw_text) {
            Some(fm) => match parse_frontmatter_backends(&fm) {
                Ok(b) => (b, None),
                Err(e) => (None, Some(e.message)),
            },
            None => (None, None),
        }
    };
    let mut emit_name: Option<String> = None;
    if let Some(backend) = ext.backend_namespace() {
        let sugar_set = BackendSet::from_names([backend.clone()]);
        // The sugar also implies the smelt-level name is the emit name.
        emit_name = Some(name.clone());
        declared_backends = match declared_backends {
            None => Some(sugar_set),
            Some(explicit) => Some(explicit),
        };
        let _ = backend; // keep for future use.
    }

    Some(FunctionSig {
        name,
        params,
        return_type_text,
        return_type,
        return_type_range,
        tier,
        name_range,
        origin: SigOrigin::Extern,
        declared_backends,
        emit_name,
        frontmatter_parse_error,
        return_not_null,
    })
}

/// Extract all function signatures from a parsed file.
///
/// Pure function: takes an AST + source text and returns a freshly
/// allocated vector of signatures in declaration order. Includes both
/// `smelt.define` and `smelt.extern` declarations — callers inspect
/// `FunctionSig::origin` to distinguish them.
///
/// Order: externs interleave with defines in the order they appear in the
/// syntax tree, so downstream per-declaration diagnostics fire in source
/// order regardless of origin.
pub fn extract_function_signatures(file: &AstFile, text: &str) -> Vec<FunctionSig> {
    extract_function_signatures_with_raw(file, text, text)
}

/// `extract_function_signatures` variant that takes both the stripped
/// source (used for the parsed AST ranges) and the raw pre-strip source
/// (used to look up per-declaration frontmatter). Phase 11 call sites
/// pass the two explicitly; legacy callers using `extract_function_signatures`
/// get frontmatter-less signatures (`declared_backends = None`).
pub fn extract_function_signatures_with_raw(
    file: &AstFile,
    stripped_text: &str,
    raw_text: &str,
) -> Vec<FunctionSig> {
    let mut out: Vec<FunctionSig> = Vec::new();
    for child in file.syntax().children() {
        if let Some(d) = SmeltDefine::cast(child.clone()) {
            if let Some(sig) = extract_signature_with_raw(&d, stripped_text, raw_text) {
                out.push(sig);
            }
        } else if let Some(e) = SmeltExtern::cast(child) {
            if let Some(sig) = extract_extern_signature_with_raw(&e, stripped_text, raw_text) {
                out.push(sig);
            }
        }
    }
    out
}

/// Extract the signature for a single named define, if present.
///
/// Returns `None` when no declaration with the given name exists. If
/// multiple declarations share the name (which is a diagnostic), the
/// first in declaration order wins.
pub fn extract_function_signature_by_name(
    file: &AstFile,
    text: &str,
    name: &str,
) -> Option<FunctionSig> {
    file.defines()
        .filter_map(|d| extract_signature(&d, text))
        .find(|sig| sig.name == name)
}

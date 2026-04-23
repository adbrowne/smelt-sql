//! Function signature indexing for `smelt.define` declarations.
//!
//! This module defines the data shape used by the Salsa-backed function
//! registry (`smelt-db::functions_in_file`, `function_signature`, etc.) and
//! the pure extraction function that produces it from a parsed AST.
//!
//! Pure-function rule (CLAUDE.md): everything here is dependency-free
//! w.r.t. Salsa. Callers in `smelt-db` are responsible for wiring these
//! extractors into tracked queries.
//!
//! Phase 3 scope: raw `type_ref_text` only. Phase 4 adds structured `Expr<T>`
//! parsing into [`SmeltType`], alongside [`TypeConstraint`] for the `Numeric`
//! / `Any` constraints per §16 #9. Phase 7 adds the [`TypeConstraint::Ordered`]
//! member (§16 #13) and a monomorphic [`BuiltinRegistry`] skeleton seeded with
//! a handful of SQL built-ins. Generics, variadics, and non-`Expr` sorts
//! (TableExpr, AggExpr, …) are deferred to later phases of the smelt-functions
//! plan.

use crate::{parse_type, DataType};
use smelt_parser::ast::{File as AstFile, Param as AstParam, Range, SmeltDefine, TypeRef};
use smelt_parser::offset_to_position;
use std::collections::HashMap;
use std::sync::LazyLock;

/// Constraint placed on the type parameter of a fragment sort (e.g. `Expr<T>`).
///
/// `Concrete(dt)` pins the parameter to a single [`DataType`]. The abstract
/// constraints (`Numeric`, `Any`) are drawn from §16 #9 of the smelt-functions
/// research. Additional constraints (`Ordered`, …) land in later phases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeConstraint {
    /// A single concrete [`DataType`] such as `Integer` or `Boolean`.
    Concrete(DataType),
    /// Any numeric type per §16 #9: `SmallInt`, `Integer`, `BigInt`, `Float`,
    /// `Double`, `Decimal`. Boolean is deliberately excluded.
    Numeric,
    /// Any type with a total order on every v1 backend, per §16 #13.
    ///
    /// Members: `Numeric` ∪ the string family (`Text`, `Varchar`, `Char`) ∪
    /// the temporal family (`Date`, `Time`, `Timestamp` with either tz, and
    /// `Interval`) ∪ `Boolean` ∪ `Blob`. Composite types (`Array`, `Struct`,
    /// `Map`) are excluded in v1 because their ordering semantics diverge
    /// across backends — `Null`/`Unknown` are likewise non-members.
    Ordered,
    /// Any type — effectively "no constraint". Reserved for parameters like
    /// `x: Expr<Any>`.
    Any,
}

impl TypeConstraint {
    /// Does the given [`DataType`] satisfy this constraint?
    ///
    /// Pure, deterministic, no Salsa dependency. Used both in signature checks
    /// and as the oracle for the registry-wide `numeric_constraint_*` tests.
    pub fn satisfies(&self, dt: &DataType) -> bool {
        match self {
            TypeConstraint::Concrete(expected) => expected == dt,
            // Centralise the numeric membership on `DataType::is_numeric()` so
            // there is a single source of truth matching §16 #9.
            TypeConstraint::Numeric => dt.is_numeric(),
            // §16 #13 members: numeric ∪ string family ∪ temporal family ∪
            // {Boolean, Blob}. Composite types, `Null`, and `Unknown` are
            // intentionally not members.
            TypeConstraint::Ordered => {
                dt.is_numeric()
                    || dt.is_string()
                    || dt.is_temporal()
                    || matches!(dt, DataType::Boolean | DataType::Blob)
            }
            TypeConstraint::Any => true,
        }
    }
}

/// Parsed smelt type reference.
///
/// Phase 4 only models the `Expr<T>` sort. Later phases will extend this with
/// `TableExpr`, `AggExpr`, etc. Unsupported sorts surface as
/// [`SmeltTypeParseError::UnsupportedSort`] rather than panicking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmeltType {
    /// `Expr<T>` where T is a [`TypeConstraint`] — either a concrete
    /// [`DataType`] or one of the abstract constraints in
    /// [`TypeConstraint`].
    Expr(TypeConstraint),
}

/// Error produced when a type-reference annotation can't be resolved into a
/// structured [`SmeltType`].
///
/// Every variant carries the raw `span_text` (the original user-written
/// annotation) so diagnostic messages can quote what the user actually wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmeltTypeParseError {
    /// A recognised sort keyword other than `Expr` — e.g. `TableExpr<T>`,
    /// `AggExpr<T>`, `WindowExpr<T>`. Deferred to Step 3 of the plan.
    UnsupportedSort { sort: String, span_text: String },
    /// `Expr<Expr<Integer>>` — the inner parameter must itself be a concrete
    /// type or a constraint, never another sort.
    NestedExpr { span_text: String },
    /// The inner payload wasn't recognised as either a known concrete type or
    /// a known constraint (e.g. `Expr<FooBar>`).
    UnknownInner { inner: String, span_text: String },
    /// The annotation couldn't be lexically split into `Sort<Inner>` — missing
    /// angle brackets, empty, unbalanced, etc.
    Malformed { span_text: String },
}

impl std::fmt::Display for SmeltTypeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SmeltTypeParseError::UnsupportedSort { sort, span_text } => write!(
                f,
                "unsupported type sort `{sort}` in `{span_text}` (only `Expr<T>` is supported in Step 1)"
            ),
            SmeltTypeParseError::NestedExpr { span_text } => write!(
                f,
                "nested `Expr<...>` is not allowed in `{span_text}`; the inner must be a concrete type or constraint"
            ),
            SmeltTypeParseError::UnknownInner { inner, span_text } => write!(
                f,
                "unknown type `{inner}` in `{span_text}`"
            ),
            SmeltTypeParseError::Malformed { span_text } => {
                write!(f, "malformed type reference: `{span_text}`")
            }
        }
    }
}

impl std::error::Error for SmeltTypeParseError {}

/// Parse a type-reference annotation such as `Expr<Integer>` or
/// `Expr<Numeric>` into a structured [`SmeltType`].
///
/// This is deliberately a string-level parser — the Rowan CST's `TypeRef` is
/// a flat token run (see `parse_type_ref` in `smelt-parser`) and the grammar
/// we accept here is intentionally tiny:
///
/// ```text
/// TypeRef  := Sort '<' Inner '>'
/// Sort     := 'Expr'
/// Inner    := <concrete-DataType-name> | 'Numeric' | 'Any'
/// ```
///
/// Anything else is an error. In particular, the recognised non-`Expr` sorts
/// (`TableExpr`, `AggExpr`, `WindowExpr`, `SelectItems`, `OrderSpec`) deliver
/// [`SmeltTypeParseError::UnsupportedSort`] with a clear, actionable message
/// pointing the user at a future phase.
pub fn parse_smelt_type(text: &str) -> Result<SmeltType, SmeltTypeParseError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(SmeltTypeParseError::Malformed {
            span_text: text.to_string(),
        });
    }

    // Split on the first '<'. We intentionally require brackets — bare
    // `Integer` or similar is not a valid Step-1 type reference.
    let Some(lt_idx) = trimmed.find('<') else {
        return Err(SmeltTypeParseError::Malformed {
            span_text: text.to_string(),
        });
    };
    let Some(gt_idx) = trimmed.rfind('>') else {
        return Err(SmeltTypeParseError::Malformed {
            span_text: text.to_string(),
        });
    };
    if gt_idx <= lt_idx + 1 {
        return Err(SmeltTypeParseError::Malformed {
            span_text: text.to_string(),
        });
    }

    // Anything after the closing '>' must be whitespace only.
    let tail = trimmed[gt_idx + 1..].trim();
    if !tail.is_empty() {
        return Err(SmeltTypeParseError::Malformed {
            span_text: text.to_string(),
        });
    }

    let sort = trimmed[..lt_idx].trim();
    let inner_raw = trimmed[lt_idx + 1..gt_idx].trim();
    if inner_raw.is_empty() {
        return Err(SmeltTypeParseError::Malformed {
            span_text: text.to_string(),
        });
    }

    // `<` in the inner means another generic — reject as nested Expr if the
    // inner sort is `Expr`, otherwise unsupported sort.
    if let Some(inner_lt) = inner_raw.find('<') {
        let inner_sort = inner_raw[..inner_lt].trim();
        if sort == "Expr" && inner_sort == "Expr" {
            return Err(SmeltTypeParseError::NestedExpr {
                span_text: text.to_string(),
            });
        }
        // Other nested sort in an Expr<...> — still malformed from Step 1's
        // perspective. Surface the outer sort decision first.
        if sort != "Expr" {
            return Err(SmeltTypeParseError::UnsupportedSort {
                sort: sort.to_string(),
                span_text: text.to_string(),
            });
        }
        return Err(SmeltTypeParseError::UnknownInner {
            inner: inner_raw.to_string(),
            span_text: text.to_string(),
        });
    }

    // Sort dispatch.
    match sort {
        "Expr" => {
            let constraint = parse_inner_constraint(inner_raw).ok_or_else(|| {
                SmeltTypeParseError::UnknownInner {
                    inner: inner_raw.to_string(),
                    span_text: text.to_string(),
                }
            })?;
            Ok(SmeltType::Expr(constraint))
        }
        "TableExpr" | "AggExpr" | "WindowExpr" | "SelectItems" | "OrderSpec" => {
            Err(SmeltTypeParseError::UnsupportedSort {
                sort: sort.to_string(),
                span_text: text.to_string(),
            })
        }
        // Unknown sort keyword — treat as malformed so we're not lenient about
        // typos. Callers show the span text so users can see what they wrote.
        other if !other.is_empty() => Err(SmeltTypeParseError::UnsupportedSort {
            sort: other.to_string(),
            span_text: text.to_string(),
        }),
        _ => Err(SmeltTypeParseError::Malformed {
            span_text: text.to_string(),
        }),
    }
}

/// Recognise the inner payload of an `Expr<...>`.
///
/// The order matters: constraint keywords (`Numeric`, `Any`) take priority over
/// concrete type names so a future `Ordered` constraint can live alongside
/// whatever `parse_type` might try to map to a concrete type.
fn parse_inner_constraint(inner: &str) -> Option<TypeConstraint> {
    match inner {
        "Numeric" => Some(TypeConstraint::Numeric),
        "Ordered" => Some(TypeConstraint::Ordered),
        "Any" => Some(TypeConstraint::Any),
        other => parse_type(other).ok().map(TypeConstraint::Concrete),
    }
}

/// Description of a single parameter in a `smelt.define`.
///
/// `type_ref_text` is the raw source text of the `TypeRef` node (e.g.
/// `"Expr<Numeric>"`) or `None` when the parameter is unannotated. Phase 4
/// adds a parsed `type_ref` alongside it; callers that need the structured
/// form should prefer that field. `type_ref_text` stays in place (Option A
/// from the plan) so downstream integration tests that read the raw text
/// don't need to change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamSpec {
    /// The parameter's declared name.
    pub name: String,
    /// Source range of the parameter-name identifier, suitable for anchoring
    /// diagnostics (notably `DuplicateParameterName` in Phase 5). `None` only
    /// if the declaration was so malformed that no IDENT token was present in
    /// the PARAM node.
    pub name_range: Option<Range>,
    /// Raw text of the declared type, or `None` if unannotated.
    pub type_ref_text: Option<String>,
    /// Structured parse of `type_ref_text`, or `None` if unannotated.
    /// `Some(Err(...))` when an annotation was written but couldn't be parsed
    /// — these errors are surfaced as diagnostics by higher layers.
    pub type_ref: Option<Result<SmeltType, SmeltTypeParseError>>,
    /// Source range of the `TypeRef` node for this parameter, suitable for
    /// anchoring diagnostic spans. `None` when the parameter has no
    /// annotation at all.
    pub type_ref_range: Option<Range>,
    /// `true` when the parameter has a default value.
    pub has_default: bool,
}

/// A single frame of expansion context attached to a body/call-site
/// diagnostic.
///
/// Phase 6 populates a 0-or-1-element `Vec<FrameInfo>` on every
/// `smelt.fn.*`-originated diagnostic:
///
/// - When the body's own type-check surfaces an error (no expansion
///   happened), the vec is empty.
/// - When the error fires *inside* an expanded body, the vec contains
///   one frame for each nested expansion, innermost-first → outermost-last
///   (Phase 12 upgrades the renderer to emit multi-level; Phase 6 only renders
///   the innermost).
///
/// `bound_type` is the concrete type that the parameter was bound to at the
/// call site, rendered via `DataType::to_string()` for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameInfo {
    /// Name of the function whose expansion is responsible for this frame
    /// (e.g. `"safe_divide"`).
    pub function: String,
    /// Name of the parameter inside that function whose binding produced
    /// the inner error (e.g. `"numerator"`).
    pub param: String,
    /// Textual rendering of the type that `param` was bound to at this
    /// call site (e.g. `"INTEGER"`, `"VARCHAR"`).
    pub bound_type: String,
}

/// Tier of a function, derived from annotation completeness.
///
/// - `Tier::Three`: every parameter annotated AND return type annotated.
/// - `Tier::Two`: every parameter annotated, return type missing.
/// - `Tier::One`: at least one parameter unannotated (return-type state
///   irrelevant for this tier — unannotated inputs force Tier 1).
///
/// Behavioural differences between tiers (bidirectional checking, param
/// coercion, etc.) land in Steps 5+.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    One,
    Two,
    Three,
}

/// Signature of a single `smelt.define` declaration.
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
    pub return_type_range: Option<Range>,
    /// Tier, derived from annotation completeness at parse time.
    pub tier: Tier,
    /// Line/column range of the function-name identifier, for diagnostics.
    pub name_range: Range,
}

fn type_ref_range(type_ref: &TypeRef, text: &str) -> Range {
    let tr = type_ref.syntax().text_range();
    Range {
        start: offset_to_position(text, usize::from(tr.start())),
        end: offset_to_position(text, usize::from(tr.end())),
    }
}

/// Extract a `ParamSpec` from an AST `Param`.
///
/// Pure: does not consult Salsa. Takes only the AST node and the source text
/// (for the range conversion).
fn extract_param_spec(param: &AstParam, text: &str) -> ParamSpec {
    let type_ref_node = param.type_ref();
    let type_ref_text = type_ref_node.as_ref().map(|t| t.text());
    let type_ref = type_ref_text.as_deref().map(parse_smelt_type);
    let type_ref_range = type_ref_node.as_ref().map(|t| type_ref_range(t, text));
    let name_range = param.name_range().map(|r| Range {
        start: offset_to_position(text, usize::from(r.start())),
        end: offset_to_position(text, usize::from(r.end())),
    });
    ParamSpec {
        name: param.name().unwrap_or_default(),
        name_range,
        type_ref_text,
        type_ref,
        type_ref_range,
        has_default: param.default_value().is_some(),
    }
}

fn compute_tier(params: &[ParamSpec], return_type_text: Option<&str>) -> Tier {
    let all_params_typed = params.iter().all(|p| p.type_ref_text.is_some());
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
pub fn extract_signature(define: &SmeltDefine, text: &str) -> Option<FunctionSig> {
    let name = define.name()?;
    let name_text_range = define.name_range()?;
    let name_range = Range {
        start: offset_to_position(text, usize::from(name_text_range.start())),
        end: offset_to_position(text, usize::from(name_text_range.end())),
    };

    let params: Vec<ParamSpec> = define
        .param_list()
        .map(|pl| pl.params().map(|p| extract_param_spec(&p, text)).collect())
        .unwrap_or_default();

    let return_type_node = define.return_type();
    let return_type_text = return_type_node.as_ref().map(|t| t.text());
    let return_type = return_type_text.as_deref().map(parse_smelt_type);
    let return_type_range = return_type_node.as_ref().map(|t| type_ref_range(t, text));
    let tier = compute_tier(&params, return_type_text.as_deref());

    Some(FunctionSig {
        name,
        params,
        return_type_text,
        return_type,
        return_type_range,
        tier,
        name_range,
    })
}

/// Extract all function signatures from a parsed file.
///
/// Pure function: takes an AST + source text and returns a freshly
/// allocated vector of signatures in declaration order. Callers in
/// `smelt-db` wrap this in a Salsa tracked query.
pub fn extract_function_signatures(file: &AstFile, text: &str) -> Vec<FunctionSig> {
    file.defines()
        .filter_map(|d| extract_signature(&d, text))
        .collect()
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

/// Monomorphic signature of a SQL built-in in the canonical registry.
///
/// Phase 7 ships a deliberately tiny shape: every parameter and the return
/// type are [`TypeConstraint`] values. Concrete scalar entries use
/// [`TypeConstraint::Concrete`]; the `Numeric` / `Ordered` / `Any` constraints
/// exist for forward-compatibility but none of the Phase 7 seeds use them
/// directly. Generics and variadics land in Phase 8 and will extend this
/// type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// Canonical (upper-cased) function name.
    pub name: String,
    /// Positional parameter constraints, in declaration order.
    pub params: Vec<TypeConstraint>,
    /// Return type constraint.
    pub return_type: TypeConstraint,
}

/// Canonical registry of SQL built-in signatures.
///
/// Phase 7 seeds the registry with four monomorphic scalar entries (`LOWER`,
/// `UPPER`, `LENGTH`, `ABS`) so downstream phases (and this phase's tests)
/// can exercise the lookup surface. The registry is populated once at
/// program start via [`std::sync::LazyLock`], stays `'static`, and is
/// keyed by ASCII-uppercased name — callers use [`BuiltinRegistry::resolve`]
/// which folds case at the boundary.
///
/// The registry is *data only*: it has no Salsa dependency, no inference
/// wiring, and is not yet consumed by the type checker (the hand-written
/// match in `smelt-db::type_inference` continues to drive inference until
/// Phase 9 rewires it).
pub struct BuiltinRegistry;

impl BuiltinRegistry {
    /// Resolve a built-in by name, case-insensitively (ASCII folding).
    ///
    /// Returns `Some(&'static Signature)` when the name matches a registered
    /// entry, `None` otherwise.
    pub fn resolve(name: &str) -> Option<&'static Signature> {
        REGISTRY.get(&name.to_ascii_uppercase())
    }

    /// Iterator over all canonical (upper-cased) names in the registry.
    pub fn names() -> impl Iterator<Item = &'static str> {
        REGISTRY.keys().map(|s| s.as_str())
    }
}

static REGISTRY: LazyLock<HashMap<String, Signature>> = LazyLock::new(|| {
    let mut m: HashMap<String, Signature> = HashMap::new();
    let text = || TypeConstraint::Concrete(DataType::Text);
    let integer = || TypeConstraint::Concrete(DataType::Integer);
    let double = || TypeConstraint::Concrete(DataType::Double);
    let mut insert = |name: &str, params: Vec<TypeConstraint>, return_type: TypeConstraint| {
        m.insert(
            name.to_string(),
            Signature {
                name: name.to_string(),
                params,
                return_type,
            },
        );
    };
    // Phase 7 seeds. All entries are monomorphic scalars; Phase 8 will
    // add generic and variadic forms.
    insert("LOWER", vec![text()], text());
    insert("UPPER", vec![text()], text());
    insert("LENGTH", vec![text()], integer());
    insert("ABS", vec![double()], double());
    m
});

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_parser::{parse, File as AstFile};

    fn parse_file(text: &str) -> (AstFile, String) {
        let clean = smelt_parser::strip_frontmatter(text);
        let parse = parse(&clean);
        let ast = AstFile::cast(parse.syntax()).expect("FILE node");
        (ast, clean)
    }

    // === Phase 3 tests (still passing) ===

    #[test]
    fn extracts_minimal_signature() {
        let (file, text) = parse_file("smelt.define foo(x) AS (x + 1)");
        let sigs = extract_function_signatures(&file, &text);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].name, "foo");
        assert_eq!(sigs[0].params.len(), 1);
        assert_eq!(sigs[0].params[0].name, "x");
        assert!(sigs[0].params[0].type_ref_text.is_none());
        assert!(sigs[0].params[0].type_ref.is_none());
        assert!(sigs[0].return_type_text.is_none());
        assert!(sigs[0].return_type.is_none());
        assert_eq!(sigs[0].tier, Tier::One);
    }

    #[test]
    fn tier_two_when_params_annotated_return_missing() {
        let (file, text) =
            parse_file("smelt.define f(x: Expr<Integer>, y: Expr<Integer>) AS (x + y)");
        let sigs = extract_function_signatures(&file, &text);
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].tier, Tier::Two);
    }

    #[test]
    fn tier_three_when_fully_annotated() {
        let (file, text) =
            parse_file("smelt.define f(x: Expr<Integer>) -> Expr<Integer> AS (x + 1)");
        let sigs = extract_function_signatures(&file, &text);
        assert_eq!(sigs[0].tier, Tier::Three);
        let ret = sigs[0].return_type_text.as_deref().unwrap();
        assert!(
            ret.contains("Expr<Integer>"),
            "expected return text to contain Expr<Integer>, got {ret:?}"
        );
    }

    #[test]
    fn default_value_flagged() {
        let (file, text) = parse_file("smelt.define f(x: Expr<Integer> = 0) AS (x)");
        let sigs = extract_function_signatures(&file, &text);
        assert!(sigs[0].params[0].has_default);
    }

    #[test]
    fn lookup_by_name() {
        let (file, text) = parse_file("smelt.define a(x) AS (x)\nsmelt.define b(y) AS (y)\n");
        let sig = extract_function_signature_by_name(&file, &text, "b").unwrap();
        assert_eq!(sig.name, "b");
        assert!(extract_function_signature_by_name(&file, &text, "nope").is_none());
    }

    // === Phase 4 TDD tests ===

    #[test]
    fn parses_expr_of_concrete_type() {
        assert_eq!(
            parse_smelt_type("Expr<Integer>"),
            Ok(SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)))
        );
    }

    #[test]
    fn parses_expr_of_boolean_concrete_type() {
        // The plan explicitly calls out Boolean as a required concrete case.
        assert_eq!(
            parse_smelt_type("Expr<Boolean>"),
            Ok(SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean)))
        );
    }

    #[test]
    fn parses_expr_of_numeric_constraint() {
        assert_eq!(
            parse_smelt_type("Expr<Numeric>"),
            Ok(SmeltType::Expr(TypeConstraint::Numeric))
        );
    }

    #[test]
    fn rejects_unknown_sort() {
        // `TableExpr<T>` — explicitly deferred to Step 3. We expect
        // `UnsupportedSort` with the sort keyword exposed to the user.
        match parse_smelt_type("TableExpr<T>") {
            Err(SmeltTypeParseError::UnsupportedSort { sort, span_text }) => {
                assert_eq!(sort, "TableExpr");
                assert_eq!(span_text, "TableExpr<T>");
            }
            other => panic!("expected UnsupportedSort, got {other:?}"),
        }
    }

    #[test]
    fn rejects_nested_expr() {
        match parse_smelt_type("Expr<Expr<Integer>>") {
            Err(SmeltTypeParseError::NestedExpr { span_text }) => {
                assert_eq!(span_text, "Expr<Expr<Integer>>");
            }
            other => panic!("expected NestedExpr, got {other:?}"),
        }
    }

    #[test]
    fn numeric_constraint_accepts_integer() {
        let c = TypeConstraint::Numeric;
        // Full membership of §16 #9.
        assert!(c.satisfies(&DataType::SmallInt));
        assert!(c.satisfies(&DataType::Integer));
        assert!(c.satisfies(&DataType::BigInt));
        assert!(c.satisfies(&DataType::Float));
        assert!(c.satisfies(&DataType::Double));
        assert!(c.satisfies(&DataType::Decimal {
            precision: 10,
            scale: 2,
        }));
    }

    #[test]
    fn numeric_constraint_rejects_text() {
        let c = TypeConstraint::Numeric;
        assert!(!c.satisfies(&DataType::Text));
        assert!(!c.satisfies(&DataType::Boolean));
        assert!(!c.satisfies(&DataType::Date));
    }

    #[test]
    fn any_constraint_accepts_everything() {
        let c = TypeConstraint::Any;
        assert!(c.satisfies(&DataType::Integer));
        assert!(c.satisfies(&DataType::Text));
        assert!(c.satisfies(&DataType::Boolean));
    }

    #[test]
    fn concrete_constraint_is_exact() {
        let c = TypeConstraint::Concrete(DataType::Integer);
        assert!(c.satisfies(&DataType::Integer));
        assert!(!c.satisfies(&DataType::BigInt));
        assert!(!c.satisfies(&DataType::Text));
    }

    #[test]
    fn parses_expr_with_whitespace() {
        assert_eq!(
            parse_smelt_type("Expr< Integer >"),
            Ok(SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)))
        );
    }

    #[test]
    fn malformed_missing_angle_brackets() {
        assert!(matches!(
            parse_smelt_type("Expr"),
            Err(SmeltTypeParseError::Malformed { .. })
        ));
    }

    #[test]
    fn malformed_empty_inner() {
        assert!(matches!(
            parse_smelt_type("Expr<>"),
            Err(SmeltTypeParseError::Malformed { .. })
        ));
    }

    #[test]
    fn unknown_inner_type() {
        match parse_smelt_type("Expr<FooBar>") {
            Err(SmeltTypeParseError::UnknownInner { inner, .. }) => {
                assert_eq!(inner, "FooBar");
            }
            other => panic!("expected UnknownInner, got {other:?}"),
        }
    }

    // === FunctionSig / ParamSpec wiring ===

    #[test]
    fn function_sig_exposes_parsed_param_types() {
        let (file, text) = parse_file(
            "smelt.define f(x: Expr<Integer>, y: Expr<Numeric>) -> Expr<Double> AS (x + y)",
        );
        let sigs = extract_function_signatures(&file, &text);
        assert_eq!(sigs.len(), 1);

        let sig = &sigs[0];
        assert_eq!(
            sig.params[0].type_ref.as_ref().unwrap().as_ref().unwrap(),
            &SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))
        );
        assert_eq!(
            sig.params[1].type_ref.as_ref().unwrap().as_ref().unwrap(),
            &SmeltType::Expr(TypeConstraint::Numeric)
        );
        assert_eq!(
            sig.return_type.as_ref().unwrap().as_ref().unwrap(),
            &SmeltType::Expr(TypeConstraint::Concrete(DataType::Double))
        );

        // Ranges populated on annotated params and return.
        assert!(sig.params[0].type_ref_range.is_some());
        assert!(sig.params[1].type_ref_range.is_some());
        assert!(sig.return_type_range.is_some());
    }

    #[test]
    fn function_sig_surfaces_bad_annotation_as_error() {
        // `TableExpr<T>` should be parsed into an `Err(UnsupportedSort)` so
        // higher layers can emit a diagnostic. Until the Phase 6 unified
        // harness arrives this is the targeted unit test called out in the
        // plan.
        let (file, text) = parse_file("smelt.define bad(x: TableExpr<T>) AS (x)");
        let sigs = extract_function_signatures(&file, &text);
        assert_eq!(sigs.len(), 1);

        let param = &sigs[0].params[0];
        let err = param
            .type_ref
            .as_ref()
            .expect("annotation present")
            .as_ref()
            .expect_err("should be an error");
        match err {
            SmeltTypeParseError::UnsupportedSort { sort, .. } => {
                assert_eq!(sort, "TableExpr");
            }
            other => panic!("expected UnsupportedSort, got {other:?}"),
        }
        assert!(param.type_ref_range.is_some());
    }

    #[test]
    fn function_sig_surfaces_bad_return_annotation() {
        let (file, text) = parse_file("smelt.define bad(x: Expr<Integer>) -> TableExpr<T> AS (x)");
        let sigs = extract_function_signatures(&file, &text);
        assert_eq!(sigs.len(), 1);

        let err = sigs[0]
            .return_type
            .as_ref()
            .expect("annotation present")
            .as_ref()
            .expect_err("should be an error");
        assert!(matches!(err, SmeltTypeParseError::UnsupportedSort { .. }));
        assert!(sigs[0].return_type_range.is_some());
    }

    // === Phase 7 TDD tests — Ordered constraint + registry skeleton ===

    #[test]
    fn ordered_members_match_decision_13() {
        // §16 #13: Numeric ∪ {Text family, temporal family, Boolean, Interval,
        // Blob}. This test enumerates every member exhaustively.
        let c = TypeConstraint::Ordered;

        // Numeric members (also covered by numeric_is_subset_of_ordered, but
        // the research text explicitly enumerates them here).
        assert!(c.satisfies(&DataType::SmallInt));
        assert!(c.satisfies(&DataType::Integer));
        assert!(c.satisfies(&DataType::BigInt));
        assert!(c.satisfies(&DataType::Float));
        assert!(c.satisfies(&DataType::Double));
        assert!(c.satisfies(&DataType::Decimal {
            precision: 10,
            scale: 2,
        }));

        // String family.
        assert!(c.satisfies(&DataType::Text));
        assert!(c.satisfies(&DataType::Varchar { max_length: None }));
        assert!(c.satisfies(&DataType::Varchar {
            max_length: Some(10)
        }));
        assert!(c.satisfies(&DataType::Char { length: 1 }));

        // Temporal family, including both Timestamp tz variants, plus
        // Interval.
        assert!(c.satisfies(&DataType::Date));
        assert!(c.satisfies(&DataType::Time));
        assert!(c.satisfies(&DataType::Timestamp {
            with_timezone: false
        }));
        assert!(c.satisfies(&DataType::Timestamp {
            with_timezone: true
        }));
        assert!(c.satisfies(&DataType::Interval));

        // Remaining singletons.
        assert!(c.satisfies(&DataType::Boolean));
        // "Binary" in §16 #13 is spelt Blob here.
        assert!(c.satisfies(&DataType::Blob));
    }

    #[test]
    fn ordered_excludes_composites() {
        let c = TypeConstraint::Ordered;
        assert!(!c.satisfies(&DataType::Array(Box::new(DataType::Integer))));
        assert!(!c.satisfies(&DataType::Struct(vec![(
            "a".to_string(),
            DataType::Integer,
        )])));
        assert!(!c.satisfies(&DataType::Map(
            Box::new(DataType::Text),
            Box::new(DataType::Integer),
        )));
        // Null and Unknown are explicitly not Ordered members.
        assert!(!c.satisfies(&DataType::Null));
        assert!(!c.satisfies(&DataType::Unknown));
    }

    #[test]
    fn numeric_is_subset_of_ordered() {
        // Every type the Numeric constraint accepts must also satisfy the
        // Ordered constraint (§16 #13: Numeric ⊂ Ordered).
        let numerics = [
            DataType::SmallInt,
            DataType::Integer,
            DataType::BigInt,
            DataType::Float,
            DataType::Double,
            DataType::Decimal {
                precision: 10,
                scale: 2,
            },
        ];
        for dt in &numerics {
            assert!(
                TypeConstraint::Numeric.satisfies(dt),
                "expected Numeric to accept {dt:?}"
            );
            assert!(
                TypeConstraint::Ordered.satisfies(dt),
                "expected Ordered to accept numeric {dt:?}"
            );
        }
    }

    #[test]
    fn registry_lookup_by_name() {
        let lower = BuiltinRegistry::resolve("LOWER").expect("LOWER present");
        assert_eq!(lower.name, "LOWER");
        assert_eq!(lower.params, vec![TypeConstraint::Concrete(DataType::Text)]);
        assert_eq!(lower.return_type, TypeConstraint::Concrete(DataType::Text));

        let upper = BuiltinRegistry::resolve("UPPER").expect("UPPER present");
        assert_eq!(upper.params, vec![TypeConstraint::Concrete(DataType::Text)]);
        assert_eq!(upper.return_type, TypeConstraint::Concrete(DataType::Text));

        let length = BuiltinRegistry::resolve("LENGTH").expect("LENGTH present");
        assert_eq!(
            length.params,
            vec![TypeConstraint::Concrete(DataType::Text)]
        );
        assert_eq!(
            length.return_type,
            TypeConstraint::Concrete(DataType::Integer)
        );

        let abs = BuiltinRegistry::resolve("ABS").expect("ABS present");
        assert_eq!(abs.params, vec![TypeConstraint::Concrete(DataType::Double)]);
        assert_eq!(abs.return_type, TypeConstraint::Concrete(DataType::Double));
    }

    #[test]
    fn registry_lookup_case_insensitive() {
        let canonical = BuiltinRegistry::resolve("LOWER").expect("LOWER present");
        let lowercase = BuiltinRegistry::resolve("lower").expect("lower present");
        let titlecase = BuiltinRegistry::resolve("Lower").expect("Lower present");
        let mixed = BuiltinRegistry::resolve("LoWeR").expect("LoWeR present");

        // All four lookups must resolve to the same `&'static Signature` —
        // ASCII case folding happens at the lookup boundary, not by inserting
        // multiple entries.
        assert!(std::ptr::eq(canonical, lowercase));
        assert!(std::ptr::eq(canonical, titlecase));
        assert!(std::ptr::eq(canonical, mixed));
    }
}

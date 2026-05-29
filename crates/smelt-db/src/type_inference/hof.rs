//! Higher-order functions: HofKind, reducer registry, list literals, pipe, splice, define-name shadowing, HOF position diagnostics.

#![allow(unused_imports)]
use rowan::TextRange;
use smelt_parser::ast::{
    BinaryExpr, CaseExpr, CastExpr, Cte, Expr, ExtractExpr, FunctionCall, RowConstructor,
    SelectStmt, SmeltAsStructCall, SmeltPathCall, StructLiteral, Subquery,
};
use smelt_types::signatures::{
    kind_ceiling, unify_call_with_expected, BuiltinRegistry, ExprKind, FunctionSig, RecordRegistry,
    SmeltType, TypeConstraint,
};
use smelt_types::{parse_type, DataType, SqlFunction, TypedColumn};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::type_context::TypeContext;
#[allow(unused_imports)]
use super::*;

// ─── Phase B (meta-language): HOF inference + reducer registry + pipe ────────

/// The three built-in higher-order functions (Phase B meta-language).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HofKind {
    /// `map(xs: List<T>, f: Lambda<T, U>) -> List<U>`
    Map,
    /// `filter(xs: List<T>, p: Lambda<T, Boolean>) -> List<T>`
    Filter,
    /// `reduce(xs: List<T>, r)` where `r` is a bare reducer identifier.
    Reduce,
}

impl HofKind {
    /// Parse a HOF name into a [`HofKind`]. Returns `None` for non-HOF names.
    pub fn from_name(name: &str) -> Option<HofKind> {
        match name {
            "map" => Some(HofKind::Map),
            "filter" => Some(HofKind::Filter),
            "reduce" => Some(HofKind::Reduce),
            _ => None,
        }
    }
}

/// Sentinel produced by HOF / reducer inference (Phase B).
///
/// Phase 2 records these sentinels but does NOT emit diagnostic codes —
/// that is Phase 3's job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HofInferSentinel {
    /// The lambda body's synthesised type does not match the HOF's required
    /// result shape (e.g. `filter` predicate not `Boolean`).
    /// Corresponds to `LambdaResultTypeMismatch`.
    LambdaResultTypeMismatch {
        expected: SmeltType,
        found: SmeltType,
    },
    /// The second argument to `reduce` is not a recognised reducer name.
    /// Corresponds to `HofExpectsReducer`.
    HofExpectsReducer,
    /// The second argument to `map` or `filter` is not a lambda node.
    /// Corresponds to `HofExpectsLambda`.
    HofExpectsLambda,
    /// The input element type does not satisfy the reducer's declared input
    /// constraint. Corresponds to `ReducerInputTypeMismatch`.
    ReducerInputTypeMismatch {
        reducer_name: String,
        expected_constraint: String,
        found: SmeltType,
    },
    /// An empty list was passed to a reducer with no declared identity.
    /// Corresponds to `ReducerEmptyNoIdentity`.
    ReducerEmptyNoIdentity { reducer_name: String },
    /// The first argument to the HOF did not infer to a `List<T>`.
    InputNotList { found: SmeltType },
}

/// Result of HOF / reducer inference (Phase B).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HofInferResult {
    /// The inferred [`SmeltType`] for the HOF call.
    pub inferred: SmeltType,
    /// Optional pending diagnostic sentinel. `None` on the happy path.
    pub sentinel: Option<HofInferSentinel>,
}

/// Spec for a single entry in the closed reducer registry (Phase B).
///
/// Pure data — no Salsa dependency.
pub struct ReducerSpec {
    /// The reducer's source name (e.g. `"and_all"`).
    pub name: &'static str,
    /// The input constraint for each element: `Boolean`, `Numeric`, `Text`,
    /// `Any` (for `comma_sep`), or `TableExpr` (for `union_all`/`intersect_all`).
    /// `None` means the element type is unconstrained (accepts any `Expr<T>`).
    pub input_constraint: ReducerInputConstraint,
    /// The output [`SmeltType`] of the reducer (after a non-empty evaluation).
    pub output_sort: ReducerOutputSort,
    /// Empty-list behaviour: `Some(identity)` means the reducer has a known
    /// identity element; `None` means `ReducerEmptyNoIdentity`.
    pub empty_identity: EmptyIdentity,
}

/// Input-element constraint for a reducer entry (Phase B).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReducerInputConstraint {
    /// Any `Expr<T>` is accepted (used by `comma_sep`).
    AnyExpr,
    /// Only `Expr<Boolean>` is accepted.
    Boolean,
    /// Any `Expr<Numeric>` is accepted (satisfies `TypeConstraint::Numeric`).
    Numeric,
    /// Only `Expr<Text>` is accepted.
    Text,
    /// Only `TableExpr` is accepted.
    TableExpr,
}

impl ReducerInputConstraint {
    /// Check whether a [`SmeltType`] satisfies this input constraint.
    pub fn is_satisfied_by(&self, ty: &SmeltType) -> bool {
        match (self, ty) {
            (ReducerInputConstraint::AnyExpr, SmeltType::Expr(_)) => true,
            (ReducerInputConstraint::Boolean, SmeltType::Expr(tc)) => {
                matches!(tc, smelt_types::signatures::TypeConstraint::Concrete(dt) if *dt == DataType::Boolean)
            }
            (ReducerInputConstraint::Numeric, SmeltType::Expr(tc)) => {
                smelt_types::signatures::TypeConstraint::Numeric.satisfies(&match tc {
                    smelt_types::signatures::TypeConstraint::Concrete(dt) => dt.clone(),
                    _ => return false,
                })
            }
            (ReducerInputConstraint::Text, SmeltType::Expr(tc)) => {
                matches!(tc,
                    smelt_types::signatures::TypeConstraint::Concrete(dt)
                    if matches!(dt, DataType::Text | DataType::Varchar { .. } | DataType::Char { .. })
                )
            }
            (ReducerInputConstraint::TableExpr, SmeltType::TableExpr(_)) => true,
            // ModelRef <: TableExpr and SourceRef <: TableExpr (Phase D subtyping rule).
            // A `ModelRef` or `SourceRef` element satisfies the `TableExpr` reducer
            // constraint because of the fragment-sort subtyping rule in `signatures.rs`.
            (ReducerInputConstraint::TableExpr, SmeltType::ModelRef) => true,
            (ReducerInputConstraint::TableExpr, SmeltType::SourceRef) => true,
            _ => false,
        }
    }
}

/// Output sort for a reducer (Phase B).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReducerOutputSort {
    /// Output is `Expr<Boolean>`.
    Boolean,
    /// Output is `Expr<T>` where `T` is the LUB-promoted element type of the input list
    /// (used by `plus_chain` and `concat`).
    SameAsElementType,
    /// Output is `SelectItems<Scalar>` (used by `comma_sep`).
    SelectItemsScalar,
    /// Output is `TableExpr` (used by `union_all`, `intersect_all`).
    TableExpr,
}

/// Empty-list identity rule for a reducer (Phase B).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmptyIdentity {
    /// The reducer has an identity element: the output sort with a known value.
    /// The bool/text/numeric variant matches the output sort.
    Boolean,
    /// The reducer has a numeric identity element (0 cast to LUB element type).
    Numeric,
    /// The reducer has a text identity element (empty string `''`).
    Text,
    /// Empty list produces `SelectItems<Scalar>` with no sentinel (used by `comma_sep`).
    /// An empty comma-separated list elides at the splice point without error.
    EmptySelectItems,
    /// The reducer has no identity — empty list is an error (`ReducerEmptyNoIdentity`).
    None,
}

/// The closed reducer registry — seven entries (Phase B spec).
pub static REDUCER_REGISTRY: &[ReducerSpec] = &[
    ReducerSpec {
        name: "comma_sep",
        input_constraint: ReducerInputConstraint::AnyExpr,
        output_sort: ReducerOutputSort::SelectItemsScalar,
        empty_identity: EmptyIdentity::EmptySelectItems, // empty list elides at splice (see spec)
    },
    ReducerSpec {
        name: "and_all",
        input_constraint: ReducerInputConstraint::Boolean,
        output_sort: ReducerOutputSort::Boolean,
        empty_identity: EmptyIdentity::Boolean, // TRUE
    },
    ReducerSpec {
        name: "or_any",
        input_constraint: ReducerInputConstraint::Boolean,
        output_sort: ReducerOutputSort::Boolean,
        empty_identity: EmptyIdentity::Boolean, // FALSE
    },
    ReducerSpec {
        name: "union_all",
        input_constraint: ReducerInputConstraint::TableExpr,
        output_sort: ReducerOutputSort::TableExpr,
        empty_identity: EmptyIdentity::None,
    },
    ReducerSpec {
        name: "intersect_all",
        input_constraint: ReducerInputConstraint::TableExpr,
        output_sort: ReducerOutputSort::TableExpr,
        empty_identity: EmptyIdentity::None,
    },
    ReducerSpec {
        name: "plus_chain",
        input_constraint: ReducerInputConstraint::Numeric,
        output_sort: ReducerOutputSort::SameAsElementType,
        empty_identity: EmptyIdentity::Numeric, // 0-cast-to-LUB
    },
    ReducerSpec {
        name: "concat",
        input_constraint: ReducerInputConstraint::Text,
        output_sort: ReducerOutputSort::SameAsElementType,
        empty_identity: EmptyIdentity::Text, // empty string ''
    },
];

/// Look up a reducer by name in the closed registry.
///
/// Returns `None` for unknown names. Pure — no Salsa dependency.
pub fn lookup_reducer(name: &str) -> Option<&'static ReducerSpec> {
    REDUCER_REGISTRY.iter().find(|r| r.name == name)
}

// ─── Phase F: Parameterised reducer registry ─────────────────────────────────

/// Spec for a single parameter of a parameterised reducer.
pub struct ParameterisedReducerParam {
    /// The parameter name (e.g. `"sep"` for `concat_with`).
    pub name: &'static str,
    /// A function returning the required type for this parameter.
    pub ty: fn() -> SmeltType,
}

/// Spec for a parameterised reducer (Phase F).
///
/// Parameterised reducers are called as `IDENT(args...)` in the second-argument
/// position of a `reduce(xs, ...)` call. The arguments must be compile-time
/// resolvable (literals, `smelt.config.var(...)` results).
pub struct ParameterisedReducerSpec {
    /// The reducer's source name (e.g. `"concat_with"`).
    pub name: &'static str,
    /// The required parameters in positional order.
    pub params: &'static [ParameterisedReducerParam],
    /// The input element type constraint.
    pub input_constraint: ReducerInputConstraint,
    /// The output type of the reducer.
    pub output_type: fn() -> SmeltType,
    /// Empty-list identity (same semantics as `ReducerSpec::empty_identity`).
    pub empty_identity: EmptyIdentity,
}

fn text_smelt_type() -> SmeltType {
    SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
        DataType::Text,
    ))
}

/// The closed parameterised reducer registry (Phase F v1).
///
/// Only `concat_with` is registered. Adding entries requires a spec edit
/// and a single-line addition here.
pub static PARAMETERISED_REDUCER_REGISTRY: &[ParameterisedReducerSpec] =
    &[ParameterisedReducerSpec {
        name: "concat_with",
        params: &[ParameterisedReducerParam {
            name: "sep",
            ty: text_smelt_type,
        }],
        input_constraint: ReducerInputConstraint::Text,
        output_type: text_smelt_type,
        empty_identity: EmptyIdentity::Text, // identity is '' (empty string)
    }];

/// Look up a parameterised reducer by name.
///
/// Returns `None` for unknown names. Pure — no Salsa dependency.
pub fn lookup_parameterised_reducer(name: &str) -> Option<&'static ParameterisedReducerSpec> {
    PARAMETERISED_REDUCER_REGISTRY
        .iter()
        .find(|r| r.name == name)
}

/// Pending diagnostic sentinel from `infer_parameterised_reducer_call`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParameterisedReducerSentinel {
    /// Wrong number of positional arguments.
    ArityMismatch {
        reducer: String,
        expected: usize,
        actual: usize,
    },
    /// An argument is a named argument (e.g. `sep => ' OR '`).
    NamedArgument,
    /// An argument's type does not match the expected parameter type.
    ArgTypeMismatch {
        reducer: String,
        param: String,
        expected: SmeltType,
        found: SmeltType,
    },
    /// An argument is not a compile-time resolvable value.
    ArgNotCompileTime {
        reducer: String,
        param: String,
        found: SmeltType,
    },
}

/// Result of `infer_parameterised_reducer_call`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterisedReducerResult {
    /// The inferred output type of the reducer (or `Unknown` on error).
    pub output_type: SmeltType,
    /// Optional diagnostic sentinel. `None` on the happy path.
    pub sentinel: Option<ParameterisedReducerSentinel>,
}

/// Infer the type of a `REDUCER_CALL` CST node (Phase F).
///
/// Called from the second-argument position of a `reduce(xs, REDUCER_CALL)` call.
/// Validates:
/// - The reducer name is in `PARAMETERISED_REDUCER_REGISTRY`; else returns an error
///   sentinel (caller translates to `HofExpectsReducer`).
/// - Positional argument count matches the registry entry.
/// - No named arguments (e.g. `sep => ' OR '`).
/// - Each argument synthesises a compile-time-resolvable type matching the
///   declared parameter type.
///
/// Pure function — no Salsa dependency.
pub fn infer_parameterised_reducer_call(
    call: &smelt_parser::ast::ReducerCall,
    ctx: &TypeContext,
) -> ParameterisedReducerResult {
    // 1. Look up reducer name.
    let name = call.name().unwrap_or_default();
    let Some(spec) = lookup_parameterised_reducer(&name) else {
        // Unknown reducer name — caller emits HofExpectsReducer.
        return ParameterisedReducerResult {
            output_type: SmeltType::Unknown,
            sentinel: None, // caller handles unknown name
        };
    };

    // 2. Extract arguments from the ARG_LIST child node.
    let arg_list_node = call.args();
    let arg_exprs: Vec<smelt_parser::ast::Expr> = if let Some(arg_list) = arg_list_node {
        use smelt_parser::SyntaxKind::{EXPRESSION, NAMED_PARAM};
        // Check for named arguments first (named params look like `name => value`).
        let has_named = arg_list.children().any(|n| n.kind() == NAMED_PARAM);
        if has_named {
            return ParameterisedReducerResult {
                output_type: SmeltType::Unknown,
                sentinel: Some(ParameterisedReducerSentinel::NamedArgument),
            };
        }
        // Collect EXPRESSION children as positional args.
        arg_list
            .children()
            .filter(|n| n.kind() == EXPRESSION)
            .filter_map(smelt_parser::ast::Expr::cast)
            .collect()
    } else {
        Vec::new()
    };

    // 3. Arity check.
    let expected_arity = spec.params.len();
    let actual_arity = arg_exprs.len();
    if actual_arity != expected_arity {
        return ParameterisedReducerResult {
            output_type: SmeltType::Unknown,
            sentinel: Some(ParameterisedReducerSentinel::ArityMismatch {
                reducer: name.clone(),
                expected: expected_arity,
                actual: actual_arity,
            }),
        };
    }

    // 4. Validate each argument.
    for (i, (param, arg_expr)) in spec.params.iter().zip(arg_exprs.iter()).enumerate() {
        // 4a. Check compile-time resolvability.
        let is_compile_time = is_compile_time_reducer_arg(arg_expr, ctx);

        // Synthesise the argument type.
        let arg_ty = synthesise_arg_smelt_type(arg_expr, ctx);

        if !is_compile_time {
            return ParameterisedReducerResult {
                output_type: SmeltType::Unknown,
                sentinel: Some(ParameterisedReducerSentinel::ArgNotCompileTime {
                    reducer: name.clone(),
                    param: param.name.to_string(),
                    found: arg_ty,
                }),
            };
        }

        // 4b. Check type compatibility.
        let expected_ty = (param.ty)();
        if !arg_type_matches(&arg_ty, &expected_ty) {
            return ParameterisedReducerResult {
                output_type: SmeltType::Unknown,
                sentinel: Some(ParameterisedReducerSentinel::ArgTypeMismatch {
                    reducer: name.clone(),
                    param: param.name.to_string(),
                    expected: expected_ty,
                    found: arg_ty,
                }),
            };
        }

        let _ = i; // suppress unused warning
    }

    // 5. Return output type.
    ParameterisedReducerResult {
        output_type: (spec.output_type)(),
        sentinel: None,
    }
}

/// Determine whether an expression is compile-time resolvable (Phase F).
///
/// Compile-time resolvable means:
/// - A string/number/boolean literal.
/// - A `smelt.config.var(...)` call (always returns compile-time Text).
///
/// Runtime expressions (function calls like `UPPER('x')`, column references,
/// etc.) are NOT compile-time.
///
/// Pure function — no Salsa dependency.
fn is_compile_time_reducer_arg(expr: &smelt_parser::ast::Expr, _ctx: &TypeContext) -> bool {
    let text = expr.text().trim().to_string();

    // String literal: starts and ends with quote.
    if (text.starts_with('\'') && text.ends_with('\''))
        || (text.starts_with('"') && text.ends_with('"'))
    {
        return true;
    }

    // Numeric literal: all digits (possibly with decimal point, minus sign, etc.)
    if text
        .trim_start_matches('-')
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.')
        && !text.is_empty()
    {
        return true;
    }

    // Boolean literals.
    let text_lc = text.to_lowercase();
    if text_lc == "true" || text_lc == "false" {
        return true;
    }

    // smelt.config.var(...) — always compile-time.
    if text.to_lowercase().contains("smelt.config.var") {
        return true;
    }

    false
}

/// Synthesise the SmeltType for an expression in reducer argument position.
fn synthesise_arg_smelt_type(expr: &smelt_parser::ast::Expr, ctx: &TypeContext) -> SmeltType {
    use smelt_types::signatures::TypeConstraint;
    match infer_expression_type(expr, ctx) {
        Some(tc) => SmeltType::Expr(TypeConstraint::Concrete(tc.data_type)),
        None => SmeltType::Unknown,
    }
}

/// Check whether an actual argument type matches the declared parameter type.
fn arg_type_matches(actual: &SmeltType, expected: &SmeltType) -> bool {
    use smelt_types::signatures::TypeConstraint;
    use smelt_types::DataType;

    match (actual, expected) {
        // Exact match.
        (a, e) if a == e => true,
        // Text variants: Text, Varchar, Char all satisfy Expr<Text>.
        (
            SmeltType::Expr(TypeConstraint::Concrete(
                DataType::Text | DataType::Varchar { .. } | DataType::Char { .. },
            )),
            SmeltType::Expr(TypeConstraint::Concrete(
                DataType::Text | DataType::Varchar { .. } | DataType::Char { .. },
            )),
        ) => true,
        // Unknown actual: don't reject (already an error).
        (SmeltType::Unknown, _) => true,
        _ => false,
    }
}

/// Infer the output [`SmeltType`] for a `reduce(xs, reducer_name)` call
/// where `xs` infers to `List<elem_ty>` (Phase B).
///
/// Returns a [`HofInferResult`] with:
/// - On success: the reducer's declared output type, or the element type for
///   `SameAsElementType` reducers.
/// - On `ReducerInputTypeMismatch`: the input element does not satisfy the
///   reducer's constraint.
/// - On `ReducerEmptyNoIdentity`: empty list passed to a no-identity reducer.
///
/// Pure function — no Salsa dependency.
pub fn infer_reduce_call(
    list_elem_ty: &SmeltType,
    is_empty_list: bool,
    reducer_name: &str,
    _expected: Option<&SmeltType>,
) -> HofInferResult {
    let Some(spec) = lookup_reducer(reducer_name) else {
        return HofInferResult {
            inferred: SmeltType::Unknown,
            sentinel: Some(HofInferSentinel::HofExpectsReducer),
        };
    };

    // Empty-list path
    if is_empty_list {
        return match spec.empty_identity {
            EmptyIdentity::Boolean => HofInferResult {
                inferred: SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                    DataType::Boolean,
                )),
                sentinel: None,
            },
            EmptyIdentity::Numeric => HofInferResult {
                // Identity is 0-cast-to-element-type; use Unknown for empty.
                inferred: SmeltType::Expr(smelt_types::signatures::TypeConstraint::Numeric),
                sentinel: None,
            },
            EmptyIdentity::Text => HofInferResult {
                inferred: SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                    DataType::Text,
                )),
                sentinel: None,
            },
            EmptyIdentity::EmptySelectItems => HofInferResult {
                inferred: SmeltType::SelectItems {
                    kind: smelt_types::signatures::ExprKind::Scalar,
                    context: None,
                },
                sentinel: None,
            },
            EmptyIdentity::None => HofInferResult {
                inferred: SmeltType::Unknown,
                sentinel: Some(HofInferSentinel::ReducerEmptyNoIdentity {
                    reducer_name: reducer_name.to_string(),
                }),
            },
        };
    }

    // Non-empty: validate input element type against the reducer's constraint.
    if !spec.input_constraint.is_satisfied_by(list_elem_ty) {
        return HofInferResult {
            inferred: SmeltType::Unknown,
            sentinel: Some(HofInferSentinel::ReducerInputTypeMismatch {
                reducer_name: reducer_name.to_string(),
                expected_constraint: format!("{:?}", spec.input_constraint),
                found: list_elem_ty.clone(),
            }),
        };
    }

    // Derive output sort.
    let output = match &spec.output_sort {
        ReducerOutputSort::Boolean => SmeltType::Expr(
            smelt_types::signatures::TypeConstraint::Concrete(DataType::Boolean),
        ),
        ReducerOutputSort::SameAsElementType => list_elem_ty.clone(),
        ReducerOutputSort::SelectItemsScalar => SmeltType::SelectItems {
            kind: smelt_types::signatures::ExprKind::Scalar,
            context: None,
        },
        ReducerOutputSort::TableExpr => SmeltType::TableExpr(None),
    };

    HofInferResult {
        inferred: output,
        sentinel: None,
    }
}

/// Infer the output [`SmeltType`] for a HOF call (`map`, `filter`, or `reduce`)
/// given the pre-inferred list type and the lambda/reducer second argument (Phase B).
///
/// - For `map`: bind lambda parameter to `T`, synthesise body type `U`, return `List<U>`.
/// - For `filter`: bind lambda parameter to `T`, synthesise body type, require `Boolean`,
///   return `List<T>`.
/// - For `reduce`: delegate to [`infer_reduce_call`].
///
/// Pure function — no Salsa dependency.
pub fn infer_hof_call(
    hof: HofKind,
    list_ty: &SmeltType,
    second_arg: HofSecondArg<'_>,
    ctx: &TypeContext,
    expected: Option<&SmeltType>,
) -> HofInferResult {
    // Extract element type from List<T>.
    let elem_ty = match list_ty {
        SmeltType::List(inner) => (**inner).clone(),
        SmeltType::Unknown => SmeltType::Unknown,
        other => {
            return HofInferResult {
                inferred: SmeltType::Unknown,
                sentinel: Some(HofInferSentinel::InputNotList {
                    found: other.clone(),
                }),
            };
        }
    };

    // Check for empty list (List<Unknown> from empty literal `[]`).
    let is_empty = matches!(&elem_ty, SmeltType::Unknown);

    match hof {
        HofKind::Map | HofKind::Filter => {
            let lambda = match second_arg {
                HofSecondArg::Lambda(l) => l,
                HofSecondArg::ReducerName(_) | HofSecondArg::Other => {
                    return HofInferResult {
                        inferred: SmeltType::Unknown,
                        sentinel: Some(HofInferSentinel::HofExpectsLambda),
                    };
                }
            };
            let params = lambda.params();
            let param_name = params.first().cloned().unwrap_or_default();
            let body = match lambda.body() {
                Some(b) => b,
                None => {
                    return HofInferResult {
                        inferred: if hof == HofKind::Map {
                            SmeltType::List(Box::new(SmeltType::Unknown))
                        } else {
                            list_ty.clone()
                        },
                        sentinel: None,
                    };
                }
            };

            // Bind lambda parameter to elem_ty in a scoped context clone.
            let mut body_ctx = ctx.clone();
            body_ctx.add_lambda_param(
                &param_name,
                smelt_types::TypedColumn {
                    data_type: match &elem_ty {
                        SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(dt)) => {
                            dt.clone()
                        }
                        _ => DataType::Unknown,
                    },
                    nullable: true,
                },
            );

            // Synthesise body type.
            // When the expected output type for a Map is a meta-world type (e.g.
            // `ModelDef`), use smelt-level expression inference so that a
            // `ModelDef { … }` record literal in the lambda body is correctly
            // typed instead of falling through to the SQL expression inferencer.
            let elem_expected_ty: Option<SmeltType> = match (hof, &expected) {
                (HofKind::Map, Some(SmeltType::List(inner))) => Some(*inner.clone()),
                _ => None,
            };
            let body_ty = if let Some(ref elem_exp) = elem_expected_ty {
                super::multi_model::infer_expression_smelt_type(&body, &body_ctx, Some(elem_exp))
            } else {
                infer_expression_type(&body, &body_ctx)
                    .map(|tc| {
                        SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                            tc.data_type,
                        ))
                    })
                    .unwrap_or(SmeltType::Unknown)
            };

            if hof == HofKind::Filter {
                // Filter predicate body must be Boolean.
                let expected_bool = SmeltType::Expr(
                    smelt_types::signatures::TypeConstraint::Concrete(DataType::Boolean),
                );
                if body_ty != expected_bool && !matches!(body_ty, SmeltType::Unknown) {
                    return HofInferResult {
                        inferred: list_ty.clone(),
                        sentinel: Some(HofInferSentinel::LambdaResultTypeMismatch {
                            expected: expected_bool,
                            found: body_ty,
                        }),
                    };
                }
                HofInferResult {
                    inferred: list_ty.clone(),
                    sentinel: None,
                }
            } else {
                // Map: result is List<body_ty>.
                HofInferResult {
                    inferred: SmeltType::List(Box::new(body_ty)),
                    sentinel: None,
                }
            }
        }
        HofKind::Reduce => {
            let reducer_name = match second_arg {
                HofSecondArg::ReducerName(name) => name,
                HofSecondArg::Lambda(_) | HofSecondArg::Other => {
                    return HofInferResult {
                        inferred: SmeltType::Unknown,
                        sentinel: Some(HofInferSentinel::HofExpectsReducer),
                    };
                }
            };
            infer_reduce_call(&elem_ty, is_empty, reducer_name, expected)
        }
    }
}

/// The second argument to a HOF call (Phase B).
///
/// `Lambda` carries the AST lambda node (owned — Rowan handles are cheap refcounted
/// clones); `ReducerName` carries the bare identifier text; `Other` represents any
/// other node shape (triggers a sentinel).
pub enum HofSecondArg<'a> {
    Lambda(smelt_parser::ast::Lambda),
    ReducerName(&'a str),
    Other,
}

/// Infer the output type for a HOF call given a [`FunctionCall`] AST node (Phase B).
///
/// This convenience wrapper extracts the HOF kind, the list argument, and the
/// second argument (lambda or reducer name) from the function call, then delegates
/// to [`infer_hof_call`].
///
/// Returns `None` when the function is not a recognised HOF.
pub fn infer_hof_call_from_function_call(
    call: &smelt_parser::ast::FunctionCall,
    ctx: &TypeContext,
) -> HofInferResult {
    infer_hof_call_from_function_call_with_expected(call, ctx, None)
}

/// Like [`infer_hof_call_from_function_call`] but with an optional expected type
/// (used by the empty-list + identity reducer tests).
pub fn infer_hof_call_from_function_call_with_expected(
    call: &smelt_parser::ast::FunctionCall,
    ctx: &TypeContext,
    expected: Option<&SmeltType>,
) -> HofInferResult {
    // `FunctionCall::name()` only returns IDENT-typed tokens; HOF names like
    // `filter` may be lexed as keyword tokens (FILTER_KW). Fall back to the
    // first token's text for keyword-as-function-name cases.
    let name = call.name().unwrap_or_else(|| {
        // Extract the first non-trivia token's text, normalised to lowercase.
        call.syntax()
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| !t.kind().is_trivia())
            .map(|t| t.text().to_lowercase())
            .unwrap_or_default()
    });
    let name_lc = name.to_lowercase();
    let Some(hof) = HofKind::from_name(&name_lc) else {
        return HofInferResult {
            inferred: SmeltType::Unknown,
            sentinel: None,
        };
    };

    let args = call.arguments();
    if args.is_empty() {
        return HofInferResult {
            inferred: SmeltType::Unknown,
            sentinel: None,
        };
    }

    // Infer the list type from the first argument.
    let xs = &args[0];
    let list_ty = if let Some(arr) = xs.as_array_literal() {
        let elems: Vec<_> = arr.elements();
        let result = infer_list_literal(&elems, ctx, expected);
        result.inferred
    } else {
        // Non-literal first argument.
        //
        // Phase B: if the argument is a bare identifier (e.g. `xs`) that
        // names a function parameter declared as `List<T>`, the standard
        // `infer_expression_type` path collapses the type to `DataType::Unknown`
        // (because `List<T>` has no scalar `DataType` equivalent).  To preserve
        // the full `SmeltType::List(...)`, we first consult the
        // `function_param_smelt_types` map which stores the declared `SmeltType`
        // for params registered via `add_function_param_smelt_type`.
        let ident_name = xs.text().to_string();
        let ident_name = ident_name.trim();
        let is_bare_ident =
            !ident_name.is_empty() && ident_name.chars().all(|c| c.is_alphanumeric() || c == '_');

        if is_bare_ident {
            if let Some(smelt_ty) = ctx.lookup_function_param_smelt_type(ident_name) {
                smelt_ty.clone()
            } else {
                // Fall back to expression inference.
                infer_expression_type(xs, ctx)
                    .map(|tc| {
                        SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                            tc.data_type,
                        ))
                    })
                    .unwrap_or(SmeltType::Unknown)
            }
        } else {
            // Complex expression: expression inference only.
            infer_expression_type(xs, ctx)
                .map(|tc| {
                    SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                        tc.data_type,
                    ))
                })
                .unwrap_or(SmeltType::Unknown)
        }
    };

    // Extract second argument shape.
    if args.len() < 2 {
        return infer_hof_call(hof, &list_ty, HofSecondArg::Other, ctx, expected);
    }
    let arg2 = &args[1];

    // Check if second arg contains a LAMBDA node (may be wrapped in EXPRESSION).
    if let Some(lambda) = extract_lambda_from_expr(arg2) {
        return infer_hof_call(hof, &list_ty, HofSecondArg::Lambda(lambda), ctx, expected);
    }

    // Check if it's a bare identifier (reducer name).
    let text = arg2.text().trim().to_string();
    // A bare identifier: no spaces, no parens, no brackets.
    if text.chars().all(|c| c.is_alphanumeric() || c == '_') && !text.is_empty() {
        return infer_hof_call_with_reducer_name(hof, &list_ty, &text, ctx, expected);
    }

    infer_hof_call(hof, &list_ty, HofSecondArg::Other, ctx, expected)
}

/// Extract a [`Lambda`] from an [`Expr`], following the same pattern as
/// [`Expr::as_function_call`] and other `as_*` methods: check the node itself,
/// then check its direct children. This is needed because `arguments()` returns
/// `Expr` values whose inner node is an `EXPRESSION` that wraps the `LAMBDA`.
fn extract_lambda_from_expr(expr: &smelt_parser::ast::Expr) -> Option<smelt_parser::ast::Lambda> {
    smelt_parser::ast::Lambda::cast(expr.syntax().clone()).or_else(|| {
        expr.syntax()
            .children()
            .find_map(smelt_parser::ast::Lambda::cast)
    })
}

/// Extract a [`PipeExpr`] from an [`Expr`], following the same pattern.
pub fn extract_pipe_expr_from_expr(
    expr: &smelt_parser::ast::Expr,
) -> Option<smelt_parser::ast::PipeExpr> {
    smelt_parser::ast::PipeExpr::cast(expr.syntax().clone()).or_else(|| {
        expr.syntax()
            .children()
            .find_map(smelt_parser::ast::PipeExpr::cast)
    })
}

/// Inner helper for the reducer-name path (avoids lifetime issues with `HofSecondArg`).
fn infer_hof_call_with_reducer_name(
    hof: HofKind,
    list_ty: &SmeltType,
    reducer_name: &str,
    ctx: &TypeContext,
    expected: Option<&SmeltType>,
) -> HofInferResult {
    infer_hof_call(
        hof,
        list_ty,
        HofSecondArg::ReducerName(reducer_name),
        ctx,
        expected,
    )
}

/// Infer the output [`SmeltType`] for a pipe expression `LHS |> CALL(args...)` (Phase B).
///
/// Pipe desugaring: `LHS |> CALL(args...)` is equivalent to `CALL(LHS, args...)`.
/// This function constructs the equivalent direct call and infers its type.
///
/// Pure function — no Salsa dependency.
pub fn infer_pipe_expr(
    pipe: &smelt_parser::ast::PipeExpr,
    ctx: &TypeContext,
    expected: Option<&SmeltType>,
) -> HofInferResult {
    // Get LHS and RHS.
    let lhs = match pipe.lhs() {
        Some(l) => l,
        None => {
            return HofInferResult {
                inferred: SmeltType::Unknown,
                sentinel: None,
            }
        }
    };
    let rhs = match pipe.rhs() {
        Some(r) => r,
        None => {
            return HofInferResult {
                inferred: SmeltType::Unknown,
                sentinel: None,
            }
        }
    };

    // If the LHS itself is a pipe, recurse to infer its type first.
    let lhs_ty = if let Some(inner_pipe) = extract_pipe_expr_from_expr(&lhs) {
        let r = infer_pipe_expr(&inner_pipe, ctx, None);
        r.inferred
    } else {
        // Infer LHS as a list literal or expression.
        if let Some(arr) = lhs.as_array_literal() {
            let elems: Vec<_> = arr.elements();
            infer_list_literal(&elems, ctx, None).inferred
        } else {
            infer_expression_type(&lhs, ctx)
                .map(|tc| {
                    SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                        tc.data_type,
                    ))
                })
                .unwrap_or(SmeltType::Unknown)
        }
    };

    // RHS must be a function call — if so, extract call name and synthesise.
    if let Some(call) = rhs.as_function_call() {
        // `call.name()` only finds IDENT tokens; keyword-based names like
        // `filter` (lexed as FILTER_KW) need a fallback to the first token.
        let name = call.name().unwrap_or_else(|| {
            call.syntax()
                .children_with_tokens()
                .filter_map(|e| e.into_token())
                .find(|t| !t.kind().is_trivia())
                .map(|t| t.text().to_lowercase())
                .unwrap_or_default()
        });
        let name_lc = name.to_lowercase();
        if let Some(hof) = HofKind::from_name(&name_lc) {
            // Build the effective argument list: LHS + RHS args.
            let rhs_args = call.arguments();

            // We reconstruct the inference by using the lhs_ty directly.
            if rhs_args.is_empty() {
                return HofInferResult {
                    inferred: SmeltType::Unknown,
                    sentinel: None,
                };
            }
            let second_arg = &rhs_args[0];

            // Check if second arg contains a LAMBDA node (may be wrapped in EXPRESSION).
            if let Some(lambda) = extract_lambda_from_expr(second_arg) {
                return infer_hof_call(hof, &lhs_ty, HofSecondArg::Lambda(lambda), ctx, expected);
            }
            // Check if it's a bare reducer name.
            let text = second_arg.text().trim().to_string();
            if text.chars().all(|c| c.is_alphanumeric() || c == '_') && !text.is_empty() {
                return infer_hof_call_with_reducer_name(hof, &lhs_ty, &text, ctx, expected);
            }
            return infer_hof_call(hof, &lhs_ty, HofSecondArg::Other, ctx, expected);
        }
    }

    // Non-HOF RHS or non-call RHS — Phase 3 emits PipeRhsNotCall diagnostic.
    // For now return Unknown without sentinel.
    HofInferResult {
        inferred: SmeltType::Unknown,
        sentinel: None,
    }
}

// ─── Phase A (meta-language): list-literal type inference ───────────────────

/// Pending diagnostic sentinel produced by [`infer_list_literal`].
///
/// Phase 2 records these sentinels but does NOT emit diagnostic codes —
/// that is Phase 3's job. The sentinel carries enough information for Phase 3
/// to produce a rich diagnostic without re-inferring.
///
/// Pure data — no Salsa dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListInferSentinel {
    /// An empty list literal (`[]`) was found in a position where no target
    /// sort context was supplied. Corresponds to `MetaListEmptyTypeUnknown`.
    EmptyTypeUnknown,
    /// Two or more elements in the list literal have incompatible types that
    /// cannot be unified by the numeric promotion chain, or elements whose
    /// sorts differ (e.g. a scalar `Expr<T>` mixed with a nested `List<T>`).
    /// Corresponds to `MetaListHeterogeneous`.
    ///
    /// Fields carry [`SmeltType`] (not bare [`DataType`]) so that cross-sort
    /// heterogeneity (scalar vs. nested list) can be represented faithfully.
    /// Phase 3 renders these for the user; it handles both `Expr<…>` and
    /// `List<…>` variants.
    Heterogeneous {
        /// The first element sort seen before the incompatibility.
        first: SmeltType,
        /// The element sort that was incompatible with `first`.
        incompatible: SmeltType,
    },
}

/// Result of [`infer_list_literal`].
///
/// Pure data — no Salsa dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListLiteralInferResult {
    /// The inferred [`SmeltType`] for this list literal.
    ///
    /// * Homogeneous literal: `List<Expr<T>>` where `T` is the LUB.
    /// * Nested literal: `List<List<…>>`.
    /// * Heterogeneous literal: `List<Unknown>`.
    /// * Empty literal with target: the target `List<T>`.
    /// * Empty literal without target: `List<Unknown>`.
    pub inferred: SmeltType,
    /// Pending diagnostic sentinels. Empty on the happy path.
    pub sentinels: Vec<ListInferSentinel>,
}

/// Infer the type of a list literal `[e_1, …, e_n]` (Phase A, meta-language).
///
/// Control flow:
/// 1. Empty literal — consult `expected`; produce `List<T>` if known, else
///    `List<Unknown>` + [`ListInferSentinel::EmptyTypeUnknown`].
/// 2. Non-empty — for each element compute its [`SmeltType`]:
///    * A list-literal element recursively calls this function → yields
///      `List<…>`.
///    * A scalar element is inferred via [`infer_expression_type`] → yields
///      `Expr<T>`.
///
///    NULL scalars are skipped (compatible with any type).
/// 3. The resulting `SmeltType`s are LUB-ed via [`smelt_type_lub`]:
///    * Two `Expr<T>` values use [`promote_types`] on their inner `DataType`.
///    * Two identical `SmeltType`s are already equal.
///    * Any cross-sort mix (`Expr<…>` vs `List<…>`) or same-sort but
///      incompatible pair produces `List<Unknown>` +
///      [`ListInferSentinel::Heterogeneous`].
///
/// This single-function design makes the "scalar wrapped as List" bug
/// structurally impossible: scalars always carry sort `Expr<…>`, list-literal
/// elements always carry sort `List<…>`, so a cross-sort mix naturally fails
/// the LUB step.
///
/// Pure function — no Salsa dependency. `TypeContext` is accepted for element
/// type inference but the function itself performs no Salsa calls.
pub fn infer_list_literal(
    elements: &[Expr],
    ctx: &TypeContext,
    expected: Option<&SmeltType>,
) -> ListLiteralInferResult {
    // Empty literal — consult expected type.
    if elements.is_empty() {
        if let Some(exp) = expected {
            // The expected type must be a List<T> to be meaningful context for
            // an empty list literal. If the caller passes a non-List expected
            // (e.g. `Expr<Numeric>` at an unconstrained position), fall through
            // to the unknown-target branch rather than returning a non-List type.
            if matches!(exp, SmeltType::List(_)) {
                // Expected is a fully-formed List<T>; return it (spec rule 4).
                return ListLiteralInferResult {
                    inferred: exp.clone(),
                    sentinels: vec![],
                };
            }
            // Expected is not a List<T> — treat as unconstrained.
        }
        return ListLiteralInferResult {
            inferred: SmeltType::List(Box::new(SmeltType::Unknown)),
            sentinels: vec![ListInferSentinel::EmptyTypeUnknown],
        };
    }

    // Non-empty: infer each element as a SmeltType, then LUB at SmeltType level.
    // Working at SmeltType level (not DataType level) is what makes cross-sort
    // heterogeneity (Expr<T> vs List<T>) naturally detectable.
    let mut accumulated: Option<SmeltType> = None;
    let mut sentinels: Vec<ListInferSentinel> = Vec::new();
    let mut heterogeneous = false;

    for elem in elements {
        // Determine element's SmeltType:
        //   - list-literal element → recurse → List<…>
        //   - scalar element       → infer_expression_type → Expr<T>
        let (elem_ty, elem_sentinels) = if let Some(inner_arr) = elem.as_array_literal() {
            let inner_elems: Vec<Expr> = inner_arr.elements();
            let r = infer_list_literal(&inner_elems, ctx, None);
            (r.inferred, r.sentinels)
        } else {
            let typed = infer_expression_type(elem, ctx).unwrap_or(TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            });
            let dt = typed.data_type;
            if dt == DataType::Null {
                // NULL is compatible with any element type; skip without
                // contributing to the running LUB.
                continue;
            }
            let smelt = SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(dt));
            (smelt, vec![])
        };

        sentinels.extend(elem_sentinels);

        match accumulated.take() {
            None => {
                accumulated = Some(elem_ty);
            }
            Some(existing) => {
                // LUB at SmeltType level.
                let lub = smelt_type_lub(&existing, &elem_ty);
                match lub {
                    Some(t) => {
                        accumulated = Some(t);
                    }
                    None => {
                        // Incompatible sorts or types → heterogeneous.
                        if !heterogeneous {
                            sentinels.push(ListInferSentinel::Heterogeneous {
                                first: existing.clone(),
                                incompatible: elem_ty,
                            });
                            heterogeneous = true;
                        }
                        // Restore `existing` so the accumulated first type is
                        // preserved for subsequent elements' sentinel comparison.
                        accumulated = Some(existing);
                    }
                }
            }
        }
    }

    if heterogeneous {
        return ListLiteralInferResult {
            inferred: SmeltType::List(Box::new(SmeltType::Unknown)),
            sentinels,
        };
    }

    // All elements were NULL — use Null as the element type.
    let elem_ty = accumulated.unwrap_or(SmeltType::Expr(
        smelt_types::signatures::TypeConstraint::Concrete(DataType::Null),
    ));
    ListLiteralInferResult {
        inferred: SmeltType::List(Box::new(elem_ty)),
        sentinels,
    }
}

/// Compute the least-upper-bound (LUB) of two [`SmeltType`]s for the purposes
/// of list-element unification (Phase A).
///
/// Rules:
/// * Two identical types → `Some(that type)`.
/// * Two `Expr<Concrete(T)>` values → promote their inner `DataType` via
///   [`promote_types`]; `Unknown` result means incompatible → `None`.
/// * Two `List<T>` values → recurse into inner types; `None` inner → `None`.
/// * Any cross-sort pair (`Expr` vs `List`, etc.) → `None`.
///
/// Returns `None` when the types cannot be unified.
fn smelt_type_lub(a: &SmeltType, b: &SmeltType) -> Option<SmeltType> {
    if a == b {
        return Some(a.clone());
    }
    match (a, b) {
        (
            SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(da)),
            SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(db)),
        ) => {
            let promoted = promote_types(
                &TypedColumn::not_null(da.clone()),
                &TypedColumn::not_null(db.clone()),
            );
            if promoted.data_type == DataType::Unknown {
                None
            } else {
                Some(SmeltType::Expr(
                    smelt_types::signatures::TypeConstraint::Concrete(promoted.data_type),
                ))
            }
        }
        (SmeltType::List(inner_a), SmeltType::List(inner_b)) => {
            // Nested lists: LUB on inner types.
            smelt_type_lub(inner_a, inner_b).map(|inner_lub| SmeltType::List(Box::new(inner_lub)))
        }
        // Cross-sort or any other pair that isn't equal and doesn't fit above.
        _ => None,
    }
}

// ─── Phase A Phase 3: diagnostics + bidirectional disambiguation + spread ────

/// Position kind for spread validation. Determines whether a spread operator
/// is in a valid or forbidden position.
///
/// Valid positions: Select, GroupBy, OrderBy, FunctionArg, InList, ValuesRow,
/// ListLiteralBody.
/// Forbidden positions: Where, Boolean, NamedArgValue, FromWithoutReducer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplicePosition {
    /// SELECT list — spread allowed.
    Select,
    /// GROUP BY — spread allowed.
    GroupBy,
    /// ORDER BY — spread allowed.
    OrderBy,
    /// Positional function argument — spread allowed.
    FunctionArg,
    /// IN-list `x IN (...vs)` — spread allowed.
    InList,
    /// VALUES row — spread allowed.
    ValuesRow,
    /// Inside another list literal `[a, ...xs, b]` — spread allowed.
    ListLiteralBody,
    /// WHERE clause — spread forbidden.
    Where,
    /// Boolean composition (`x AND ...preds`, `y OR ...preds`) — spread forbidden.
    Boolean,
    /// Named-argument value (`name => value`) — spread forbidden.
    NamedArgValue,
    /// FROM clause without an explicit reducer — spread forbidden.
    FromWithoutReducer,
}

impl SplicePosition {
    /// Returns `true` if spread is allowed in this position.
    pub fn spread_allowed(self) -> bool {
        matches!(
            self,
            SplicePosition::Select
                | SplicePosition::GroupBy
                | SplicePosition::OrderBy
                | SplicePosition::FunctionArg
                | SplicePosition::InList
                | SplicePosition::ValuesRow
                | SplicePosition::ListLiteralBody
        )
    }

    /// Human-readable position name for the `MetaSpreadInForbiddenPosition`
    /// message, matching the spec's prescribed strings.
    pub fn forbidden_position_name(self) -> &'static str {
        match self {
            SplicePosition::Where => "WHERE clause",
            SplicePosition::Boolean => "boolean composition",
            SplicePosition::NamedArgValue => "named argument value",
            SplicePosition::FromWithoutReducer => "FROM clause without an explicit reducer",
            _ => "unknown position",
        }
    }
}

/// Result of the bidirectional disambiguation of a `[...]` list literal.
///
/// Implements spec `meta_language.md` Phase A §"Rule 3 — Bidirectional
/// disambiguation":
/// - `List<T>` expected → `MetaList`.
/// - `Expr<Array<U>>` expected → `DataWorldArray`.
/// - Both admissible → `MetaList` (meta wins).
/// - Neither admissible → `MetaList` with `List<Unknown>` (error type).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListDisambiguation {
    /// Treat the literal as a compile-time meta `List<T>`. The carried value
    /// is the infer result from [`infer_list_literal`].
    MetaList(ListLiteralInferResult),
    /// Treat the literal as a Data-World runtime array (`ARRAY[...]`).
    DataWorldArray,
}

/// Disambiguate a `[...]` literal between meta-list and Data-World array.
///
/// Implements spec rule 3:
/// - If `expected` is `Some(List<T>)` → meta-list interpretation.
/// - If `expected` is `Some(Expr<Array<U>>)` → Data-World array.
/// - Otherwise (both admissible or no expected) → meta-list wins.
///
/// Pure function — no Salsa dependency.
pub fn disambiguate_list_literal(
    elements: &[smelt_parser::ast::Expr],
    ctx: &TypeContext,
    expected: Option<&SmeltType>,
) -> ListDisambiguation {
    match expected {
        // Explicitly expected Data-World array → data path.
        Some(SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
            DataType::Array(_),
        ))) => ListDisambiguation::DataWorldArray,
        // Explicitly expected meta-list (List<T>) → pass expected through.
        Some(SmeltType::List(_)) => {
            let result = infer_list_literal(elements, ctx, expected);
            ListDisambiguation::MetaList(result)
        }
        // Both admissible (no expected, or an unrelated expected) → meta wins.
        _ => {
            let result = infer_list_literal(elements, ctx, None);
            ListDisambiguation::MetaList(result)
        }
    }
}

/// Convert `ListInferSentinel`s to `Diagnostic` records.
///
/// This is Phase 3's "sentinel to diagnostic" converter. It takes the
/// sentinels produced by `infer_list_literal` and converts them into
/// diagnostics anchored at `span` (the list literal's source span).
///
/// Pure function — no Salsa dependency. Returns the diagnostics so the
/// caller can append them to whatever accumulator is in use.
pub fn list_literal_sentinels_to_diagnostics(
    elements: &[smelt_parser::ast::Expr],
    ctx: &TypeContext,
    span: rowan::TextRange,
) -> Vec<crate::Diagnostic> {
    use smelt_types::signatures::format_smelt_type_hover;

    let result = infer_list_literal(elements, ctx, None);
    let mut diags = Vec::new();

    let range = span;

    for sentinel in &result.sentinels {
        let (code, message) = match sentinel {
            ListInferSentinel::EmptyTypeUnknown => (
                crate::DiagnosticCode::MetaListEmptyTypeUnknown,
                crate::meta_list_diagnostic_message(
                    crate::DiagnosticCode::MetaListEmptyTypeUnknown,
                    None,
                    None,
                    None,
                ),
            ),
            ListInferSentinel::Heterogeneous {
                first,
                incompatible,
            } => {
                let t0 = format_smelt_type_hover(first);
                let tk = format_smelt_type_hover(incompatible);
                (
                    crate::DiagnosticCode::MetaListHeterogeneous,
                    crate::meta_list_diagnostic_message(
                        crate::DiagnosticCode::MetaListHeterogeneous,
                        Some(&t0),
                        Some(&tk),
                        None,
                    ),
                )
            }
        };
        diags.push(crate::Diagnostic {
            severity: crate::DiagnosticSeverity::Error,
            message,
            range,
            code: Some(code),
            data: None,
        });
    }

    diags
}

/// Provenance origin tag — Phase A Phase 3.
///
/// Records where a synthesized item came from. `Synthesized(SpreadFrom(span))`
/// marks each item emitted by a spread expansion. The `TextRange` is the span
/// of the spread operand (the `...xs` expression).
///
/// This is the Phase A minimal implementation. Phase B will extend this with
/// `Caller(span)` and `Callee(fn_id, span)` variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OriginTag {
    /// Item was synthesized by a spread expansion. The `TextRange` is the
    /// span of the `...operand` expression in the source.
    Synthesized(SynthesizedReason),
}

/// Reason for synthesis, per `expansion.md` §"Provenance origin tags".
///
/// Phase A adds `SpreadFrom`; Phase B will add the full `fn_id`-bearing
/// variants. `fn_id` is `Option<String>` here because a top-level spread
/// has no enclosing function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SynthesizedReason {
    /// Emitted by a list spread `...xs`. The `TextRange` is the span of the
    /// spread operator node.
    SpreadFrom(rowan::TextRange),
}

/// Result returned by [`check_select_list_spreads`].
///
/// Records what the spread-expansion check found: how many items were
/// expanded (for testing), any diagnostics emitted, and provenance tags for
/// the expanded items.
#[derive(Debug, Clone)]
pub struct SelectListSpreadResult {
    /// Total number of items produced by spread expansion across all spreads
    /// in the SELECT list (sum of individual spread expansion widths).
    pub expanded_item_count: usize,
    /// Diagnostics emitted during expansion (e.g. `MetaSpreadOnNonList`).
    pub diagnostics: Vec<crate::Diagnostic>,
    /// Provenance origin tags for every item produced by spread expansion.
    /// Length equals `expanded_item_count`.
    pub provenance_tags: Vec<OriginTag>,
}

/// Check all `LIST_SPREAD` nodes in a SELECT list for validity and expand them.
///
/// For each `LIST_SPREAD` found as a direct child of the SELECT_LIST:
/// 1. Infer the operand's type.
/// 2. If it is `List<T>`, expand the elements — each gets a
///    `Synthesized(SpreadFrom(spread_span))` provenance tag.
/// 3. If it is not `List<T>`, emit `MetaSpreadOnNonList` and drop the spread.
/// 4. Empty-list spreads produce zero expansion items (elision).
///
/// Forbidden-position checking (WHERE, etc.) is handled separately by
/// [`check_forbidden_position_spreads`].
///
/// Pure function — no Salsa dependency.
pub fn check_select_list_spreads(
    select_stmt: &smelt_parser::ast::SelectStmt,
    ctx: &TypeContext,
) -> SelectListSpreadResult {
    use smelt_parser::SyntaxKind::{LIST_SPREAD, SELECT_LIST};
    use smelt_types::signatures::format_smelt_type_hover;

    let mut result = SelectListSpreadResult {
        expanded_item_count: 0,
        diagnostics: Vec::new(),
        provenance_tags: Vec::new(),
    };

    // Find the SELECT_LIST node, then iterate its children for LIST_SPREAD nodes.
    let select_list_node = select_stmt
        .syntax()
        .children()
        .find(|n| n.kind() == SELECT_LIST);

    let Some(sl_node) = select_list_node else {
        return result;
    };

    for child in sl_node.children() {
        if child.kind() != LIST_SPREAD {
            continue;
        }
        let spread = smelt_parser::ast::ListSpread::cast(child.clone())
            .expect("LIST_SPREAD node must cast to ListSpread");
        let spread_span = child.text_range();

        let Some(operand) = spread.operand() else {
            continue;
        };

        // Check first if the operand is an inline list literal `...[e1, e2, ...]`.
        // This must be checked before `infer_expression_type` because the parser
        // emits `[]` as an ARRAY_LITERAL which `infer_expression_type` would see
        // as a Data-World `Array(Unknown)` rather than a meta-list.
        if let Some(arr) = operand.as_array_literal() {
            // Inline list literal spread `...[e1, e2, ...]`.
            let elements: Vec<_> = arr.elements();

            if elements.is_empty() {
                // Empty-list spread: elide with zero expansion and no diagnostics.
                // Spec rule 7: "A spread of any compile-time empty List<T> emits
                // zero copies; adjacent commas elide."
                continue;
            }

            let infer_result = infer_list_literal(&elements, ctx, None);

            // Emit sentinels as diagnostics (heterogeneous only — EmptyTypeUnknown
            // can't fire here since we checked is_empty() above).
            for sentinel in &infer_result.sentinels {
                let (code, message) = match sentinel {
                    ListInferSentinel::EmptyTypeUnknown => {
                        // Unreachable: we checked is_empty() above.
                        continue;
                    }
                    ListInferSentinel::Heterogeneous {
                        first,
                        incompatible,
                    } => {
                        let t0 = format_smelt_type_hover(first);
                        let tk = format_smelt_type_hover(incompatible);
                        (
                            crate::DiagnosticCode::MetaListHeterogeneous,
                            crate::meta_list_diagnostic_message(
                                crate::DiagnosticCode::MetaListHeterogeneous,
                                Some(&t0),
                                Some(&tk),
                                None,
                            ),
                        )
                    }
                };
                let span_range = spread_span;
                result.diagnostics.push(crate::Diagnostic {
                    severity: crate::DiagnosticSeverity::Error,
                    message,
                    range: span_range,
                    code: Some(code),
                    data: None,
                });
            }

            // Expand: emit provenance per element.
            let elem_count = elements.len();
            for _ in 0..elem_count {
                result
                    .provenance_tags
                    .push(OriginTag::Synthesized(SynthesizedReason::SpreadFrom(
                        spread_span,
                    )));
            }
            result.expanded_item_count += elem_count;
            continue;
        }

        // Non-literal operand: infer the operand's type.
        let operand_type = infer_expression_type(&operand, ctx);

        match operand_type {
            Some(typed) => {
                match &typed.data_type {
                    DataType::Unknown => {
                        // Unknown type — propagate without error.
                        // Handles unresolvable identifiers gracefully (no avalanche).
                    }
                    _ => {
                        // Non-list type → emit MetaSpreadOnNonList and drop.
                        let actual = smelt_types::SmeltType::Expr(
                            smelt_types::signatures::TypeConstraint::Concrete(
                                typed.data_type.clone(),
                            ),
                        );
                        let actual_str = format_smelt_type_hover(&actual);
                        let span_range = spread_span;
                        result.diagnostics.push(crate::Diagnostic {
                            severity: crate::DiagnosticSeverity::Error,
                            message: crate::meta_list_diagnostic_message(
                                crate::DiagnosticCode::MetaSpreadOnNonList,
                                None,
                                Some(&actual_str),
                                None,
                            ),
                            range: span_range,
                            code: Some(crate::DiagnosticCode::MetaSpreadOnNonList),
                            data: None,
                        });
                        // Drop spread — continue type-checking.
                    }
                }
            }
            None => {
                // Could not infer type — treat as empty (no avalanche).
            }
        }
    }

    result
}

/// Detect `LIST_SPREAD` nodes (or orphaned `DOT_DOT_DOT` tokens produced by
/// parse-error recovery) in forbidden positions and emit
/// `MetaSpreadInForbiddenPosition` diagnostics.
///
/// **Current parser behaviour**: The parser only emits `LIST_SPREAD` nodes in
/// valid spread positions (SELECT list, GROUP BY, ORDER BY, function args,
/// IN-list, VALUES rows, list-literal body). When a `...` appears in a
/// forbidden position (e.g. `WHERE ...preds`), the parser's error-recovery
/// ejects the `DOT_DOT_DOT` token outside the `WHERE_CLAUSE` node (typically
/// as a sibling of the `SELECT_STMT` at the `FILE` level). This function
/// detects both cases:
///
/// 1. `LIST_SPREAD` nodes that somehow appear inside a `WHERE_CLAUSE`
///    descendant (future-proofing).
/// 2. Orphaned `DOT_DOT_DOT` tokens at the parent node level when the
///    `SelectStmt` has a `WHERE` clause — these represent spread-in-WHERE
///    parse errors.
///
/// Pure function — no Salsa dependency.
pub fn check_forbidden_position_spreads(
    select_stmt: &smelt_parser::ast::SelectStmt,
    _ctx: &TypeContext,
) -> Vec<crate::Diagnostic> {
    use smelt_parser::SyntaxKind::{DOT_DOT_DOT, LIST_SPREAD, WHERE_CLAUSE};

    let mut diags = Vec::new();

    let has_where = select_stmt
        .syntax()
        .children()
        .any(|n| n.kind() == WHERE_CLAUSE);

    // Case 1: Walk the WHERE clause for LIST_SPREAD descendants (future-proofing
    // for if the parser is updated to allow `...` in WHERE context).
    if has_where {
        let where_node = select_stmt
            .syntax()
            .children()
            .find(|n| n.kind() == WHERE_CLAUSE);

        if let Some(wn) = where_node {
            for desc in wn.descendants() {
                if desc.kind() == LIST_SPREAD {
                    let range = desc.text_range();
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_list_diagnostic_message(
                            crate::DiagnosticCode::MetaSpreadInForbiddenPosition,
                            None,
                            None,
                            Some(SplicePosition::Where.forbidden_position_name()),
                        ),
                        range,
                        code: Some(crate::DiagnosticCode::MetaSpreadInForbiddenPosition),
                        data: None,
                    });
                }
            }
        }

        // Case 2: Detect orphaned DOT_DOT_DOT tokens at the parent level.
        // The parser ejects `...expr` from the WHERE body on parse error, leaving
        // the DOT_DOT_DOT token(s) as siblings of the SELECT_STMT in the parent.
        if let Some(parent) = select_stmt.syntax().parent() {
            let stmt_range = select_stmt.syntax().text_range();

            // Walk the parent's children_with_tokens AFTER the SELECT_STMT node.
            let mut past_stmt = false;
            for child in parent.children_with_tokens() {
                let child_range = match &child {
                    rowan::NodeOrToken::Node(n) => n.text_range(),
                    rowan::NodeOrToken::Token(t) => t.text_range(),
                };

                if !past_stmt {
                    // Check if this child IS or FOLLOWS the select stmt.
                    if child_range.start() >= stmt_range.end() {
                        past_stmt = true;
                    } else if child_range == stmt_range {
                        past_stmt = true;
                        continue; // skip the stmt itself
                    } else {
                        continue;
                    }
                }

                // Look for DOT_DOT_DOT tokens after the SELECT_STMT.
                if let rowan::NodeOrToken::Token(tok) = &child {
                    if tok.kind() == DOT_DOT_DOT {
                        let range = tok.text_range();
                        diags.push(crate::Diagnostic {
                            severity: crate::DiagnosticSeverity::Error,
                            message: crate::meta_list_diagnostic_message(
                                crate::DiagnosticCode::MetaSpreadInForbiddenPosition,
                                None,
                                None,
                                Some(SplicePosition::Where.forbidden_position_name()),
                            ),
                            range,
                            code: Some(crate::DiagnosticCode::MetaSpreadInForbiddenPosition),
                            data: None,
                        });
                    }
                }
                // Also check for LIST_SPREAD nodes at parent level.
                if let rowan::NodeOrToken::Node(node) = &child {
                    if node.kind() == LIST_SPREAD {
                        let range = node.text_range();
                        diags.push(crate::Diagnostic {
                            severity: crate::DiagnosticSeverity::Error,
                            message: crate::meta_list_diagnostic_message(
                                crate::DiagnosticCode::MetaSpreadInForbiddenPosition,
                                None,
                                None,
                                Some(SplicePosition::Where.forbidden_position_name()),
                            ),
                            range,
                            code: Some(crate::DiagnosticCode::MetaSpreadInForbiddenPosition),
                            data: None,
                        });
                    }
                }
            }
        }
    }

    // Boolean composition, named-arg value, FROM-without-reducer:
    // These are Phase B extensions — the spec's full forbidden-position list
    // is enforced here for WHERE (the tested case). The other positions will
    // be wired when their parser positions are exercised.

    diags
}

/// Phase B+ unified entry point for spread expansion.
///
/// Phase A wires only SELECT-list via [`check_select_list_spreads`]; this
/// function will be the integration target for GROUP BY / ORDER BY / function
/// args / IN-list / VALUES in Phase B.
///
/// This is the general-purpose spread expansion function that handles:
/// (a) forbidden-position validation → emits `MetaSpreadInForbiddenPosition`
/// (b) non-list operand → emits `MetaSpreadOnNonList`
/// (c) valid expansion → returns per-element provenance tags
///
/// Returns `(expanded_items, diagnostics)` where `expanded_items` is the
/// number of items the spread contributed (0 for non-list or forbidden; the
/// actual element count for valid expansion).
///
/// Pure function — no Salsa dependency.
#[allow(dead_code)]
pub fn expand_spread_into_position(
    spread: &smelt_parser::ast::ListSpread,
    ctx: &TypeContext,
    position: SplicePosition,
) -> (usize, Vec<OriginTag>, Vec<crate::Diagnostic>) {
    use smelt_types::signatures::format_smelt_type_hover;

    let zero_range = rowan::TextRange::empty(rowan::TextSize::from(0));

    // (a) Forbidden-position check.
    if !position.spread_allowed() {
        let diag = crate::Diagnostic {
            severity: crate::DiagnosticSeverity::Error,
            message: crate::meta_list_diagnostic_message(
                crate::DiagnosticCode::MetaSpreadInForbiddenPosition,
                None,
                None,
                Some(position.forbidden_position_name()),
            ),
            range: zero_range,
            code: Some(crate::DiagnosticCode::MetaSpreadInForbiddenPosition),
            data: None,
        };
        return (0, vec![], vec![diag]);
    }

    let spread_span = spread.syntax().text_range();

    let Some(operand) = spread.operand() else {
        return (0, vec![], vec![]);
    };

    // (b) Determine element count by checking operand type.
    // If the operand is an inline list literal, use its element count directly.
    if let Some(arr) = operand.as_array_literal() {
        let elements: Vec<_> = arr.elements();
        let elem_count = elements.len();
        let tags: Vec<OriginTag> = (0..elem_count)
            .map(|_| OriginTag::Synthesized(SynthesizedReason::SpreadFrom(spread_span)))
            .collect();
        return (elem_count, tags, vec![]);
    }

    // Otherwise, infer the operand's type.
    let operand_ty = infer_expression_type(&operand, ctx);
    match operand_ty {
        Some(typed) => {
            match &typed.data_type {
                DataType::Unknown => {
                    // Unknown — treat as empty expansion to avoid false positives.
                    (0, vec![], vec![])
                }
                _ => {
                    // Non-list type → emit MetaSpreadOnNonList and drop.
                    let actual = SmeltType::Expr(
                        smelt_types::signatures::TypeConstraint::Concrete(typed.data_type.clone()),
                    );
                    let actual_str = format_smelt_type_hover(&actual);
                    let diag = crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_list_diagnostic_message(
                            crate::DiagnosticCode::MetaSpreadOnNonList,
                            None,
                            Some(&actual_str),
                            None,
                        ),
                        range: zero_range,
                        code: Some(crate::DiagnosticCode::MetaSpreadOnNonList),
                        data: None,
                    };
                    (0, vec![], vec![diag])
                }
            }
        }
        None => {
            // Cannot infer — treat as empty (no avalanche).
            (0, vec![], vec![])
        }
    }
}

// ─── Phase B (meta-language): HOF position checks + diagnostic emission ──────

/// Check a `smelt.define` declaration for name-shadowing of built-in HOFs or reducers.
///
/// The set of protected HOF names is `{map, filter, reduce}`.
/// The set of protected reducer names is `{comma_sep, and_all, or_any, union_all,
/// intersect_all, plus_chain, concat}`.
///
/// Emits:
/// - `HofNameShadowed` when the declared name is a built-in HOF.
/// - `ReducerNameShadowed` when the declared name is a built-in reducer.
///
/// Pure function — no Salsa dependency.
pub fn check_define_name_shadowing(
    define: &smelt_parser::ast::SmeltDefine,
) -> Vec<crate::Diagnostic> {
    const HOF_NAMES: &[&str] = &["map", "filter", "reduce"];
    /// Reserved ternary keywords — must not be used as smelt.define, smelt.record,
    /// or lambda-parameter names.
    const TERNARY_KEYWORDS: &[&str] = &["if", "then", "else"];

    let mut diags = Vec::new();
    let Some(name) = define.name() else {
        return diags;
    };
    let name_lc = name.to_lowercase();

    let range = define
        .name_range()
        .unwrap_or_else(|| rowan::TextRange::empty(rowan::TextSize::from(0)));

    if HOF_NAMES.contains(&name_lc.as_str()) {
        diags.push(crate::Diagnostic {
            severity: crate::DiagnosticSeverity::Error,
            message: crate::meta_hof_diagnostic_message(
                crate::DiagnosticCode::HofNameShadowed,
                Some(&name),
                None,
                None,
                None,
                None,
                None,
                None,
            ),
            range,
            code: Some(crate::DiagnosticCode::HofNameShadowed),
            data: None,
        });
    } else if REDUCER_REGISTRY
        .iter()
        .any(|r| r.name.eq_ignore_ascii_case(name_lc.as_str()))
    {
        diags.push(crate::Diagnostic {
            severity: crate::DiagnosticSeverity::Error,
            message: crate::meta_hof_diagnostic_message(
                crate::DiagnosticCode::ReducerNameShadowed,
                None,
                Some(&name),
                None,
                None,
                None,
                None,
                None,
            ),
            range,
            code: Some(crate::DiagnosticCode::ReducerNameShadowed),
            data: None,
        });
    } else if TERNARY_KEYWORDS.contains(&name_lc.as_str()) {
        diags.push(crate::Diagnostic {
            severity: crate::DiagnosticSeverity::Error,
            message: crate::meta_hof_diagnostic_message(
                crate::DiagnosticCode::TernaryKeywordShadowed,
                Some(&name),
                None,
                None,
                None,
                None,
                None,
                None,
            ),
            range,
            code: Some(crate::DiagnosticCode::TernaryKeywordShadowed),
            data: None,
        });
    }

    diags
}

/// Walk a `SelectStmt` and emit Phase B HOF-related diagnostics:
///
/// - `LambdaInForbiddenPosition`: a `LAMBDA` CST node whose parent is not a
///   HOF positional argument.
/// - `LambdaArityMismatch`: a `LAMBDA` in map/filter position with arity != 1.
/// - `LambdaZeroParameters`: a `LAMBDA` with no parameters.
/// - `LambdaDuplicateParameter`: a `LAMBDA` with duplicate parameter names.
/// - `LambdaResultTypeMismatch`: `filter` predicate body not `Boolean`.
/// - `HofExpectsLambda`: second arg to `map`/`filter` is not a `LAMBDA`.
/// - `HofExpectsReducer`: second arg to `reduce` is not a registered reducer.
/// - `PipeRhsNotCall`: RHS of `|>` is not a call expression.
/// - `ReducerInputTypeMismatch`: reducer applied to incompatible input type.
/// - `ReducerEmptyNoIdentity`: `union_all`/`intersect_all` reducing an empty list.
///
/// Pure function — no Salsa dependency. `text` is the raw source for span
/// conversion; pass `""` in unit tests where exact position is not under test.
pub fn check_hof_position_diagnostics(
    select_stmt: &smelt_parser::ast::SelectStmt,
    ctx: &TypeContext,
    text: &str,
) -> Vec<crate::Diagnostic> {
    use smelt_parser::SyntaxKind::{FUNCTION_CALL, LAMBDA, PIPE_EXPR};
    use smelt_types::signatures::format_smelt_type_hover;

    let mut diags: Vec<crate::Diagnostic> = Vec::new();

    // Helper: get function name from a FunctionCall node, including keyword-named
    // functions like `filter` which lex as FILTER_KW rather than IDENT.
    let call_name_lc = |call: &smelt_parser::ast::FunctionCall| -> String {
        call.name()
            .unwrap_or_else(|| {
                // `FunctionCall::name()` only finds IDENT tokens; HOF names like
                // `filter` are lexed as keyword tokens (FILTER_KW). Fall back to the
                // first non-trivia token's text.
                call.syntax()
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .find(|t| !t.kind().is_trivia())
                    .map(|t| t.text().to_string())
                    .unwrap_or_default()
            })
            .to_lowercase()
    };

    // Collect every descendant of the select statement for inspection.
    // We perform a depth-first walk; each node is examined individually.
    let root = select_stmt.syntax();

    // Helper: range from a Rowan TextRange, text may be "" in tests.
    // Walk all descendants.
    for node in root.descendants() {
        match node.kind() {
            // ── LAMBDA node ────────────────────────────────────────────────
            LAMBDA => {
                let lambda = smelt_parser::ast::Lambda::cast(node.clone()).unwrap();
                let lambda_range = node.text_range();

                // Arity check: does the LAMBDA have zero parameters?
                let param_count = lambda.lambda_params().len();
                let has_zero_params = param_count == 0;

                if has_zero_params {
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_hof_diagnostic_message(
                            crate::DiagnosticCode::LambdaZeroParameters,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                        ),
                        range: lambda_range,
                        code: Some(crate::DiagnosticCode::LambdaZeroParameters),
                        data: None,
                    });
                    // Still check for forbidden position below.
                }

                // Duplicate parameter check.
                {
                    let mut seen = std::collections::HashSet::new();
                    for lp in lambda.lambda_params() {
                        if let Some(pname) = lp.name() {
                            if !seen.insert(pname.clone()) {
                                let dup_range = lp.syntax().text_range();
                                diags.push(crate::Diagnostic {
                                    severity: crate::DiagnosticSeverity::Error,
                                    message: crate::meta_hof_diagnostic_message(
                                        crate::DiagnosticCode::LambdaDuplicateParameter,
                                        None,
                                        Some(&pname),
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                    ),
                                    range: dup_range,
                                    code: Some(crate::DiagnosticCode::LambdaDuplicateParameter),
                                    data: None,
                                });
                            }
                        }
                    }
                }

                // Position check: is this LAMBDA inside a HOF positional argument?
                // Also detect which HOF to check arity expectations.
                //
                // Parent chain: LAMBDA → EXPRESSION → ARG_LIST → FUNCTION_CALL
                // We walk up through EXPRESSION and ARG_LIST wrappers.
                let hof_context: Option<HofKind> = {
                    let mut parent_opt = node.parent();
                    let mut found_hof: Option<HofKind> = None;

                    // Walk up through EXPRESSION / ARG_LIST wrappers.
                    while let Some(p) = parent_opt {
                        match p.kind() {
                            FUNCTION_CALL => {
                                // Reached a function call — check it's a known HOF.
                                let call =
                                    smelt_parser::ast::FunctionCall::cast(p.clone()).unwrap();
                                let name = call_name_lc(&call);
                                found_hof = HofKind::from_name(&name);
                                break;
                            }
                            smelt_parser::SyntaxKind::EXPRESSION
                            | smelt_parser::SyntaxKind::ARG_LIST => {
                                // Keep walking up through transparent wrapper nodes.
                                parent_opt = p.parent();
                            }
                            _ => break,
                        }
                    }
                    found_hof
                };

                if hof_context.is_none() {
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_hof_diagnostic_message(
                            crate::DiagnosticCode::LambdaInForbiddenPosition,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                        ),
                        range: lambda_range,
                        code: Some(crate::DiagnosticCode::LambdaInForbiddenPosition),
                        data: None,
                    });
                } else if let Some(hof_kind) = hof_context {
                    // Check arity expectations: map/filter require arity 1.
                    // (reduce takes a reducer, not a lambda, so lambdas in reduce
                    // position will emit HofExpectsReducer from the FUNCTION_CALL walk.)
                    let required_arity: Option<usize> = match hof_kind {
                        HofKind::Map | HofKind::Filter => Some(1),
                        HofKind::Reduce => None, // reduce doesn't use lambdas
                    };
                    if let Some(req) = required_arity {
                        if param_count != req && !has_zero_params {
                            let hof_name = match hof_kind {
                                HofKind::Map => "map",
                                HofKind::Filter => "filter",
                                HofKind::Reduce => "reduce",
                            };
                            diags.push(crate::Diagnostic {
                                severity: crate::DiagnosticSeverity::Error,
                                message: crate::meta_hof_diagnostic_message(
                                    crate::DiagnosticCode::LambdaArityMismatch,
                                    Some(hof_name),
                                    None,
                                    Some(&req.to_string()),
                                    Some(&param_count.to_string()),
                                    None,
                                    None,
                                    None,
                                ),
                                range: lambda_range,
                                code: Some(crate::DiagnosticCode::LambdaArityMismatch),
                                data: None,
                            });
                        }
                    }
                }
            }

            // ── FUNCTION_CALL node: validate HOF argument shapes ───────────
            FUNCTION_CALL => {
                let call = match smelt_parser::ast::FunctionCall::cast(node.clone()) {
                    Some(c) => c,
                    None => continue,
                };
                let call_name = call_name_lc(&call);
                let Some(hof) = HofKind::from_name(&call_name) else {
                    continue;
                };

                let args = call.arguments();
                if args.is_empty() {
                    continue;
                }

                // First arg: infer list type.
                let first_arg = &args[0];
                let lhs_ty = if let Some(arr) = first_arg.as_array_literal() {
                    let elems: Vec<_> = arr.elements();
                    infer_list_literal(&elems, ctx, None).inferred
                } else {
                    infer_expression_type(first_arg, ctx)
                        .map(|tc| {
                            SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                                tc.data_type,
                            ))
                        })
                        .unwrap_or(SmeltType::Unknown)
                };

                let call_range = node.text_range();

                match hof {
                    HofKind::Map | HofKind::Filter => {
                        if args.len() < 2 {
                            continue;
                        }
                        let second_arg = &args[1];

                        // Check if second arg contains any LAMBDA node.
                        // Phase F: multi-arg lambdas now parse as proper LAMBDA CST nodes,
                        // so `has_any_lambda` includes both single-arg and multi-arg lambdas.
                        // Arity mismatch (multi-arg in map/filter) is detected by the LAMBDA
                        // node walk above and emits LambdaArityMismatch there.
                        let has_any_lambda = second_arg
                            .syntax()
                            .descendants()
                            .any(|n| n.kind() == LAMBDA)
                            || second_arg.syntax().kind() == LAMBDA;

                        // Check if second arg contains a valid single-arg LAMBDA.
                        let has_valid_lambda = extract_lambda_from_expr(second_arg).is_some();

                        if !has_any_lambda {
                            // HofExpectsLambda — not a lambda at all.
                            let actual = infer_expression_type(second_arg, ctx)
                                .map(|tc| {
                                    format_smelt_type_hover(&SmeltType::Expr(
                                        smelt_types::signatures::TypeConstraint::Concrete(
                                            tc.data_type,
                                        ),
                                    ))
                                })
                                .unwrap_or_else(|| "unknown".to_string());
                            diags.push(crate::Diagnostic {
                                severity: crate::DiagnosticSeverity::Error,
                                message: crate::meta_hof_diagnostic_message(
                                    crate::DiagnosticCode::HofExpectsLambda,
                                    Some(&call_name),
                                    None,
                                    None,
                                    Some(&actual),
                                    None,
                                    None,
                                    None,
                                ),
                                range: second_arg.text_range(),
                                code: Some(crate::DiagnosticCode::HofExpectsLambda),
                                data: None,
                            });
                        } else if has_valid_lambda {
                            // Valid single-arg lambda in map or filter.
                            // (a) For filter: check body is Boolean.
                            if hof == HofKind::Filter {
                                let second_hof_arg = extract_lambda_from_expr(second_arg)
                                    .map(HofSecondArg::Lambda)
                                    .unwrap_or(HofSecondArg::Other);
                                let result =
                                    infer_hof_call(hof, &lhs_ty, second_hof_arg, ctx, None);
                                if let Some(HofInferSentinel::LambdaResultTypeMismatch {
                                    expected,
                                    found,
                                }) = result.sentinel
                                {
                                    let exp_str = format_smelt_type_hover(&expected);
                                    let act_str = format_smelt_type_hover(&found);
                                    diags.push(crate::Diagnostic {
                                        severity: crate::DiagnosticSeverity::Error,
                                        message: crate::meta_hof_diagnostic_message(
                                            crate::DiagnosticCode::LambdaResultTypeMismatch,
                                            Some(&call_name),
                                            None,
                                            Some(&exp_str),
                                            Some(&act_str),
                                            None,
                                            None,
                                            None,
                                        ),
                                        range: call_range,
                                        code: Some(crate::DiagnosticCode::LambdaResultTypeMismatch),
                                        data: None,
                                    });
                                }
                            }

                            // (b) Walk the lambda body for type errors and stamp
                            // with an anonymous HOF expansion frame.
                            if let Some(lambda) = extract_lambda_from_expr(second_arg) {
                                let param_name =
                                    lambda.params().into_iter().next().unwrap_or_default();
                                let elem_ty = match &lhs_ty {
                                    SmeltType::List(inner) => match inner.as_ref() {
                                        SmeltType::Expr(
                                            smelt_types::signatures::TypeConstraint::Concrete(dt),
                                        ) => Some(smelt_types::TypedColumn {
                                            data_type: dt.clone(),
                                            nullable: false,
                                        }),
                                        _ => None,
                                    },
                                    _ => None,
                                };
                                if let (Some(elem), Some(body_expr)) = (elem_ty, lambda.body()) {
                                    let mut lambda_ctx = ctx.clone();
                                    lambda_ctx.add_lambda_param(&param_name, elem);
                                    // Pass `None` for `nested`: `check_hof_position_diagnostics`
                                    // is a top-level walker without a smelt.functions.* dispatch
                                    // handler. The `nested` handler is available only inside
                                    // `check_smelt_path_call` (function body expansion context).
                                    let body_diags = crate::function_body_check::walk_hof_lambda_body_with_anonymous_frame(
                                        &body_expr,
                                        &lambda_ctx,
                                        text,
                                        &call_name,
                                        Some(call_range),
                                        None,
                                    );
                                    diags.extend(body_diags);
                                }
                            }
                        }
                        // If has_any_lambda but !has_valid_lambda: it's a multi-arg lambda.
                        // The LAMBDA node walk above will emit LambdaArityMismatch.
                    }

                    HofKind::Reduce => {
                        if args.len() < 2 {
                            continue;
                        }
                        let second_arg = &args[1];

                        // Check for a parameterised REDUCER_CALL node (e.g. `concat_with(' OR ')`).
                        // These must be handled BEFORE the bare-identifier reducer check.
                        let reducer_call_node = second_arg
                            .syntax()
                            .descendants()
                            .find(|n| n.kind() == smelt_parser::SyntaxKind::REDUCER_CALL)
                            .or_else(|| {
                                if second_arg.syntax().kind()
                                    == smelt_parser::SyntaxKind::REDUCER_CALL
                                {
                                    Some(second_arg.syntax().clone())
                                } else {
                                    None
                                }
                            });

                        if let Some(reducer_node) = reducer_call_node {
                            if let Some(reducer_call) =
                                smelt_parser::ast::ReducerCall::cast(reducer_node.clone())
                            {
                                let reducer_name = reducer_call.name().unwrap_or_default();
                                // Run parameterised reducer inference.
                                let result = infer_parameterised_reducer_call(&reducer_call, ctx);
                                let rc_range = reducer_node.text_range();
                                if let Some(sentinel) = result.sentinel {
                                    match sentinel {
                                        ParameterisedReducerSentinel::ArityMismatch {
                                            reducer,
                                            expected,
                                            actual,
                                        } => {
                                            diags.push(crate::Diagnostic {
                                                severity: crate::DiagnosticSeverity::Error,
                                                message: crate::meta_hof_diagnostic_message(
                                                    crate::DiagnosticCode::ReducerArityMismatch,
                                                    None,
                                                    None,
                                                    Some(&expected.to_string()),
                                                    Some(&actual.to_string()),
                                                    Some(&reducer),
                                                    None,
                                                    None,
                                                ),
                                                range: rc_range,
                                                code: Some(
                                                    crate::DiagnosticCode::ReducerArityMismatch,
                                                ),
                                                data: None,
                                            });
                                        }
                                        ParameterisedReducerSentinel::NamedArgument => {
                                            diags.push(crate::Diagnostic {
                                                severity: crate::DiagnosticSeverity::Error,
                                                message: crate::meta_hof_diagnostic_message(
                                                    crate::DiagnosticCode::ReducerNamedArgument,
                                                    None,
                                                    None,
                                                    None,
                                                    None,
                                                    Some(&reducer_name),
                                                    None,
                                                    None,
                                                ),
                                                range: rc_range,
                                                code: Some(
                                                    crate::DiagnosticCode::ReducerNamedArgument,
                                                ),
                                                data: None,
                                            });
                                        }
                                        ParameterisedReducerSentinel::ArgTypeMismatch {
                                            reducer,
                                            param,
                                            expected,
                                            found,
                                        } => {
                                            let exp_str = format_smelt_type_hover(&expected);
                                            let found_str = format_smelt_type_hover(&found);
                                            diags.push(crate::Diagnostic {
                                                severity: crate::DiagnosticSeverity::Error,
                                                message: crate::meta_hof_diagnostic_message(
                                                    crate::DiagnosticCode::ReducerArgTypeMismatch,
                                                    None,
                                                    None,
                                                    Some(&exp_str),
                                                    None,
                                                    Some(&reducer),
                                                    Some(&param),
                                                    Some(&found_str),
                                                ),
                                                range: rc_range,
                                                code: Some(
                                                    crate::DiagnosticCode::ReducerArgTypeMismatch,
                                                ),
                                                data: None,
                                            });
                                        }
                                        ParameterisedReducerSentinel::ArgNotCompileTime {
                                            reducer,
                                            param,
                                            found,
                                        } => {
                                            let found_str = format_smelt_type_hover(&found);
                                            diags.push(crate::Diagnostic {
                                                severity: crate::DiagnosticSeverity::Error,
                                                message: crate::meta_hof_diagnostic_message(
                                                    crate::DiagnosticCode::ReducerArgNotCompileTime,
                                                    None,
                                                    None,
                                                    None,
                                                    None,
                                                    Some(&reducer),
                                                    Some(&param),
                                                    Some(&found_str),
                                                ),
                                                range: rc_range,
                                                code: Some(
                                                    crate::DiagnosticCode::ReducerArgNotCompileTime,
                                                ),
                                                data: None,
                                            });
                                        }
                                    }
                                }
                            }
                            // Skip the bare-identifier reducer check below.
                            continue;
                        }

                        // Second arg must be a bare reducer identifier.
                        let arg_text = second_arg.text().trim().to_string();
                        let is_reducer = arg_text.chars().all(|c| c.is_alphanumeric() || c == '_')
                            && !arg_text.is_empty()
                            && lookup_reducer(&arg_text).is_some();

                        // But is it a lambda? (wrong type for reduce)
                        let is_lambda = extract_lambda_from_expr(second_arg).is_some();

                        if is_lambda || !is_reducer {
                            diags.push(crate::Diagnostic {
                                severity: crate::DiagnosticSeverity::Error,
                                message: crate::meta_hof_diagnostic_message(
                                    crate::DiagnosticCode::HofExpectsReducer,
                                    None,
                                    None,
                                    None,
                                    Some(&arg_text),
                                    None,
                                    None,
                                    None,
                                ),
                                range: second_arg.text_range(),
                                code: Some(crate::DiagnosticCode::HofExpectsReducer),
                                data: None,
                            });
                        } else {
                            // Valid reducer name — check input type compatibility.
                            // Re-infer with actual element type.
                            let elem_ty = match &lhs_ty {
                                SmeltType::List(inner) => (**inner).clone(),
                                _ => SmeltType::Unknown,
                            };
                            let is_empty = matches!(&elem_ty, SmeltType::Unknown);
                            let result = infer_reduce_call(&elem_ty, is_empty, &arg_text, None);

                            match &result.sentinel {
                                Some(HofInferSentinel::ReducerInputTypeMismatch {
                                    reducer_name,
                                    expected_constraint,
                                    found,
                                }) => {
                                    let found_str = format_smelt_type_hover(found);
                                    diags.push(crate::Diagnostic {
                                        severity: crate::DiagnosticSeverity::Error,
                                        message: crate::meta_hof_diagnostic_message(
                                            crate::DiagnosticCode::ReducerInputTypeMismatch,
                                            None,
                                            None,
                                            None,
                                            None,
                                            Some(reducer_name),
                                            Some(expected_constraint),
                                            Some(&found_str),
                                        ),
                                        range: second_arg.text_range(),
                                        code: Some(crate::DiagnosticCode::ReducerInputTypeMismatch),
                                        data: None,
                                    });
                                }
                                Some(HofInferSentinel::ReducerEmptyNoIdentity { reducer_name }) => {
                                    diags.push(crate::Diagnostic {
                                        severity: crate::DiagnosticSeverity::Error,
                                        message: crate::meta_hof_diagnostic_message(
                                            crate::DiagnosticCode::ReducerEmptyNoIdentity,
                                            None,
                                            None,
                                            None,
                                            None,
                                            Some(reducer_name),
                                            None,
                                            None,
                                        ),
                                        range: call_range,
                                        code: Some(crate::DiagnosticCode::ReducerEmptyNoIdentity),
                                        data: None,
                                    });
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }

            // ── PIPE_EXPR node: validate RHS is a call + check data position ─
            PIPE_EXPR => {
                let pipe = match smelt_parser::ast::PipeExpr::cast(node.clone()) {
                    Some(p) => p,
                    None => continue,
                };

                // Check: is this pipe expression inside a Data-World grammar slot?
                // A pipe in a WHERE clause, JOIN condition, or HAVING clause
                // (any ancestor is a WHERE_CLAUSE node) emits PipeInDataPosition.
                let in_data_position = {
                    let mut parent_opt = node.parent();
                    let mut in_data = false;
                    while let Some(p) = parent_opt {
                        if p.kind() == smelt_parser::SyntaxKind::WHERE_CLAUSE {
                            in_data = true;
                            break;
                        }
                        // Stop at statement boundaries.
                        if p.kind() == smelt_parser::SyntaxKind::SELECT_STMT
                            || p.kind() == smelt_parser::SyntaxKind::FILE
                        {
                            break;
                        }
                        parent_opt = p.parent();
                    }
                    in_data
                };

                if in_data_position {
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_hof_diagnostic_message(
                            crate::DiagnosticCode::PipeInDataPosition,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                        ),
                        range: node.text_range(),
                        code: Some(crate::DiagnosticCode::PipeInDataPosition),
                        data: None,
                    });
                }

                if !pipe.rhs_is_call() {
                    let range = pipe
                        .rhs()
                        .map(|r| r.text_range())
                        .unwrap_or_else(|| node.text_range());
                    diags.push(crate::Diagnostic {
                        severity: crate::DiagnosticSeverity::Error,
                        message: crate::meta_hof_diagnostic_message(
                            crate::DiagnosticCode::PipeRhsNotCall,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                        ),
                        range,
                        code: Some(crate::DiagnosticCode::PipeRhsNotCall),
                        data: None,
                    });
                }
            }

            _ => {}
        }
    }

    diags
}

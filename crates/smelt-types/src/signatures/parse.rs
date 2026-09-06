use super::*;
use crate::parse_type;

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

    // Bare `TableExpr` (no `<...>`) is a valid row-polymorphic table
    // parameter (Phase 15). `TableExpr<{...}>` row requirements are
    // parsed by the signature extractor directly off the CST
    // (`extract_param_spec`) — the string-level parser here has no
    // structured grammar for row requirements, so it returns
    // `UnsupportedSort`, and the extractor overrides that with a
    // structured `SmeltType::TableExpr(Some(req))` when the CST
    // carries a `ROW_REQUIREMENT` (Phase 16).
    if trimmed == "TableExpr" {
        return Ok(SmeltType::TableExpr(None));
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
    if inner_raw.contains('<') {
        let inner_lt = inner_raw.find('<').unwrap();
        let inner_sort = inner_raw[..inner_lt].trim();
        // Also check for comma-prefixed sorts like ", Expr<...>" (second Lambda param).
        let actual_sort = inner_sort.trim_start_matches(',').trim();
        if sort == "Expr" && (actual_sort == "Expr") {
            return Err(SmeltTypeParseError::NestedExpr {
                span_text: text.to_string(),
            });
        }
        // `List<T>` and `Lambda<T, U>` allow nested generics — fall through to
        // sort dispatch so `List<Expr<Integer>>`, `Lambda<Expr<T>, Expr<U>>`, etc.
        // parse correctly.
        if sort == "List" || sort == "Lambda" || sort == "SelectItems" {
            // Fall through to the sort dispatch match below.
        } else if sort != "Expr" {
            // Other nested sort — surface the outer sort decision first.
            return Err(SmeltTypeParseError::UnsupportedSort {
                sort: sort.to_string(),
                span_text: text.to_string(),
            });
        } else {
            return Err(SmeltTypeParseError::UnknownInner {
                inner: inner_raw.to_string(),
                span_text: text.to_string(),
            });
        }
    }

    // Sort dispatch.
    match sort {
        "Expr" => {
            // Phase 19: strip optional `, ctx` context binding — the context
            // identifier is extracted from the CST in `extract_param_spec`.
            let type_part = inner_raw
                .find(',')
                .map(|i| inner_raw[..i].trim())
                .unwrap_or(inner_raw);
            let constraint = parse_inner_constraint(type_part).ok_or_else(|| {
                SmeltTypeParseError::UnknownInner {
                    inner: inner_raw.to_string(),
                    span_text: text.to_string(),
                }
            })?;
            Ok(SmeltType::Expr(constraint))
        }
        "List" => {
            // `List<T>` — recursive parse of the inner type.
            // `inner_raw` already has the leading `<` / trailing `>` stripped
            // (they were split off by the lt_idx / gt_idx logic above).
            let inner_ty =
                parse_smelt_type(inner_raw).map_err(|_| SmeltTypeParseError::UnknownInner {
                    inner: inner_raw.to_string(),
                    span_text: text.to_string(),
                })?;
            Ok(SmeltType::List(Box::new(inner_ty)))
        }
        "SelectItems" => {
            // `SelectItems<Kind>` or `SelectItems<Kind, ctx_name>` (Phase 21).
            let kind_part = inner_raw
                .find(',')
                .map(|i| inner_raw[..i].trim())
                .unwrap_or(inner_raw);
            let ctx_part = inner_raw
                .find(',')
                .map(|i| inner_raw[i + 1..].trim().to_string());
            let kind = match kind_part {
                "Agg" => ExprKind::Agg,
                "Scalar" => ExprKind::Scalar,
                "Window" => ExprKind::Window,
                _ => {
                    return Err(SmeltTypeParseError::UnknownInner {
                        inner: inner_raw.to_string(),
                        span_text: text.to_string(),
                    });
                }
            };
            let context = ctx_part.filter(|s| !s.is_empty()).map(ContextRef);
            Ok(SmeltType::SelectItems { kind, context })
        }
        "Lambda" => {
            // `Lambda<T, U>` — parse T and U as two comma-separated SmeltTypes.
            // `inner_raw` has outer `<>` stripped, e.g. "Expr<INTEGER>, Expr<TEXT>".
            // We need to split on the comma that separates the two type args,
            // respecting nested angle brackets.
            let split_pos =
                find_lambda_comma(inner_raw).ok_or_else(|| SmeltTypeParseError::Malformed {
                    span_text: text.to_string(),
                })?;
            let t_raw = inner_raw[..split_pos].trim();
            let u_raw = inner_raw[split_pos + 1..].trim();
            let t = parse_smelt_type(t_raw).map_err(|_| SmeltTypeParseError::UnknownInner {
                inner: t_raw.to_string(),
                span_text: text.to_string(),
            })?;
            let u = parse_smelt_type(u_raw).map_err(|_| SmeltTypeParseError::UnknownInner {
                inner: u_raw.to_string(),
                span_text: text.to_string(),
            })?;
            Ok(SmeltType::Lambda(vec![t], Box::new(u)))
        }
        "TableExpr" | "AggExpr" | "WindowExpr" | "OrderSpec" => {
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

/// Find the position of the top-level comma separating the two type parameters
/// in a `Lambda<T, U>` inner string (after the outer `<>` are stripped).
///
/// Respects nested angle brackets so `Lambda<Expr<Integer>, Expr<Text>>` correctly
/// splits at the comma between the two top-level parameters, not at a nested one.
fn find_lambda_comma(inner: &str) -> Option<usize> {
    let mut depth: i32 = 0;
    for (i, c) in inner.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// Recognise the inner payload of an `Expr<...>`.
///
/// The order matters: constraint keywords (`Numeric`, `Any`) take priority over
/// concrete type names so a future `Ordered` constraint can live alongside
/// whatever `parse_type` might try to map to a concrete type.
pub(super) fn parse_inner_constraint(inner: &str) -> Option<TypeConstraint> {
    match inner {
        "Numeric" => Some(TypeConstraint::Numeric),
        "Ordered" => Some(TypeConstraint::Ordered),
        "Any" => Some(TypeConstraint::Any),
        other => parse_type(other).ok().map(TypeConstraint::Concrete),
    }
}

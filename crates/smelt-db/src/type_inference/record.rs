//! Meta-Text-as-identifier lift, record literal/projection inference, Map<T,V> method inference.

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

// ─── Phase C Phase 2: meta-Text-as-identifier lift (narrow rule) ─────────────

/// The four grammar positions where a compile-time meta-`Text` value may lift
/// to an unquoted SQL identifier (Phase C §"Meta-`Text`-as-identifier lift").
///
/// Every other position (function-argument where the parameter sort is
/// `Expr<Text>`, comparison operands, named-argument values, etc.) is NOT a
/// lift position; in those positions a meta-`Text` retains its string-value
/// meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaTextLiftPosition {
    /// `SELECT c.name FROM t` — the expression itself is in a SELECT-list
    /// slot where the grammar expects a column reference.  Lift fires when
    /// the whole select-item expression is a meta-`Text` value with no
    /// explicit `AS` alias.
    ColumnReference,
    /// `AS <meta-Text>` alias of a SELECT item.  Lift fires and treats the
    /// meta-`Text` value as the output column identifier.  No scope check —
    /// aliases introduce names, they do not reference existing ones.
    AsAlias,
    /// `ORDER BY <meta-Text>` — the expression is the sort key.  Lift fires;
    /// the lifted identifier is validated against the surrounding
    /// column-resolution scope.
    OrderBy,
    /// `GROUP BY <meta-Text>` — the expression is a grouping key.  Same
    /// scope rule as `OrderBy`.
    GroupBy,
}

impl MetaTextLiftPosition {
    /// Returns `true` when the lifted identifier must be validated against the
    /// surrounding column-resolution scope (i.e., `UnknownColumn` is possible).
    ///
    /// `AsAlias` returns `false` — aliases introduce names, not reference them.
    pub fn requires_scope_validation(self) -> bool {
        !matches!(self, MetaTextLiftPosition::AsAlias)
    }
}

/// Returns the lifted identifier text if `expr` is a compile-time
/// meta-`Text` value — specifically, a `ColumnRef.name` field projection
/// whose binding is registered as `SmeltType::ColumnRef` in `ctx`.
///
/// In Phase C the only producer of compile-time meta-`Text` values is a
/// `<binding>.name` field access where `<binding>` was declared as
/// `ColumnRef` (e.g. the lambda parameter `c` in `map(smelt.columns_of(t),
/// fn c => ...)`) .  All other expressions — including runtime `Expr<Text>`
/// results like `UPPER('foo')` and SQL string literals like `'foo'` — return
/// `None`.
///
/// When `Some(text)` is returned, `text` is the field-name token in the
/// source — i.e. the literal identifier `"name"`.  The actual runtime value
/// of `c.name` (the column name string at expansion time) is determined by
/// Phase 3's expansion-time materialisation; Phase 2 only recognises the
/// structural pattern.
///
/// Pure — no Salsa dependency.
pub fn is_meta_text_value(expr: &Expr, ctx: &TypeContext) -> Option<String> {
    use smelt_types::signatures::SmeltType;

    // Only a bare qualified column-ref of the form `qualifier.field` can be a
    // meta-Text value in Phase C; complex expressions (function calls, binary
    // expressions, literals) are all runtime values.
    let col_ref = smelt_parser::ast::ColumnRef::from_expr(expr)?;
    let qualifier = col_ref.qualifier()?;
    let field = col_ref.name();

    // Is the qualifier registered as a ColumnRef-typed binding?
    let is_column_ref_binding = ctx
        .lookup_function_param_smelt_type(qualifier)
        .map(|ty| matches!(ty, SmeltType::ColumnRef))
        .unwrap_or(false);

    if !is_column_ref_binding {
        return None;
    }

    // Is the field the Text-typed `name` field (the only Text-typed member of
    // the closed ColumnRef field set)?  Other fields (`type` → Unknown;
    // `is_numeric`, `is_decimal`, `is_string`, `is_temporal`, `is_integer`,
    // `is_boolean` → Boolean) are NOT meta-Text and do not lift.
    use smelt_types::signatures::{column_ref_field, TypeConstraint};
    let field_ty = column_ref_field(field)?;
    let is_text_field = matches!(
        field_ty,
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
    );
    if !is_text_field {
        return None;
    }

    // Return the field name token as the lifted identifier text.  `name` is
    // the only Text-typed ColumnRef field; future Text-typed additions would
    // be handled here automatically.
    Some(field.to_string())
}

/// Check all four meta-`Text`-as-identifier lift positions in a SELECT
/// statement and return `UndeclaredColumnInfo` diagnostics for any lifted
/// identifier that names no in-scope column.
///
/// Lift positions checked (§"Meta-`Text`-as-identifier lift"):
/// 1. **Column-reference position** — every expression in the SELECT list
///    that IS itself a meta-`Text` value (i.e. no sub-expressions; the bare
///    `c.name` is the entire select-item expression).
/// 2. **ORDER BY column-reference** — every sort-key expression that is a
///    meta-`Text` value.
/// 3. **GROUP BY column-reference** — every grouping-key expression that is
///    a meta-`Text` value.
/// 4. **AS alias** — detected by inspecting select items whose expression is
///    a meta-`Text` value; no scope validation is performed for aliases
///    (aliases introduce names, not reference them).
///
/// **Body-check-time scope validation is suppressed for all four positions.**
/// `is_meta_text_value` returns the *field-name token* (e.g. `"name"`), not
/// the per-element column name that `c.name` will evaluate to at expansion
/// time.  Validating the field-name token against in-scope columns would
/// produce false positives whenever no column literally named `"name"` exists
/// in the body context (almost always), and would mask real errors when one
/// happens to exist by accident.  Per §Semantics rule 6, lift-scope validation
/// is correctly located at expansion time, after the per-element column name is
/// known.  This function therefore recognises the structural lift pattern and
/// returns an empty `Vec`; expansion-time validation is handled elsewhere.
///
/// Expressions that are NOT meta-`Text` values are silently skipped; they
/// continue to be validated by `check_undeclared_columns` through the normal
/// path.
///
/// Pure — no Salsa dependency.
pub fn check_meta_text_lift_diagnostics(
    _select_stmt: &smelt_parser::ast::SelectStmt,
    _ctx: &TypeContext,
) -> Vec<UndeclaredColumnInfo> {
    // Body-check-time scope validation is suppressed: the field-name token
    // returned by is_meta_text_value (always "name" for the Text-typed
    // ColumnRef field) is not the per-element column name that the lift
    // produces at expansion time.  Expansion-time validation is correct;
    // body-check-time validation against this token is not.
    Vec::new()
}

// ─── Phase E1 Record inference (Phase 3) ──────────────────────────────────────

/// Diagnostic emitted by the record literal checker (Phase E1 Phase 3).
///
/// Each sentinel carries the [`crate::DiagnosticCode`] variant, an anchoring
/// [`TextRange`], and a pre-rendered message. The orchestrating layer in
/// `smelt-db::lib.rs` converts these into [`crate::Diagnostic`] values.
#[derive(Debug, Clone)]
pub struct RecordLiteralSentinel {
    /// The diagnostic code identifying which rule fired.
    pub code: crate::DiagnosticCode,
    /// Source span of the offending token or position.
    pub span: TextRange,
    /// Human-readable message per the spec's message format.
    pub message: String,
}

/// Result of type-checking a record literal (Phase E1 Phase 3).
#[derive(Debug)]
pub struct RecordLiteralResult {
    /// The synthesised type. On success, `Record{name: Some("TypeName"), fields: ...}`.
    /// On `RecordLiteralUnknownTarget`, `Record<Unknown>` (i.e. `Record { name: None,
    /// fields: BTreeMap::new() }`). On partial error, the declared type with `Unknown`
    /// for failed fields (drop-on-error).
    pub inferred: SmeltType,
    /// All diagnostics emitted during checking (0 on happy path).
    pub sentinels: Vec<RecordLiteralSentinel>,
}

/// Check a `RecordLiteral` node against a named target type from the workspace
/// registry (Phase E1 Phase 3).
///
/// Implements the bidirectional checking algorithm from spec §Semantics rules 5–6:
/// - Required field check: each field declared in the target must appear in the
///   literal exactly once (`RecordFieldMissing` at closing brace).
/// - Unknown field check: each field in the literal not declared in the target
///   emits `RecordFieldUnknown` at the field-name token; the field is dropped.
/// - Duplicate field check: a field name appearing twice emits `RecordFieldDuplicate`
///   at the second occurrence; the duplicate is dropped.
/// - Type mismatch check: a field value whose inferred type is not assignable to
///   the declared field type emits `RecordFieldTypeMismatch` at the value expression;
///   the field carries `Unknown` (drop-on-error).
/// - No target: emits `RecordLiteralUnknownTarget` at the opening brace.
///
/// Pure — no Salsa dependency. Pass `""` for `text` in unit tests where exact
/// span positions are not under test.
pub fn check_record_literal(
    lit: &smelt_parser::ast::RecordLiteral,
    ctx: &TypeContext,
    target_type: Option<&SmeltType>,
    _text: &str,
) -> RecordLiteralResult {
    use smelt_parser::SyntaxKind::RBRACE;
    use std::collections::BTreeMap;

    let mut sentinels: Vec<RecordLiteralSentinel> = Vec::new();

    // Resolve target to a Record declaration (named or inline fields).
    let target_record = match target_type {
        None => {
            // No inferable target — emit RecordLiteralUnknownTarget at the opening brace.
            // The opening brace is the first token of the node.
            let open_brace_range = lit
                .syntax()
                .children_with_tokens()
                .find_map(|e| {
                    let tok = e.into_token()?;
                    if tok.kind() == smelt_parser::SyntaxKind::LBRACE {
                        Some(tok.text_range())
                    } else {
                        None
                    }
                })
                .unwrap_or(lit.syntax().text_range());
            sentinels.push(RecordLiteralSentinel {
                code: crate::DiagnosticCode::RecordLiteralUnknownTarget,
                span: open_brace_range,
                message: crate::meta_record_diagnostic_message(
                    crate::DiagnosticCode::RecordLiteralUnknownTarget,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                ),
            });
            return RecordLiteralResult {
                inferred: SmeltType::Record {
                    fields: BTreeMap::new(),
                    name: None,
                },
                sentinels,
            };
        }
        Some(SmeltType::Record { fields, name }) => {
            // Use these fields and name directly.
            (fields.clone(), name.clone())
        }
        Some(other) => {
            // Target is not a record type — treat as unknown target.
            let open_brace_range = lit
                .syntax()
                .children_with_tokens()
                .find_map(|e| {
                    let tok = e.into_token()?;
                    if tok.kind() == smelt_parser::SyntaxKind::LBRACE {
                        Some(tok.text_range())
                    } else {
                        None
                    }
                })
                .unwrap_or(lit.syntax().text_range());
            sentinels.push(RecordLiteralSentinel {
                code: crate::DiagnosticCode::RecordLiteralUnknownTarget,
                span: open_brace_range,
                message: format!(
                    "cannot infer record type from context (target is {}); annotate the target type",
                    other
                ),
            });
            return RecordLiteralResult {
                inferred: SmeltType::Record {
                    fields: BTreeMap::new(),
                    name: None,
                },
                sentinels,
            };
        }
    };

    let (declared_fields, record_name) = target_record;
    let type_display = record_name.as_deref().unwrap_or("record").to_string();

    // Build a sorted display of declared field names for diagnostic messages.
    let declared_fields_list: Vec<String> = declared_fields.keys().cloned().collect();
    let declared_fields_str = declared_fields_list.join(", ");

    // Walk literal fields left-to-right, collecting results.
    let mut seen_names: HashMap<String, ()> = HashMap::new();
    let mut provided: HashMap<String, SmeltType> = HashMap::new();

    for field in lit.fields() {
        let Some(field_name) = field.name() else {
            continue;
        };

        // Find the name token span (for unknown/duplicate anchoring).
        let name_span = field
            .syntax()
            .children_with_tokens()
            .find_map(|e| {
                let tok = e.into_token()?;
                if tok.kind() == smelt_parser::SyntaxKind::IDENT {
                    Some(tok.text_range())
                } else {
                    None
                }
            })
            .unwrap_or(field.syntax().text_range());

        // Duplicate check.
        if seen_names.contains_key(&field_name) {
            sentinels.push(RecordLiteralSentinel {
                code: crate::DiagnosticCode::RecordFieldDuplicate,
                span: name_span,
                message: crate::meta_record_diagnostic_message(
                    crate::DiagnosticCode::RecordFieldDuplicate,
                    None,
                    Some(&field_name),
                    None,
                    None,
                    None,
                    None,
                ),
            });
            continue; // drop duplicate
        }
        seen_names.insert(field_name.clone(), ());

        // Unknown field check.
        if !declared_fields.contains_key(&field_name) {
            // Find the closest valid field names for the message.
            sentinels.push(RecordLiteralSentinel {
                code: crate::DiagnosticCode::RecordFieldUnknown,
                span: name_span,
                message: crate::meta_record_diagnostic_message(
                    crate::DiagnosticCode::RecordFieldUnknown,
                    Some(&type_display),
                    Some(&field_name),
                    None,
                    None,
                    None,
                    Some(&declared_fields_str),
                ),
            });
            continue; // drop unknown field
        }

        let declared_ty = &declared_fields[&field_name];

        // Type-check the value expression.
        let field_ty = if let Some(value_expr) = field.value_expr() {
            let inferred = infer_expression_type(&value_expr, ctx);
            let inferred_ty = match &inferred {
                Some(tc) => SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                    tc.data_type.clone(),
                )),
                None => SmeltType::Unknown,
            };

            // Check assignability: inferred_ty <: declared_ty.
            if !smelt_types::signatures::is_subtype_of(&inferred_ty, declared_ty) {
                // For concrete Expr types, also check DataType compatibility.
                let compatible = match (&inferred_ty, declared_ty) {
                    (
                        SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                            found_dt,
                        )),
                        SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(exp_dt)),
                    ) => types_assignable(found_dt, exp_dt),
                    _ => false,
                };

                if !compatible {
                    let found_str = format!("{}", inferred_ty);
                    let exp_str = format!("{}", declared_ty);
                    let value_span = value_expr.syntax().text_range();
                    sentinels.push(RecordLiteralSentinel {
                        code: crate::DiagnosticCode::RecordFieldTypeMismatch,
                        span: value_span,
                        message: crate::meta_record_diagnostic_message(
                            crate::DiagnosticCode::RecordFieldTypeMismatch,
                            Some(&type_display),
                            Some(&field_name),
                            None,
                            Some(&exp_str),
                            Some(&found_str),
                            None,
                        ),
                    });
                    provided.insert(field_name, SmeltType::Unknown); // drop-on-error
                    continue;
                }
            }
            declared_ty.clone()
        } else {
            SmeltType::Unknown
        };

        provided.insert(field_name, field_ty);
    }

    // Missing field check: every declared field must appear in `provided`.
    // Anchor at the closing brace.
    let close_brace_range = lit
        .syntax()
        .children_with_tokens()
        .filter_map(|e| {
            let tok = e.into_token()?;
            if tok.kind() == RBRACE {
                Some(tok.text_range())
            } else {
                None
            }
        })
        .last()
        .unwrap_or(lit.syntax().text_range());

    let mut missing_fields: Vec<String> = declared_fields
        .keys()
        .filter(|k| !provided.contains_key(*k) && !seen_names.contains_key(*k))
        .cloned()
        .collect();
    missing_fields.sort();

    for missing in &missing_fields {
        sentinels.push(RecordLiteralSentinel {
            code: crate::DiagnosticCode::RecordFieldMissing,
            span: close_brace_range,
            message: crate::meta_record_diagnostic_message(
                crate::DiagnosticCode::RecordFieldMissing,
                Some(&type_display),
                Some(missing),
                None,
                None,
                None,
                None,
            ),
        });
    }

    // Build the synthesised type: use declared fields, substituting Unknown for errors.
    let mut result_fields: BTreeMap<String, SmeltType> = BTreeMap::new();
    for k in declared_fields.keys() {
        let field_ty = provided.get(k).cloned().unwrap_or(SmeltType::Unknown);
        result_fields.insert(k.clone(), field_ty);
    }

    RecordLiteralResult {
        inferred: SmeltType::Record {
            fields: result_fields,
            name: record_name,
        },
        sentinels,
    }
}

/// Check whether `found` is assignable to `expected` for record field
/// type-checking. Equality, Text/Varchar interop, and widening-only numeric
/// promotion per `docs/specs/types.md §"Numeric promotion chain"`.
fn types_assignable(found: &DataType, expected: &DataType) -> bool {
    if found == expected {
        return true;
    }
    let found_is_text = matches!(found, DataType::Text | DataType::Varchar { .. });
    let expected_is_text = matches!(expected, DataType::Text | DataType::Varchar { .. });
    if found_is_text && expected_is_text {
        return true;
    }
    if found.is_numeric() && expected.is_numeric() {
        // Widening-only: found must fit inside expected per the promotion chain.
        return numeric_rank(found).is_some_and(|f| numeric_rank(expected).is_some_and(|e| f <= e));
    }
    false
}

/// Position in the numeric promotion chain. Lower rank fits inside higher rank.
/// SmallInt < Integer < BigInt < Float < Double < Decimal. Decimal precision
/// widening is deferred (see `types.md §Known Divergences`).
fn numeric_rank(t: &DataType) -> Option<u8> {
    match t {
        DataType::SmallInt => Some(0),
        DataType::Integer => Some(1),
        DataType::BigInt => Some(2),
        DataType::Float => Some(3),
        DataType::Double => Some(4),
        DataType::Decimal { .. } => Some(5),
        _ => None,
    }
}

/// Result of field-projection inference on a record-typed value (Phase E1 Phase 3).
#[derive(Debug)]
pub struct RecordFieldProjectionResult {
    /// The synthesised field type, or `Unknown` if the field was not found or
    /// the receiver was not projectable.
    pub inferred: SmeltType,
    /// Diagnostics emitted (0 on happy path).
    pub sentinels: Vec<RecordLiteralSentinel>,
}

/// Infer the type of `<binding_name>.<field_name>` where `binding_name` is
/// registered in `ctx` as a `Record<…>` type (Phase E1 Phase 3).
///
/// Rules per spec §Semantics rule 7:
/// - If the receiver is `Record{...}`, look up `field_name` in the declared
///   field set. If found, return the field type. If not found, emit
///   `RecordFieldUnknown` at `field_span` and return `Unknown`.
/// - If the receiver is NOT a record type, emit `RecordFieldNotProjectable` at
///   `field_span` and return `Unknown`.
/// - If `binding_name` resolves to a named record (via the registry), the field
///   set is the declaration's closed set. For width-subtyping rule 11: the
///   declared static type governs projections — a wider runtime type doesn't
///   expand the closed set.
///
/// Pure — no Salsa dependency.
pub fn infer_record_field_projection(
    receiver_type: &SmeltType,
    field_name: &str,
    field_span: TextRange,
    _text: &str,
) -> RecordFieldProjectionResult {
    let mut sentinels = Vec::new();

    match receiver_type {
        SmeltType::Record { fields, name } => {
            if let Some(field_ty) = fields.get(field_name) {
                RecordFieldProjectionResult {
                    inferred: field_ty.clone(),
                    sentinels,
                }
            } else {
                // RecordFieldUnknown at the field token.
                let type_display = name.as_deref().unwrap_or("record");
                let mut field_list: Vec<String> = fields.keys().cloned().collect();
                field_list.sort();
                let fields_str = field_list.join(", ");
                sentinels.push(RecordLiteralSentinel {
                    code: crate::DiagnosticCode::RecordFieldUnknown,
                    span: field_span,
                    message: crate::meta_record_diagnostic_message(
                        crate::DiagnosticCode::RecordFieldUnknown,
                        Some(type_display),
                        Some(field_name),
                        None,
                        None,
                        None,
                        Some(&fields_str),
                    ),
                });
                RecordFieldProjectionResult {
                    inferred: SmeltType::Unknown,
                    sentinels,
                }
            }
        }
        other => {
            // RecordFieldNotProjectable at the field token.
            let type_display = format!("{}", other);
            sentinels.push(RecordLiteralSentinel {
                code: crate::DiagnosticCode::RecordFieldNotProjectable,
                span: field_span,
                message: crate::meta_record_diagnostic_message(
                    crate::DiagnosticCode::RecordFieldNotProjectable,
                    Some(&type_display),
                    Some(field_name),
                    None,
                    None,
                    None,
                    None,
                ),
            });
            RecordFieldProjectionResult {
                inferred: SmeltType::Unknown,
                sentinels,
            }
        }
    }
}

/// Check whether a record-typed value is being referenced in a Data-World
/// (SQL) position (non-splice) — emits `RecordInDataWorld` (Phase E1 Phase 3).
///
/// Per spec §Semantics rule 10: a record value never reaches the database
/// engine. A bare record-typed binding reference at a non-splice SQL position
/// emits this diagnostic. Field projections that produce a non-record type
/// (e.g. `c.name` → `Text`) do NOT emit this diagnostic — the projection
/// exits into Data-World via the field's type.
///
/// `is_splice_context` should be `true` when the caller is inside a
/// meta-language splice point (e.g. a HOF argument, a `smelt.fn` argument).
/// When `false`, and `receiver_type` is `Record{…}`, the diagnostic fires.
///
/// Pure — no Salsa dependency.
pub fn check_record_in_data_world(
    receiver_type: &SmeltType,
    reference_span: TextRange,
    is_splice_context: bool,
    _text: &str,
) -> Option<RecordLiteralSentinel> {
    if is_splice_context {
        return None;
    }
    if matches!(receiver_type, SmeltType::Record { .. }) {
        let message = crate::meta_record_diagnostic_message(
            crate::DiagnosticCode::RecordInDataWorld,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        Some(RecordLiteralSentinel {
            code: crate::DiagnosticCode::RecordInDataWorld,
            span: reference_span,
            message,
        })
    } else {
        None
    }
}

/// Build the workspace `RecordRegistry` from a list of `SmeltRecordDeclaration`s.
///
/// This is the pure builder function used in Phase 3 tests and the Phase 5
/// Salsa wrapper. It delegates directly to
/// [`smelt_types::signatures::build_record_registry`] and returns both the
/// registry and diagnostic sentinels.
///
/// **Note:** Phase 5 will call this from a Salsa tracked function. Phase 3
/// callers (pure tests) call it directly with a hand-built declaration list.
///
/// Pure — no Salsa dependency, no I/O.
pub fn record_registry_for_workspace(
    decls: &[smelt_types::signatures::SmeltRecordDeclaration],
) -> (
    smelt_types::signatures::RecordRegistry,
    Vec<smelt_types::signatures::DiagnosticSentinel>,
) {
    smelt_types::signatures::build_record_registry(decls)
}

/// Convert a `RecordRegistryCode` sentinel from `build_record_registry` into a
/// `crate::DiagnosticCode` for the LSP accumulator.
///
/// Called by the Phase 5 Salsa orchestration layer when wiring registry
/// sentinels into `file_diagnostics`.
///
/// Pure — no Salsa dependency.
pub fn registry_code_to_diagnostic_code(
    code: smelt_types::signatures::RecordRegistryCode,
) -> crate::DiagnosticCode {
    use smelt_types::signatures::RecordRegistryCode;
    match code {
        RecordRegistryCode::SmeltRecordRedefinition => {
            crate::DiagnosticCode::SmeltRecordRedefinition
        }
        RecordRegistryCode::RecordFieldTypeForbidden => {
            crate::DiagnosticCode::RecordFieldTypeForbidden
        }
        RecordRegistryCode::RecordCyclicDeclaration => {
            crate::DiagnosticCode::RecordCyclicDeclaration
        }
    }
}

// ============================================================================
// Map<K,V> API dispatch — type inference (Phase E1 Phase 4)
// ============================================================================

/// A single argument to a Map API method call, as seen by the pure type
/// inference layer.
///
/// Callers (Salsa wrappers and unit tests) pre-process the AST arguments into
/// this data-only representation so that `infer_map_method_call` remains purely
/// functional with no AST / Salsa dependency.
#[derive(Debug, Clone)]
pub enum MapCallArg {
    /// A positional argument.
    Positional {
        /// The synthesised type of the argument expression.
        ty: SmeltType,
        /// `Some(s)` when the argument is a string literal with value `s`.
        /// Used to enable statically-known-key resolution at `m.get(k)` /
        /// `m.has(k)`. `None` means the key is not statically known at
        /// type-check time (evaluation deferred to expansion time).
        literal_value: Option<String>,
    },
    /// A named argument (`param => value`). Named arguments are never
    /// permitted on any Map API method; the caller surfaces one `MapCallArg::Named`
    /// per named argument so the function can emit `MapApiNamedArgument` for each.
    Named {
        /// The parameter name as written in the source.
        param_name: String,
        /// The synthesised type of the value expression.
        ty: SmeltType,
    },
}

/// Discriminates whether a Map API method call was resolved at type-check
/// time (statically) or deferred to expansion time.
///
/// This is carried in `MapMethodCallResult::static_resolution` so tests (and
/// future phases) can assert that the static-resolution path was taken rather
/// than the generic formula fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaticResolution {
    /// Static-key lookup (`get`) found the key present in the bound contents.
    /// The value type returned is the *per-entry* type from `contents`, which
    /// may be narrower than the declared `V`.
    Present,
    /// Static-key lookup (`get`) found the key absent from the bound contents.
    /// A `MapGetMissingKey` diagnostic is emitted; `inferred` is `Unknown`.
    Absent,
    /// Static-key presence check (`has`) resolved to a known Boolean value.
    /// `true` → key is in `contents`; `false` → key is absent.
    Bool(bool),
    /// The call was not resolved statically (non-literal key or unbound contents).
    Deferred,
}

/// Result of type-checking a Map API method call (Phase E1 Phase 4).
#[derive(Debug)]
pub struct MapMethodCallResult {
    /// The synthesised return type.
    ///
    /// On happy path: the formula from `MAP_API_METHODS[method].return_type_formula(K, V)`.
    /// On any error: `SmeltType::Unknown` (drop-on-error semantics).
    pub inferred: SmeltType,
    /// Diagnostics emitted (0 on happy path).
    pub sentinels: Vec<RecordLiteralSentinel>,
    /// Whether static-key resolution was performed.
    ///
    /// `Deferred` for all zero-arg methods and for keyed methods when the key
    /// is not a literal or when `map_contents` is `None`.
    pub static_resolution: StaticResolution,
}

/// Type-check a `Map<K,V>` method call and synthesise its return type.
///
/// # Arguments
///
/// * `receiver_type` — the `SmeltType` of the receiver expression. Must be
///   `SmeltType::Map { key, value }` (invariant) or any other type (the latter
///   is a caller error; this function only handles Map receivers — non-Map
///   receivers are routed by the surrounding dispatch in `infer_expression_type`
///   to `infer_record_field_projection` or similar).
/// * `method_name` — the name token of the method as written in the source.
/// * `args` — pre-processed argument list. Named args are represented as
///   `MapCallArg::Named`; positional args as `MapCallArg::Positional`.
/// * `map_contents` — `Some(contents)` when the Map's key-value bindings are
///   fully resolved at type-check time (e.g. from a loader). `None` when the
///   Map's contents are not statically known (defers key resolution to expansion
///   time). Only consulted for `m.get(k)` and `m.has(k)` when `k` is a string
///   literal.
/// * `call_span` — the text range of the entire call expression, used to anchor
///   diagnostics.
///
/// # Purity
///
/// Pure — no Salsa dependency. Anchors diagnostics at `call_span`.
pub fn infer_map_method_call(
    receiver_type: &SmeltType,
    method_name: &str,
    args: &[MapCallArg],
    map_contents: Option<&std::collections::BTreeMap<String, SmeltType>>,
    call_span: TextRange,
) -> MapMethodCallResult {
    use smelt_types::signatures::{is_subtype_of, lookup_map_api_method, Arity, MapApiMethodKind};

    let mut sentinels: Vec<RecordLiteralSentinel> = Vec::new();

    // Extract key/value types from receiver. `receiver_type` must be Map<K,V>.
    let (key_ty, value_ty) = match receiver_type {
        SmeltType::Map { key, value } => (key.as_ref(), value.as_ref()),
        _ => {
            // Caller routing error — should not happen in correct dispatch.
            // Treat as unknown method to avoid panicking.
            sentinels.push(RecordLiteralSentinel {
                code: crate::DiagnosticCode::MapApiUnknown,
                span: call_span,
                message: crate::meta_map_diagnostic_message(
                    crate::DiagnosticCode::MapApiUnknown,
                    None,
                    Some(method_name),
                    None,
                    None,
                    None,
                    None,
                    None,
                ),
            });
            return MapMethodCallResult {
                inferred: SmeltType::Unknown,
                sentinels,
                static_resolution: StaticResolution::Deferred,
            };
        }
    };

    // Check for named arguments — never permitted on any Map API method.
    // Emit one MapApiNamedArgument per named arg (positional args are processed
    // separately). Named args short-circuit further checks.
    let has_named = args.iter().any(|a| matches!(a, MapCallArg::Named { .. }));
    if has_named {
        for a in args {
            if let MapCallArg::Named { .. } = a {
                sentinels.push(RecordLiteralSentinel {
                    code: crate::DiagnosticCode::MapApiNamedArgument,
                    span: call_span,
                    message: crate::meta_map_diagnostic_message(
                        crate::DiagnosticCode::MapApiNamedArgument,
                        Some(method_name),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    ),
                });
            }
        }
        return MapMethodCallResult {
            inferred: SmeltType::Unknown,
            sentinels,
            static_resolution: StaticResolution::Deferred,
        };
    }

    // Collect positional args (no named args at this point).
    let pos_args: Vec<&MapCallArg> = args
        .iter()
        .filter(|a| matches!(a, MapCallArg::Positional { .. }))
        .collect();

    // Look up the method in the closed registry.
    let Some(method) = lookup_map_api_method(method_name) else {
        // Unknown method — emit MapApiUnknown anchored at call_span.
        sentinels.push(RecordLiteralSentinel {
            code: crate::DiagnosticCode::MapApiUnknown,
            span: call_span,
            message: crate::meta_map_diagnostic_message(
                crate::DiagnosticCode::MapApiUnknown,
                None,
                Some(method_name),
                None,
                None,
                None,
                None,
                None,
            ),
        });
        return MapMethodCallResult {
            inferred: SmeltType::Unknown,
            sentinels,
            static_resolution: StaticResolution::Deferred,
        };
    };

    // Arity check.
    let Arity::Exact(expected_arity) = method.arity;
    let actual_arity = pos_args.len();

    if actual_arity != expected_arity {
        // Arity mismatch: for zero-arity methods (entries/keys/values), any
        // positional argument emits MapApiUnexpectedArgument; for one-arity
        // methods (get/has) with wrong count, emit MapApiArityMismatch.
        if expected_arity == 0 {
            sentinels.push(RecordLiteralSentinel {
                code: crate::DiagnosticCode::MapApiUnexpectedArgument,
                span: call_span,
                message: crate::meta_map_diagnostic_message(
                    crate::DiagnosticCode::MapApiUnexpectedArgument,
                    Some(method_name),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                ),
            });
        } else {
            sentinels.push(RecordLiteralSentinel {
                code: crate::DiagnosticCode::MapApiArityMismatch,
                span: call_span,
                message: crate::meta_map_diagnostic_message(
                    crate::DiagnosticCode::MapApiArityMismatch,
                    Some(method_name),
                    None,
                    None,
                    Some(&actual_arity.to_string()),
                    None,
                    None,
                    None,
                ),
            });
        }
        return MapMethodCallResult {
            inferred: SmeltType::Unknown,
            sentinels,
            static_resolution: StaticResolution::Deferred,
        };
    }

    // For keyed-lookup (`get`) and keyed-presence (`has`) methods: validate the
    // key argument type and perform static-key resolution when possible.
    // The dispatch is driven by `method.kind` — no string-literal comparisons.
    if matches!(
        method.kind,
        MapApiMethodKind::KeyedGet | MapApiMethodKind::KeyedHas
    ) {
        // Exactly one positional arg (arity already validated above).
        let arg = pos_args[0];
        let MapCallArg::Positional {
            ty: arg_ty,
            literal_value,
        } = arg
        else {
            unreachable!("named args are handled above")
        };

        // Key type check: arg must be assignable to K.
        if !is_subtype_of(arg_ty, key_ty) {
            let expected_str = format!("{key_ty}");
            let actual_str = format!("{arg_ty}");
            sentinels.push(RecordLiteralSentinel {
                code: crate::DiagnosticCode::MapApiArgTypeMismatch,
                span: call_span,
                message: crate::meta_map_diagnostic_message(
                    crate::DiagnosticCode::MapApiArgTypeMismatch,
                    Some(method_name),
                    None,
                    None,
                    None,
                    None,
                    Some(&expected_str),
                    Some(&actual_str),
                ),
            });
            return MapMethodCallResult {
                inferred: SmeltType::Unknown,
                sentinels,
                static_resolution: StaticResolution::Deferred,
            };
        }

        // Static-key resolution: only when both contents are bound AND the arg is a string literal.
        if let (Some(contents), Some(key_str)) = (map_contents, literal_value.as_deref()) {
            if matches!(method.kind, MapApiMethodKind::KeyedGet) {
                // Statically-known key: look it up in the bound contents.
                if contents.contains_key(key_str) {
                    // Present → synthesise the per-entry type from `contents`
                    // (may be narrower than the declared `V`).
                    let resolved_ty = contents
                        .get(key_str)
                        .cloned()
                        .unwrap_or_else(|| value_ty.clone());
                    return MapMethodCallResult {
                        inferred: resolved_ty,
                        sentinels,
                        static_resolution: StaticResolution::Present,
                    };
                } else {
                    // Absent → MapGetMissingKey + Unknown.
                    sentinels.push(RecordLiteralSentinel {
                        code: crate::DiagnosticCode::MapGetMissingKey,
                        span: call_span,
                        message: crate::meta_map_diagnostic_message(
                            crate::DiagnosticCode::MapGetMissingKey,
                            None,
                            None,
                            Some(key_str),
                            None,
                            None,
                            None,
                            None,
                        ),
                    });
                    return MapMethodCallResult {
                        inferred: SmeltType::Unknown,
                        sentinels,
                        static_resolution: StaticResolution::Absent,
                    };
                }
            }
            // `has` with static key: resolve to Bool(true/false) with no diagnostic.
            let key_present = contents.contains_key(key_str);
            let boolean_ty = (method.return_type_formula)(key_ty, value_ty);
            return MapMethodCallResult {
                inferred: boolean_ty,
                sentinels,
                static_resolution: StaticResolution::Bool(key_present),
            };
        }
        // Non-static key: fall through to the formula computation below.
    }

    // Happy path: compute return type from the method's formula.
    let return_ty = (method.return_type_formula)(key_ty, value_ty);
    MapMethodCallResult {
        inferred: return_ty,
        sentinels,
        static_resolution: StaticResolution::Deferred,
    }
}

/// Validate a `Map<K, V>` type expression: check that `K` is `Text` (v1 constraint).
///
/// Returns `(sentinels, recovered_type)`.
///
/// * `sentinels` — empty on success; one `MapKeyTypeNotText` sentinel anchored at
///   `key_span` when `K` is not `SmeltType::Expr(Concrete(Text))`.
/// * `recovered_type` — the canonical type to use for the rest of the enclosing
///   declaration body:
///   - When `K = Text` (valid): the original `map_type` unchanged.
///   - When `K ≠ Text` (invalid): `Map<Text, V>` using the user-supplied `V`,
///     recovering the original `V` to avoid avalanche errors downstream.
///   - When `map_type` is not a `Map` at all: `map_type` unchanged (nothing to validate).
///
/// Per spec rule 1: callers must use the returned `recovered_type` rather than the
/// original `map_type` when `K ≠ Text` — this avoids cascading "expected Text, got X"
/// diagnostics for every key-expression inside the declaration.
///
/// Pure — no Salsa dependency.
pub fn validate_map_type_expression(
    map_type: &SmeltType,
    key_span: TextRange,
) -> (Vec<RecordLiteralSentinel>, SmeltType) {
    let mut sentinels = Vec::new();

    let (key_ty, value_ty) = match map_type {
        SmeltType::Map { key, value } => (key.as_ref(), value.as_ref()),
        _ => return (sentinels, map_type.clone()), // Not a Map — nothing to validate.
    };

    let is_text = matches!(
        key_ty,
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
    );

    if !is_text {
        let type_display = format!("{key_ty}");
        sentinels.push(RecordLiteralSentinel {
            code: crate::DiagnosticCode::MapKeyTypeNotText,
            span: key_span,
            message: crate::meta_map_diagnostic_message(
                crate::DiagnosticCode::MapKeyTypeNotText,
                None,
                None,
                None,
                None,
                Some(&type_display),
                None,
                None,
            ),
        });
        // Recover as Map<Text, V> to avoid avalanche errors.
        let recovered = SmeltType::Map {
            key: Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
            value: Box::new(value_ty.clone()),
        };
        return (sentinels, recovered);
    }

    (sentinels, map_type.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rowan::TextRange;
    use smelt_types::{
        signatures::{SmeltType, TypeConstraint},
        DataType,
    };

    fn map_text_integer() -> SmeltType {
        SmeltType::Map {
            key: Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
            value: Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))),
        }
    }

    fn dynamic_text_arg() -> MapCallArg {
        MapCallArg::Positional {
            ty: SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
            literal_value: None,
        }
    }

    /// Lock: `m.has(k)` with a non-static (dynamic) key synthesises `Boolean`,
    /// not `Unknown`.  A deferred `has` must never resolve to `Unknown` — the
    /// result stays a `Boolean` meta-value whose resolution defers to expansion
    /// time (spec §"Maps" rule 4; D-18).
    #[test]
    fn deferred_has_is_boolean_not_unknown() {
        let receiver = map_text_integer();
        let args = [dynamic_text_arg()];
        let zero = TextRange::new(0.into(), 0.into());
        let result = infer_map_method_call(&receiver, "has", &args, None, zero);

        assert!(
            result.sentinels.is_empty(),
            "m.has(dynamic_k) must emit no diagnostics; got: {:?}",
            result.sentinels
        );
        assert!(
            matches!(
                result.inferred,
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean))
            ),
            "m.has(dynamic_k) must synthesise Boolean (not Unknown); got: {:?}",
            result.inferred
        );
        assert_eq!(
            result.static_resolution,
            StaticResolution::Deferred,
            "m.has(dynamic_k) must report Deferred (not a static lookup); got: {:?}",
            result.static_resolution
        );
    }
}

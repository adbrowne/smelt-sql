use super::*;
use crate::DataType;

/// Trailing row-polymorphism marker on a `Struct<{…}>` parameter type
/// (Phase 35).
///
/// Mirrors the shape of [`RowTail`] used by `TableExpr<{…}>` parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructRowTail {
    /// No tail marker — the struct shape is fully concrete.
    None,
    /// `..` — anonymous row variable; extra fields accepted but not
    /// observable by name in the function body.
    Anon,
    /// `..<name>` — named row variable; extra fields are bound to this
    /// name and may be referenced / spread in the function body.
    Named(String),
}

/// Per-column requirement inside a [`SchemaRequirement`] (Phase 16).
///
/// Introduced by `TableExpr<{col: Type, ..}>` type annotations. Each
/// field can be:
///   - [`DataTypeReq::Concrete`] — a single concrete [`DataType`]
///     (e.g. `order_id: BigInt`). The caller's column must match
///     exactly via [`types_compatible_for_row_requirement`].
///   - [`DataTypeReq::Constraint`] — a constraint set from
///     [`TypeConstraint`] (e.g. `revenue: Numeric`). The caller's
///     column's data type must satisfy the constraint via
///     [`TypeConstraint::satisfies`].
///
/// The enum mirrors [`TypeConstraint`] / [`DataType`] so existing
/// constraint-satisfaction code applies unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataTypeReq {
    /// The caller's column must be (exactly) this [`DataType`].
    Concrete(DataType),
    /// The caller's column's data type must satisfy this
    /// [`TypeConstraint`] (e.g. `Numeric`, `Ordered`).
    Constraint(TypeConstraint),
}

impl DataTypeReq {
    /// Is the caller's [`DataType`] compatible with this requirement?
    ///
    /// - `Concrete(expected)`: matched against `actual` via the shared
    ///   row-requirement compatibility helper (equal, or a normalized
    ///   `Text ↔ Varchar` pair).
    /// - `Constraint(c)`: delegated to [`TypeConstraint::satisfies`].
    ///
    /// `Unknown` / `Null` actuals are always accepted — row-requirement
    /// checking is a best-effort pre-expansion guard; a truly unknown
    /// column type shouldn't generate a call-site error we can't act
    /// upon.
    pub fn is_satisfied_by(&self, actual: &DataType) -> bool {
        if matches!(actual, DataType::Unknown(_) | DataType::Null) {
            return true;
        }
        match self {
            DataTypeReq::Concrete(expected) => {
                types_compatible_for_row_requirement(expected, actual)
            }
            DataTypeReq::Constraint(c) => c.satisfies(actual),
        }
    }

    /// Human-readable rendering for diagnostic messages. Mirrors the
    /// user-facing syntax: `Integer`, `Numeric`, `Any`, …
    pub fn render(&self) -> String {
        match self {
            DataTypeReq::Concrete(dt) => dt.to_string(),
            DataTypeReq::Constraint(TypeConstraint::Concrete(dt)) => dt.to_string(),
            DataTypeReq::Constraint(TypeConstraint::Numeric) => "Numeric".to_string(),
            DataTypeReq::Constraint(TypeConstraint::Ordered) => "Ordered".to_string(),
            DataTypeReq::Constraint(TypeConstraint::Any) => "Any".to_string(),
        }
    }
}

/// Structural check used by [`DataTypeReq::is_satisfied_by`] for the
/// `Concrete` branch. Callers should not use this directly; it is
/// published so row-requirement unit tests can mimic the canonical
/// compatibility rule.
///
/// Accepts:
///   - Exact equality of `DataType` (after normalization of the
///     `Text ↔ Varchar` family).
pub fn types_compatible_for_row_requirement(expected: &DataType, actual: &DataType) -> bool {
    if expected == actual {
        return true;
    }
    expected.normalize() == actual.normalize()
}

/// Trailing row-polymorphism marker on a [`SchemaRequirement`].
///
/// Introduced by the grammar `TableExpr<{col: Type, ..r}>` or
/// `TableExpr<{col: Type, ..}>`:
///   - [`RowTail::None`] — no trailing marker; the caller's schema
///     must exactly match `required` (no extras allowed).
///   - [`RowTail::Anon`] — `..`; extras are allowed but not observable.
///   - [`RowTail::Named(name)`] — `..<name>`; extras are bound to the
///     named row variable on the per-call `row_var_env`. In Phase 16
///     the binding exists but is not yet user-referenceable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowTail {
    /// No tail marker — the schema must match exactly.
    None,
    /// `..` — extras accepted, not bound.
    Anon,
    /// `..<name>` — extras accepted and bound as a row variable.
    Named(String),
}

/// Structured row requirement on a `TableExpr<{…}>` parameter
/// (Phase 16).
///
/// `required` is the ordered list of `(column_name, requirement, not_null)`
/// triples declared in the signature. `tail` is the trailing marker
/// decision: no tail, anonymous tail, or a named row variable.
///
/// The check at the call site is performed by
/// [`check_schema_requirement`] — a pure function that takes the
/// requirement, the caller's schema, and returns either a
/// [`RowVarBinding`] (the extras captured by a named tail, empty for
/// `None` / `Anon`) or a [`SchemaMismatch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaRequirement {
    /// The declared `(column_name, requirement, not_null)` triples in source order.
    /// `not_null` is `true` when the field was declared with a `NOT NULL` qualifier
    /// (Phase 5, nullability-soundness).
    pub required: Vec<(String, DataTypeReq, bool)>,
    /// Trailing row-variable behaviour.
    pub tail: RowTail,
}

/// Per-call binding produced by a successful [`check_schema_requirement`]
/// against a [`RowTail::Named`] tail (Phase 16).
///
/// `name` is the row variable's source name (e.g. `"r"`). `extras` is
/// the ordered list of caller columns not matched by any `required`
/// entry. For [`RowTail::None`] and [`RowTail::Anon`] the binding is
/// `None`; for [`RowTail::Named`] with no extras `extras` is the
/// empty vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowVarBinding {
    /// Source name of the row variable from the signature.
    pub name: String,
    /// Caller columns not covered by `required`, in caller-supplied
    /// order. Each element is `(column_name, column_data_type)`.
    pub extras: Vec<(String, DataType)>,
}

/// Structured failure describing why a [`check_schema_requirement`]
/// check failed (Phase 16).
///
/// The call-site checker converts this into a user-facing
/// [`DiagnosticCode::RowRequirementUnsatisfied`] diagnostic.
///
/// In Phase 16 we accept extras by default — i.e. even when the
/// requirement declared no explicit tail we behave as open-record
/// row polymorphism dictates (research §8). Future phases may opt in
/// to strict-schema enforcement; for now the row-requirement check
/// reports only `MissingColumn` and `TypeMismatch` failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaMismatch {
    /// The caller's schema does not contain a column declared in the
    /// requirement. `column` is the declared name; `required` is the
    /// declared constraint rendered for diagnostics.
    MissingColumn {
        column: String,
        required: DataTypeReq,
    },
    /// The caller's column has a type that does not satisfy the
    /// declared requirement. `actual` is the caller's reported
    /// [`DataType`] rendering.
    TypeMismatch {
        column: String,
        required: DataTypeReq,
        actual: String,
    },
}

/// Pure check: does the caller's schema satisfy the
/// [`SchemaRequirement`] declared on a `TableExpr<{…}>` parameter?
///
/// On success, returns the optional [`RowVarBinding`] that the tail's
/// named row variable — if any — should map to on the per-call
/// `row_var_env`. For [`RowTail::None`] / [`RowTail::Anon`] the
/// binding is `None`.
///
/// On failure, returns the first structural problem detected. We
/// report the first problem only (not a batch) because the
/// diagnostics layer generally benefits from one concrete, actionable
/// message per check; later phases can extend this to batched
/// reporting if needed.
///
/// Pure — no Salsa, no I/O.
pub fn check_schema_requirement(
    req: &SchemaRequirement,
    arg_schema: &[(String, DataType)],
) -> Result<Option<RowVarBinding>, SchemaMismatch> {
    check_schema_requirement_with_nullability(req, arg_schema, &[])
}

/// Extended variant that also checks nullability when the caller provides
/// per-column nullable flags (Phase 5, nullability-soundness).
///
/// `caller_nullability` maps column names to `true` (nullable) /
/// `false` (non-nullable). Columns not present in the map are treated as
/// nullable (conservative). When a required column is declared `NOT NULL`
/// and the caller's column is nullable, a `NullabilityMismatch` variant
/// (rendered as `TypeMismatch` in the diagnostic) is returned.
pub fn check_schema_requirement_with_nullability(
    req: &SchemaRequirement,
    arg_schema: &[(String, DataType)],
    caller_nullability: &[(String, bool)],
) -> Result<Option<RowVarBinding>, SchemaMismatch> {
    // 1. Every required column must be present and type-compatible.
    //    We report the first structural problem in declaration order.
    for (col_name, col_req, col_not_null) in &req.required {
        let Some((_, actual_dt)) = arg_schema.iter().find(|(n, _)| n == col_name) else {
            return Err(SchemaMismatch::MissingColumn {
                column: col_name.clone(),
                required: col_req.clone(),
            });
        };
        if !col_req.is_satisfied_by(actual_dt) {
            return Err(SchemaMismatch::TypeMismatch {
                column: col_name.clone(),
                required: col_req.clone(),
                actual: actual_dt.to_string(),
            });
        }
        // Phase 5: check nullability when the column is declared NOT NULL.
        if *col_not_null {
            let caller_is_nullable = caller_nullability
                .iter()
                .find(|(n, _)| n == col_name)
                .map(|(_, nullable)| *nullable)
                .unwrap_or(true); // unknown → assume nullable (conservative)
            if caller_is_nullable {
                return Err(SchemaMismatch::TypeMismatch {
                    column: col_name.clone(),
                    required: col_req.clone(),
                    actual: format!("{} (nullable)", actual_dt),
                });
            }
        }
    }

    // 2. Extras handling. Compute the set of required column names
    //    once, then collect every caller column not in that set —
    //    this is the row-variable's extras list (or the
    //    "unexpected" list when `RowTail::None`).
    let required_names: std::collections::HashSet<&str> =
        req.required.iter().map(|(n, _, _)| n.as_str()).collect();
    let extras: Vec<(String, DataType)> = arg_schema
        .iter()
        .filter(|(n, _)| !required_names.contains(n.as_str()))
        .cloned()
        .collect();

    match &req.tail {
        // Phase 16: extras are accepted regardless of tail — the
        // open-record semantics in research §8 treat missing /
        // wrong-type columns as the only structural failures.
        // `RowTail::None` and `RowTail::Anon` simply discard extras;
        // `RowTail::Named` captures them into the per-call
        // `row_var_env` for future reference by the body / return
        // type (Phases 17+ / 37).
        RowTail::None | RowTail::Anon => Ok(None),
        RowTail::Named(name) => Ok(Some(RowVarBinding {
            name: name.clone(),
            extras,
        })),
    }
}

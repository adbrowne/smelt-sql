use super::*;
use crate::DataType;
use std::collections::HashMap;

/// Result of a successful [`unify_call`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifyResult {
    /// The concrete return type after resolving all type variables.
    pub return_type: DataType,
    /// Bindings collected for each declared type variable.
    pub bindings: HashMap<String, DataType>,
}

/// Error produced when call-site arguments don't match a signature (§16 #14/#15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnificationError {
    /// A concrete argument type didn't satisfy the parameter's constraint.
    /// `position` is 1-based.
    ConstraintViolation {
        position: usize,
        param_constraint: TypeConstraint,
        actual: DataType,
    },
    /// A type variable bound inconsistently across positions. `positions`
    /// holds every 1-based argument index where this variable appeared
    /// (plus the return position only if it participated — not used in v1).
    InconsistentBinding {
        var_name: String,
        positions: Vec<usize>,
        types: Vec<DataType>,
    },
    /// Not enough positional arguments supplied.
    MissingArgs { expected: usize, got: usize },
    /// Too many positional arguments — no variadic to absorb the overflow.
    TooManyArgs { expected: usize, got: usize },
    /// A variadic type variable with no supplied arguments and no return
    /// binding — can't determine what to bind it to (§16 #15 fallback 2).
    EmptyVariadicTypeVar { var_name: String },
}

impl std::fmt::Display for UnificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnificationError::ConstraintViolation {
                position,
                param_constraint,
                actual,
            } => write!(
                f,
                "argument at position {position} (type {actual}) does not satisfy {param_constraint:?}"
            ),
            UnificationError::InconsistentBinding {
                var_name,
                positions,
                types,
            } => {
                write!(f, "type variable `{var_name}` inferred inconsistently:")?;
                for (pos, ty) in positions.iter().zip(types.iter()) {
                    write!(f, " position {pos} = {ty};")?;
                }
                Ok(())
            }
            UnificationError::MissingArgs { expected, got } => {
                write!(f, "expected at least {expected} argument(s), got {got}")
            }
            UnificationError::TooManyArgs { expected, got } => {
                write!(f, "expected {expected} argument(s), got {got}")
            }
            UnificationError::EmptyVariadicTypeVar { var_name } => write!(
                f,
                "cannot infer type variable `{var_name}` — variadic position received no arguments and no return type is expected from context"
            ),
        }
    }
}

impl std::error::Error for UnificationError {}

/// Unify a signature against a list of concrete argument types, optionally
/// incorporating an expected return type for bidirectional inference (§16 #14,
/// Decision 14 — Phase 27).
///
/// When `expected_return` is `Some(dt)` and the signature's `return_type` is
/// [`TypeExpr::Var(name)`], `dt` is injected as an additional position at
/// **index 0** ("return context") before the binding-reduction step.  This
/// means the expected return participates in LUB (for `Numeric`-constrained
/// variables) or exact-equality checks (for `Ordered`/`Any`/`Concrete`
/// variables) alongside the argument-derived positions.
///
/// Concretely: `COALESCE(1, 2)` in a `Double` context has positions
/// `{(0, Double), (1, Integer), (2, Integer)}`; LUB under the Numeric chain
/// = `Double`, so the call successfully types as `Double`.
///
/// When `expected_return` is `None` this function is equivalent to
/// the plain [`unify_call`].
///
/// ### Position encoding
/// - Positions 1, 2, … are argument positions (1-based, as in v1).
/// - Position 0 is reserved for the "return context" when
///   `expected_return` is `Some(_)`.
///
/// The `lub` closure lives outside this crate because the real LUB
/// computation is in `smelt-db::type_inference::promote_types`, and
/// `smelt-types` must remain dependency-free.
pub fn unify_call_with_expected(
    sig: &Signature,
    args: &[DataType],
    expected_return: Option<&DataType>,
    lub: &dyn Fn(&DataType, &DataType) -> DataType,
) -> Result<UnifyResult, UnificationError> {
    // Determine whether the return type is a naked type variable —
    // only then does `expected_return` contribute a position.
    let return_var: Option<&str> = match &sig.return_type {
        TypeExpr::Var(name) => Some(name.as_str()),
        _ => None,
    };

    // Split leading vs (optional) trailing variadic.
    let (leading, variadic) = match sig.params.last() {
        Some(SigParam::Variadic(inner)) => {
            let last_idx = sig.params.len() - 1;
            (&sig.params[..last_idx], Some(inner.as_ref()))
        }
        _ => (&sig.params[..], None),
    };

    // Arity checks.
    if variadic.is_none() {
        if args.len() < leading.len() {
            return Err(UnificationError::MissingArgs {
                expected: leading.len(),
                got: args.len(),
            });
        }
        if args.len() > leading.len() {
            return Err(UnificationError::TooManyArgs {
                expected: leading.len(),
                got: args.len(),
            });
        }
    } else if args.len() < leading.len() {
        return Err(UnificationError::MissingArgs {
            expected: leading.len(),
            got: args.len(),
        });
    }

    // Collect positions per type variable, in 1-based order.
    let mut var_positions: HashMap<String, Vec<(usize, DataType)>> = HashMap::new();
    for tp in &sig.type_params {
        var_positions.insert(tp.name.clone(), Vec::new());
    }

    // Inject expected_return at position 0 for the return type variable.
    if let (Some(var_name), Some(expected)) = (return_var, expected_return) {
        if let Some(positions) = var_positions.get_mut(var_name) {
            // Check that the expected return satisfies the constraint first.
            let tp = sig
                .type_param(var_name)
                .expect("validated in Signature::new");
            if !tp.constraint.satisfies(expected) {
                // ConstraintViolation at position 0 (return context).
                return Err(UnificationError::ConstraintViolation {
                    position: 0,
                    param_constraint: tp.constraint.clone(),
                    actual: expected.clone(),
                });
            }
            positions.push((0, expected.clone()));
        }
    }

    let check_concrete = |position: usize,
                          constraint: &TypeConstraint,
                          arg: &DataType|
     -> Result<(), UnificationError> {
        if constraint.satisfies(arg) {
            Ok(())
        } else {
            Err(UnificationError::ConstraintViolation {
                position,
                param_constraint: constraint.clone(),
                actual: arg.clone(),
            })
        }
    };

    // Leading params.
    for (idx, (param, arg)) in leading.iter().zip(args.iter()).enumerate() {
        let position = idx + 1;
        match param {
            SigParam::Concrete(c) => check_concrete(position, c, arg)?,
            SigParam::Var(var_name) => {
                let tp = sig
                    .type_param(var_name)
                    .expect("validated in Signature::new");
                check_concrete(position, &tp.constraint, arg)?;
                var_positions
                    .get_mut(var_name)
                    .expect("initialised above")
                    .push((position, arg.clone()));
            }
            SigParam::Variadic(_) => unreachable!("leading can't contain variadic"),
        }
    }

    // Variadic params.
    if let Some(inner) = variadic {
        for (rel, arg) in args[leading.len()..].iter().enumerate() {
            let position = leading.len() + rel + 1;
            match inner {
                SigParam::Concrete(c) => check_concrete(position, c, arg)?,
                SigParam::Var(var_name) => {
                    let tp = sig
                        .type_param(var_name)
                        .expect("validated in Signature::new");
                    check_concrete(position, &tp.constraint, arg)?;
                    var_positions
                        .get_mut(var_name)
                        .expect("initialised above")
                        .push((position, arg.clone()));
                }
                SigParam::Variadic(_) => {
                    unreachable!("nested variadic rejected in Signature::try_new")
                }
            }
        }
    }

    // Reduce per-var positions into a single binding.
    let mut bindings: HashMap<String, DataType> = HashMap::new();
    for tp in &sig.type_params {
        let positions = var_positions.remove(&tp.name).unwrap_or_default();
        if positions.is_empty() {
            return Err(UnificationError::EmptyVariadicTypeVar {
                var_name: tp.name.clone(),
            });
        }
        let binding = match tp.constraint {
            // Only Numeric has a declared promotion chain in v1 (§16 #9/#14).
            TypeConstraint::Numeric => {
                let mut iter = positions.iter();
                let (_, first) = iter.next().unwrap();
                let mut acc = first.clone();
                for (_, ty) in iter {
                    acc = lub(&acc, ty);
                }
                acc
            }
            // All non-Numeric constraints (Ordered, Any, Concrete): require
            // exact equality across positions.
            _ => {
                let first = &positions[0].1;
                let disagreements: Vec<(usize, DataType)> = positions
                    .iter()
                    .filter(|(_, ty)| ty != first)
                    .cloned()
                    .collect();
                if !disagreements.is_empty() {
                    let all_positions: Vec<usize> = positions.iter().map(|(p, _)| *p).collect();
                    let all_types: Vec<DataType> =
                        positions.iter().map(|(_, t)| t.clone()).collect();
                    return Err(UnificationError::InconsistentBinding {
                        var_name: tp.name.clone(),
                        positions: all_positions,
                        types: all_types,
                    });
                }
                first.clone()
            }
        };
        bindings.insert(tp.name.clone(), binding);
    }

    // Resolve the return type.
    let return_type = match &sig.return_type {
        TypeExpr::Concrete(TypeConstraint::Concrete(dt)) => dt.clone(),
        TypeExpr::Concrete(TypeConstraint::Numeric) => DataType::Double,
        TypeExpr::Concrete(TypeConstraint::Ordered) => {
            DataType::Unknown(crate::UnknownReason::Dynamic)
        }
        TypeExpr::Concrete(TypeConstraint::Any) => DataType::Unknown(crate::UnknownReason::Dynamic),
        TypeExpr::Var(var_name) => bindings
            .get(var_name)
            .cloned()
            .expect("type var validated at Signature::new"),
    };

    Ok(UnifyResult {
        return_type,
        bindings,
    })
}

/// Unify a signature against a list of concrete argument types.
///
/// For each signature, the checker collects every position where a type
/// variable appears (argument positions only in v1; bidirectional
/// checking is deferred to Step 5). Variables whose constraint is
/// [`TypeConstraint::Numeric`] reduce by LUB via the caller-supplied
/// `lub` closure (the only promotion-chain constraint in v1, §16 #14);
/// every other constraint requires exact equality across positions.
///
/// The `lub` closure lives outside this crate because the real LUB
/// computation is in `smelt-db::type_inference::promote_types`, and
/// `smelt-types` must remain dependency-free. Tests in this module use
/// a small inline LUB that matches §16 #9's promotion chain.
///
/// This is a thin wrapper around [`unify_call_with_expected`] with
/// `expected_return = None`. All existing callers continue to compile
/// unchanged.
pub fn unify_call(
    sig: &Signature,
    args: &[DataType],
    lub: &dyn Fn(&DataType, &DataType) -> DataType,
) -> Result<UnifyResult, UnificationError> {
    unify_call_with_expected(sig, args, None, lub)
}

/// A minimal Numeric LUB matching §16 #9 — the only promotion chain in v1.
///
/// Lives in `smelt-types` so the signature-unification unit tests don't
/// depend on `smelt-db`. Production callers in Phase 9+ will pass
/// `smelt-db::promote_types` (or a thin adapter) to [`unify_call`] instead.
pub fn numeric_lub(a: &DataType, b: &DataType) -> DataType {
    use DataType::*;

    // Helper: lift an integer type to its Decimal equivalent per §15.
    // SmallInt → Decimal(5,0), Integer → Decimal(10,0), BigInt → Decimal(19,0).
    let lift_integer_to_decimal = |d: &DataType| -> Option<(u8, u8)> {
        match d {
            SmallInt => Some((5, 0)),
            Integer => Some((10, 0)),
            BigInt => Some((19, 0)),
            _ => None,
        }
    };

    // Apply the Decimal LUB formula (§15): given (p1,s1) and (p2,s2),
    // s' = max(s1,s2), p' = max(p1-s1, p2-s2) + s', saturated at 38.
    let decimal_lub = |p1: u8, s1: u8, p2: u8, s2: u8| -> DataType {
        let s = s1.max(s2) as u32;
        let int_digits1 = (p1 as u32).saturating_sub(s1 as u32);
        let int_digits2 = (p2 as u32).saturating_sub(s2 as u32);
        let p = int_digits1.max(int_digits2) + s;
        Decimal {
            precision: p.min(38) as u8,
            scale: s as u8,
        }
    };

    // Handle Decimal pairs (same or different params) using the formula.
    if let (
        Decimal {
            precision: p1,
            scale: s1,
        },
        Decimal {
            precision: p2,
            scale: s2,
        },
    ) = (a, b)
    {
        return decimal_lub(*p1, *s1, *p2, *s2);
    }

    // Handle Decimal + integer: lift the integer, then apply the formula.
    if let Decimal {
        precision: pd,
        scale: sd,
    } = a
    {
        if let Some((pi, si)) = lift_integer_to_decimal(b) {
            return decimal_lub(*pd, *sd, pi, si);
        }
    }
    if let Decimal {
        precision: pd,
        scale: sd,
    } = b
    {
        if let Some((pi, si)) = lift_integer_to_decimal(a) {
            return decimal_lub(pi, si, *pd, *sd);
        }
    }

    // For all other pairs, use the rank-based promotion chain (§16 #9).
    let rank = |d: &DataType| -> u8 {
        match d {
            SmallInt => 1,
            Integer => 2,
            BigInt => 3,
            Decimal { .. } => 4,
            Float => 5,
            Double => 6,
            _ => 0,
        }
    };
    let (ra, rb) = (rank(a), rank(b));
    if ra >= rb {
        a.clone()
    } else {
        b.clone()
    }
}

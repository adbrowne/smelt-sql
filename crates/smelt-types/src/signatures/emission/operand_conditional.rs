use super::position_rewrite::{Position, RestructureId, RewriteId};
use super::template::{validate_template, TemplateError};
use crate::signatures::{SigParam, Signature};
use crate::DataType;

/// An argument's class for an [`super::Emission::Conditional`] guard — a pure,
/// total function of its inferred [`DataType`]
/// (`docs/specs/multi_backend.md` §"Operand-conditional verdicts").
///
/// A guard names classes, never concrete types: the engine behaviours this
/// axis exists for split along exactly these lines, and a finer key would
/// multiply arms without buying a different answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperandClass {
    /// The integer widths (`SmallInt`, `Integer`, `BigInt`).
    Integral,
    /// `Decimal { .. }`.
    Decimal,
    /// `Float`, `Double`.
    Floating,
    /// `Varchar`, `Char`, `Text`.
    String,
    /// `Boolean`.
    Boolean,
    /// `Date`, `Time`, and the timestamps.
    Temporal,
    /// `Interval`.
    Interval,
    /// `Array`, `Struct`, `Map`.
    Composite,
    /// `Blob`.
    Binary,
    /// The argument's type could not be resolved — `DataType::Unknown` (type
    /// inference gave up) or `DataType::Null` (a NULL literal discriminates
    /// nothing). This is the fail-safe direction the cost rule in
    /// `docs/specs/multi_backend.md` §"Operand-conditional verdicts" demands:
    /// an unresolved operand always lands on the `otherwise` arm.
    Unresolved,
}

impl OperandClass {
    /// Classify a [`DataType`] into its [`OperandClass`]. Total — a new
    /// `DataType` variant is a compile error here, never a silent
    /// misclassification via a wildcard arm.
    pub fn of(dt: &DataType) -> OperandClass {
        match dt {
            DataType::Boolean => OperandClass::Boolean,
            DataType::SmallInt | DataType::Integer | DataType::BigInt => OperandClass::Integral,
            DataType::Float | DataType::Double => OperandClass::Floating,
            DataType::Decimal { .. } => OperandClass::Decimal,
            DataType::Varchar { .. } | DataType::Char { .. } | DataType::Text => {
                OperandClass::String
            }
            DataType::Blob => OperandClass::Binary,
            DataType::Date | DataType::Time | DataType::Timestamp { .. } => OperandClass::Temporal,
            DataType::Interval => OperandClass::Interval,
            DataType::Array(_) | DataType::Struct(_) | DataType::Map(_, _) => {
                OperandClass::Composite
            }
            DataType::Null | DataType::Unknown(_) => OperandClass::Unresolved,
        }
    }
}

/// A settled [`super::Emission`] verdict — the six ordinary verdicts an
/// [`super::Emission::Conditional`] arm may carry, and what
/// `Signature::settle_at` always returns. Unlike [`super::Emission`], this
/// type has no `Conditional` variant: a nested conditional is
/// unrepresentable by construction, not merely rejected at validation time.
/// This is what the printer consumes — it holds no type context and cannot
/// itself resolve an arm (`docs/specs/multi_backend.md` §"Operand-conditional
/// verdicts").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettledEmission {
    /// Same spelling, same semantics.
    Native,
    /// Same call shape, different name.
    Rename(&'static str),
    /// A fixed shape over the call's own positional arguments. See
    /// [`super::Emission::Template`].
    Template(&'static str),
    /// Structural rewrite. See [`super::Emission::Rewrite`].
    Rewrite(RewriteId),
    /// Statement-level restructure. See [`super::Emission::Restructure`].
    Restructure(RestructureId),
    /// The backend cannot express this. See [`super::Emission::Unsupported`].
    Unsupported { reason: &'static str },
}

/// One arm of an [`super::Emission::Conditional`] entry.
///
/// `arity`: `Some(n)` guards on the call having exactly `n` arguments;
/// `None` matches any arity. `classes`: `(argument_index, required_class)`
/// pairs, all of which must match for the arm to apply; `&[]` matches any
/// operand shape. The mandatory final `otherwise` arm has `arity: None` and
/// `classes: &[]`, so it always matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConditionalArm {
    pub arity: Option<usize>,
    pub classes: &'static [(usize, OperandClass)],
    pub verdict: SettledEmission,
}

impl ConditionalArm {
    /// Does `facts` satisfy this arm's guard?
    pub(crate) fn matches(&self, facts: &CallFacts) -> bool {
        if let Some(arity) = self.arity {
            if facts.arity != arity {
                return false;
            }
        }
        self.classes
            .iter()
            .all(|(idx, class)| facts.class_at(*idx) == Some(*class))
    }

    /// Is this arm unconditional (the required shape of the mandatory
    /// trailing `otherwise` arm)?
    fn is_otherwise(&self) -> bool {
        self.arity.is_none() && self.classes.is_empty()
    }
}

/// The call-site facts an [`super::Emission::Conditional`] entry is resolved
/// against: the call's own arity, and each argument's [`OperandClass`] by
/// position. Built once on the compile path from the source CST and the
/// same type inference that derives the model's projection; the printer
/// never constructs one from a `DataType` — see
/// [`CallFacts::unresolved`] for the printer's own lookup-miss fallback,
/// which needs only arity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallFacts {
    arity: usize,
    classes: Vec<OperandClass>,
}

impl CallFacts {
    /// Build call facts from a resolved class per positional argument.
    pub fn new(classes: Vec<OperandClass>) -> Self {
        Self {
            arity: classes.len(),
            classes,
        }
    }

    /// A call whose arity is known but whose operand types are not — every
    /// class is [`OperandClass::Unresolved`], so any class-guarded arm is
    /// skipped and resolution lands on `otherwise`.
    pub fn unresolved(arity: usize) -> Self {
        Self {
            arity,
            classes: vec![OperandClass::Unresolved; arity],
        }
    }

    /// The class of the argument at `index`, or `None` if `index` is beyond
    /// this call's arity.
    fn class_at(&self, index: usize) -> Option<OperandClass> {
        self.classes.get(index).copied()
    }
}

/// Why an [`super::Emission::Conditional`] entry failed registry-construction
/// validation (`docs/specs/multi_backend.md` §"Operand-conditional
/// verdicts"). Every variant names the offending signature, mirroring
/// [`TemplateError`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionalError {
    /// The arm list has no trailing `otherwise` arm (`arity: None, classes:
    /// &[]`), or the `otherwise` arm is not last.
    MissingOtherwise { signature: String },
    /// An arm's `arity` guard names a call shape the signature itself does
    /// not admit (fewer than the signature's non-variadic arity, or more
    /// than it admits when the signature has no variadic tail).
    ArityNotAdmitted { signature: String, arity: usize },
    /// An arm's class guard names an argument index at or beyond the arity
    /// it guards on (or, for an arity-less arm, beyond the signature's own
    /// fixed arity).
    ArgumentIndexOutOfRange { signature: String, index: usize },
    /// An arm's verdict is a [`SettledEmission::Template`] that fails
    /// [`validate_template`]'s own checks — the same discipline a top-level
    /// `Emission::Template` row is held to, applied per arm.
    InvalidTemplateArm {
        signature: String,
        arm_index: usize,
        error: TemplateError,
    },
}

impl std::fmt::Display for ConditionalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConditionalError::MissingOtherwise { signature } => write!(
                f,
                "conditional emission for `{signature}` has no trailing `otherwise` arm (arity \
                 `None`, classes `&[]`)"
            ),
            ConditionalError::ArityNotAdmitted { signature, arity } => write!(
                f,
                "conditional emission for `{signature}` has an arm guarding on arity {arity}, \
                 which the signature does not admit"
            ),
            ConditionalError::ArgumentIndexOutOfRange { signature, index } => write!(
                f,
                "conditional emission for `{signature}` has an arm naming argument index \
                 {index}, beyond the arity it guards on"
            ),
            ConditionalError::InvalidTemplateArm {
                signature,
                arm_index,
                error,
            } => write!(
                f,
                "conditional emission for `{signature}` has an invalid template at arm \
                 {arm_index}: {error}"
            ),
        }
    }
}

impl std::error::Error for ConditionalError {}

/// Validate an [`super::Emission::Conditional`] entry at registry-construction
/// time. Pure — the registry seed converts an `Err` into a build-time panic,
/// the same discipline as [`validate_template`].
pub fn validate_conditional(
    arms: &'static [ConditionalArm],
    sig: &Signature,
    position: Position,
) -> Result<(), ConditionalError> {
    let variadic = matches!(sig.params.last(), Some(SigParam::Variadic(_)));
    let fixed_arity = sig.params.len();

    match arms.last() {
        Some(last) if last.is_otherwise() => {}
        _ => {
            return Err(ConditionalError::MissingOtherwise {
                signature: sig.name.clone(),
            })
        }
    }
    // No arm before the last may itself be an `otherwise` arm — the ordered
    // list would then have unreachable arms after it, and validation must
    // catch a malformed list, not merely "there exists an otherwise arm".
    if arms[..arms.len() - 1]
        .iter()
        .any(ConditionalArm::is_otherwise)
    {
        return Err(ConditionalError::MissingOtherwise {
            signature: sig.name.clone(),
        });
    }

    for (arm_index, arm) in arms.iter().enumerate() {
        if let Some(arity) = arm.arity {
            let admitted = if variadic {
                arity + 1 >= fixed_arity
            } else {
                arity == fixed_arity
            };
            if !admitted {
                return Err(ConditionalError::ArityNotAdmitted {
                    signature: sig.name.clone(),
                    arity,
                });
            }
        }
        let bound = arm.arity.unwrap_or(fixed_arity);
        for (idx, _) in arm.classes {
            if *idx >= bound {
                return Err(ConditionalError::ArgumentIndexOutOfRange {
                    signature: sig.name.clone(),
                    index: *idx,
                });
            }
        }
        if let SettledEmission::Template(template) = arm.verdict {
            validate_template(template, sig, position).map_err(|error| {
                ConditionalError::InvalidTemplateArm {
                    signature: sig.name.clone(),
                    arm_index,
                    error,
                }
            })?;
        }
    }
    Ok(())
}

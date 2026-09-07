//! Emission vocabulary — how a built-in is spelled/lowered on a given
//! backend and call-site context, split into cohesive submodules:
//!
//! - [`expr_kind`] — [`ExprKind`], the linear scalar/agg/window subtyping rank.
//! - [`position_rewrite`] — [`Position`] (the lookup key's call-site axis),
//!   [`RewriteId`] and [`RestructureId`] (the enumerable structural rewrites
//!   and statement-level restructures the printer/planner implement).
//! - [`operand_conditional`] — [`OperandClass`], [`SettledEmission`],
//!   [`ConditionalArm`], [`CallFacts`] and [`validate_conditional`] for
//!   `Emission::Conditional`'s operand-conditional verdicts.
//! - [`template`] — [`TemplateError`], `is_call_shaped_template` and
//!   `validate_template` for `Emission::Template`'s placeholder substitution.

mod expr_kind;
mod operand_conditional;
mod position_rewrite;
mod template;

pub use expr_kind::{kind_ceiling, subkind_of, ExprKind};
pub use operand_conditional::{
    validate_conditional, CallFacts, ConditionalArm, ConditionalError, OperandClass,
    SettledEmission,
};
pub use position_rewrite::{Position, RestructureId, RewriteId};
pub use template::{is_call_shaped_template, validate_template, TemplateError};

/// How a built-in is spelled at a call site. Required so operators can be
/// registry entries at all, and what lets the audit harness derive a probe from
/// a signature instead of a hand-written table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum SyntaxForm {
    /// `NAME(a, b)` — the default, and the only form on the callable-function surface.
    #[default]
    Call,
    /// `a OP b` — `%`, `^`, `**`, `||`, `//`, `LIKE`, `ILIKE`, `GLOB`.
    Infix,
    /// `a OP` — `IS NULL`, `IS NOT NULL`.
    Postfix,
    /// `FROM UNNEST(a)` — a table function; not a scalar call position.
    TableFn,
    /// Dedicated syntax with no uniform shape — `CAST(x AS T)`, `a BETWEEN b AND c`,
    /// `a IN (…)`, `EXISTS (…)`, interval add/sub.
    Special,
}

/// The per-dialect verdict for one registry entry: how the built-in must be
/// spelled so the backend computes what smelt's semantics say it computes.
///
/// `Native` is a **claim, not an assumption**. The audit's value leg exists to
/// test it, and an untested `Native` is reported as *unverified* rather than as
/// *passing*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emission {
    /// Same spelling, same semantics.
    Native,
    /// Same call shape, different name.
    Rename(&'static str),
    /// The target spells this built-in as a fixed shape over the call's own
    /// positional arguments (`docs/specs/multi_backend.md` §"Template
    /// emission"). The `&'static str` is target-dialect text in which `{n}`
    /// names the call's zero-based `n`-th positional argument — an infix
    /// `BINARY_EXPR` supplies exactly two, `{0}` the left operand and `{1}`
    /// the right. A placeholder may reference an argument any number of
    /// times; every fixed parameter must be referenced at least once.
    /// Validated once at registry construction by [`validate_template`]:
    /// every placeholder index within the signature's fixed arity, balanced
    /// parentheses, no unreferenced fixed parameter, no template on a
    /// variadic signature, and no non-call-shaped template stated at a
    /// window position. Interpreted by one generic printer routine that
    /// substitutes each placeholder with the argument's own printed text,
    /// parenthesising a compound argument (`BINARY_EXPR`, `CASE_EXPR`,
    /// `CAST_EXPR` — comparisons and unary forms reuse `BINARY_EXPR`) and
    /// leaving an atom (literal, identifier, call) bare; a template whose
    /// outermost shape is not itself a single call is wrapped in parentheses
    /// so its result composes safely in the surrounding expression. The
    /// printer holds no function names and no per-template special case.
    Template(&'static str),
    /// Structural rewrite: the printer owns the code, the registry owns the claim.
    Rewrite(RewriteId),
    /// Statement-level restructure: the backend offers this built-in only in
    /// the opposite position from the one the call occupies, so no
    /// expression-level substitution exists — the whole query block is
    /// restructured around a synthesised CTE (`docs/specs/multi_backend.md`
    /// §"Statement-level lowering"). The registry states *that* the position
    /// needs restructuring and *which shape*; a pure planner (never the
    /// printer) turns the claim into data, and admissibility over the
    /// enclosing query block decides whether the restructure can be taken at
    /// all.
    Restructure(RestructureId),
    /// The backend cannot express this. A diagnostic, never a silent pass-through.
    Unsupported { reason: &'static str },
    /// The verdict depends on the call's own arity and/or operand types
    /// (`docs/specs/multi_backend.md` §"Operand-conditional verdicts").
    /// An ordered list of arms; the first arm whose guard the call satisfies
    /// wins. Must end in an `otherwise` arm (arity `None`, classes `&[]`) —
    /// validated once at registry construction by [`validate_conditional`].
    /// Resolved to a [`SettledEmission`] on the compile path by
    /// `Signature::settle_at`; the printer never inspects this variant.
    Conditional(&'static [ConditionalArm]),
}

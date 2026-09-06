//! The built-in SQL function registry — the one table.
//!
//! Per `CLAUDE.md` §"Function-registry single ownership", a built-in's name,
//! classification, registry-driven type and per-dialect/per-position emission
//! all derive from one table. That table is [`REGISTRY`], constructed exactly
//! once here. The per-family submodules below hold only *data*: each exposes a
//! `register(&mut dyn FnMut(Signature))` that hands its rows to this module's
//! single validating inserter. There is no second registry and no second
//! construction point.

use super::{
    validate_conditional, validate_template, Emission, SigParam, Signature, TypeConstraint,
    TypeParam,
};
use crate::DataType;
use std::collections::HashMap;
use std::sync::LazyLock;

mod aggregates;
mod extended_aggregates;
mod extended_math;
mod extended_string;
mod extended_temporal;
mod extended_window;
mod infix_operators;
mod null_compare;
mod numeric;
mod operator_stubs;
mod remaining;
mod string;
mod table_functions;
mod temporal;
mod window;

pub(super) fn tp(name: &str, c: TypeConstraint) -> TypeParam {
    TypeParam {
        name: name.to_string(),
        constraint: c,
    }
}

pub(super) fn concrete(dt: DataType) -> SigParam {
    SigParam::Concrete(TypeConstraint::Concrete(dt))
}

pub(super) fn var(name: &str) -> SigParam {
    SigParam::Var(name.to_string())
}

pub(super) fn variadic(inner: SigParam) -> SigParam {
    SigParam::Variadic(Box::new(inner))
}

/// The canonical table of built-in SQL function signatures.
///
/// Constructed exactly once. Every per-family submodule contributes its rows
/// through the single `insert` closure below, which validates each row's
/// template / conditional emission verdicts before it lands in the map.
pub(super) static REGISTRY: LazyLock<HashMap<String, Signature>> = LazyLock::new(|| {
    let mut m: HashMap<String, Signature> = HashMap::new();
    {
        let mut insert = |sig: Signature| {
            for (_, position, emission) in sig.emission {
                if let Emission::Template(template) = emission {
                    if let Err(e) = validate_template(template, &sig, *position) {
                        panic!("malformed built-in signature: {e}");
                    }
                }
                if let Emission::Conditional(arms) = emission {
                    if let Err(e) = validate_conditional(arms, &sig) {
                        panic!("malformed built-in signature: {e}");
                    }
                }
            }
            m.insert(sig.name.clone(), sig);
        };

        aggregates::register(&mut insert);
        window::register(&mut insert);
        null_compare::register(&mut insert);
        numeric::register(&mut insert);
        string::register(&mut insert);
        temporal::register(&mut insert);
        extended_aggregates::register(&mut insert);
        extended_window::register(&mut insert);
        extended_string::register(&mut insert);
        extended_math::register(&mut insert);
        extended_temporal::register(&mut insert);
        remaining::register(&mut insert);
        operator_stubs::register(&mut insert);
        infix_operators::register(&mut insert);
        table_functions::register(&mut insert);
    }

    m
});

/// Alias index derived from [`REGISTRY`]: maps every dialect alias
/// (upper-cased) to its canonical (upper-cased) entry name. Built once from
/// each [`Signature::aliases`] table — the single authoritative source per
/// architecture.md §Constraints #14 — never populated by hand.
pub(super) static ALIAS_MAP: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    for sig in REGISTRY.values() {
        for alias in sig.aliases {
            m.insert(alias.to_ascii_uppercase(), sig.name.clone());
        }
    }
    m
});

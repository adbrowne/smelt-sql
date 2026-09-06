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
//! a handful of SQL built-ins. Phase 8 extends the registry with angle-bracket
//! generics and trailing variadic parameters (§16 #14 + #15), adds
//! [`unify_call`] for signature-driven type inference, and seeds ~30
//! commonly-used SQL built-ins. Non-`Expr` sorts (TableExpr, AggExpr, …) remain
//! deferred to later phases of the smelt-functions plan.

mod builtins;
mod emission;
mod extract;
mod hover;
mod map_api;
mod meta;
mod parse;
mod record_registry;
mod registry;
mod schema;
mod signature;
mod smelt_type;
mod unify;
mod values;

#[cfg(test)]
mod tests;

pub use emission::*;
pub use extract::*;
pub use hover::*;
pub use map_api::*;
pub use meta::*;
pub use parse::*;
pub use record_registry::*;
pub use registry::*;
pub use schema::*;
pub use signature::*;
pub use smelt_type::*;
pub use unify::*;
pub use values::*;

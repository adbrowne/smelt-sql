//! The shared bottom-up property walk over a model's logical operator tree
//! (`model_properties.md` §"The composition walk").
//!
//! A parsed model is normalized into a [`QueryTree`] — CTE definitions in
//! dependency order, set-operation branches, FROM items including derived
//! tables — and folded bottom-up by [`walk`]. Each property contributes a
//! [`Transfer`] (a transfer function `(operator, child verdicts) → verdict`);
//! the walk applies it once per node, carrying a per-node [`NodeCx`]
//! (alias→source map and projected-column lineage).
//!
//! The fold distinguishes the three composition shapes a transfer function
//! needs:
//! - **sequential nesting** — a CTE body feeds its reference sites (the
//!   reference-site child verdict *is* the CTE subtree's verdict) and a
//!   derived table feeds the enclosing scope;
//! - **parallel branching** — set-operation arms are sibling children of a
//!   [`SetOpNode`];
//! - **joins** — multiple FROM inputs of one [`SelectNode`] are sibling
//!   children of that node.
//!
//! Fail-loud: an unrecognisable relational construct is normalized to an
//! explicit `Unsupported` node (never silently skipped), so a fail-closed
//! transfer function yields its reject verdict for the subtree above it.
//!
//! Split across submodules by subject, not by mechanical line count:
//! [`tree`] is the [`QueryTree`] normalization + the core bottom-up fold and
//! the [`Transfer`] trait; [`scopes`] is scope enumeration, admission
//! verdicts, and partition-skew; [`properties`] is the model property
//! vector (grain, functional dependencies, discriminants, determinism).

mod tree;

mod scopes;

mod properties;

#[cfg(test)]
mod tests;

pub use tree::*;

pub use scopes::*;

pub use properties::*;

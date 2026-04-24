//! Phase 30 — Logical plan data model.
//!
//! Defines the [`LogicalNode`] enum and supporting types that make up a
//! logical query plan. Plans are constructed by the `logical_plan` Salsa
//! query in `smelt-db` from the parsed CST; expansion to physical plans is
//! deferred to Phase 32+.
//!
//! # Design invariants
//!
//! * This module is **pure Rust** — no Salsa dependency. The Salsa query
//!   lives in `smelt-db`, which depends on `smelt-planner`.
//! * [`Plan`] is an `Arc<LogicalNode>` — cheap to clone and share.
//! * `FunctionCall` nodes carry a `transparent` flag (true for
//!   `smelt.define`, false for `smelt.extern`) and a [`FunctionProperties`]
//!   struct populated from the declaration's frontmatter.

use std::sync::Arc;

use smelt_types::DataType;

/// Fully-qualified function identifier, e.g. `"some_fn"` or `"core.math.safe_divide"`.
pub type FnId = String;

/// Per-function properties extracted from the declaration's frontmatter.
///
/// All fields default to `false` when the corresponding frontmatter key is
/// absent or unparseable.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FunctionProperties {
    /// The function always returns the same output for the same inputs
    /// (no side-effects, no randomness). Declared with `deterministic: true`.
    pub deterministic: bool,
    /// The function can be applied multiple times without changing the result
    /// beyond the first application. Declared with `idempotent: true`.
    pub idempotent: bool,
    /// The function only appends data, never deletes or modifies existing rows.
    /// Declared with `append_only: true`.
    pub append_only: bool,
}

/// Column-level data provenance information attached to a [`LogicalNode`].
///
/// Phase 30 uses `Unknown` everywhere; Phase 31+ will populate
/// `Declared` from per-declaration frontmatter annotations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provenance {
    /// No provenance information is available for this node.
    Unknown,
    /// Declared column-level lineage: each tuple maps an output column name
    /// to the list of input column names it depends on.
    Declared(Vec<(String, Vec<String>)>),
}

/// A node in the logical query plan.
///
/// Phase 30 introduces a minimal node set sufficient to represent
/// `smelt.fn.*` calls and the surrounding query structure. Full plan
/// construction is deferred to Phase 32+.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogicalNode {
    /// A call to a user-defined or extern smelt function.
    FunctionCall {
        /// Fully-qualified function name (the segments after `smelt.fn.`).
        fn_id: FnId,
        /// Positional arguments, each a sub-plan node.
        args: Vec<Arc<LogicalNode>>,
        /// `true` when the function was declared with `smelt.define`
        /// (the body is available and the call can be inlined/expanded).
        /// `false` for `smelt.extern` (opaque; must be emitted as-is).
        transparent: bool,
        /// Column-level provenance for this call site.
        provenance: Provenance,
        /// Properties parsed from the function declaration's frontmatter.
        properties: FunctionProperties,
    },
    /// A reference to a named table or model output.
    TableRef {
        /// The table or model name.
        name: String,
    },
    /// A SELECT query node.
    Select {
        /// Output projection column names (Phase 30: just names, types in
        /// Phase 32+).
        projections: Vec<String>,
        /// The FROM clause source, if any.
        from: Option<Arc<LogicalNode>>,
        /// The WHERE clause predicate, if any.
        filter: Option<Arc<LogicalNode>>,
    },
    /// A literal value node; carries the inferred [`DataType`].
    Literal(DataType),
}

/// The root of a logical plan tree. A thin alias over `Arc<LogicalNode>`.
pub type Plan = Arc<LogicalNode>;

/// Parse `deterministic`, `idempotent`, and `append_only` boolean keys out of
/// a frontmatter YAML block.
///
/// This is a pure, minimal parser — we avoid pulling in a full YAML library
/// for three well-known boolean keys. Unknown keys are silently ignored.
///
/// Accepted shapes for each key:
///   `deterministic: true`   → `true`
///   `deterministic: false`  → `false`
///   absent                  → `false`
pub fn parse_function_properties(yaml_text: &str) -> FunctionProperties {
    let mut props = FunctionProperties::default();
    for line in yaml_text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("deterministic:") {
            props.deterministic = parse_bool_value(rest.trim());
        } else if let Some(rest) = trimmed.strip_prefix("idempotent:") {
            props.idempotent = parse_bool_value(rest.trim());
        } else if let Some(rest) = trimmed.strip_prefix("append_only:") {
            props.append_only = parse_bool_value(rest.trim());
        }
    }
    props
}

fn parse_bool_value(s: &str) -> bool {
    matches!(s, "true" | "yes" | "1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_properties_defaults_to_false() {
        let props = parse_function_properties("");
        assert_eq!(props, FunctionProperties::default());
    }

    #[test]
    fn parse_properties_deterministic_true() {
        let props = parse_function_properties("deterministic: true\n");
        assert!(props.deterministic);
        assert!(!props.idempotent);
        assert!(!props.append_only);
    }

    #[test]
    fn parse_properties_all_keys() {
        let yaml = "deterministic: true\nidempotent: true\nappend_only: true\n";
        let props = parse_function_properties(yaml);
        assert!(props.deterministic);
        assert!(props.idempotent);
        assert!(props.append_only);
    }

    #[test]
    fn parse_properties_ignores_unknown_keys() {
        let yaml = "backends: [duckdb]\ndeterministic: false\n";
        let props = parse_function_properties(yaml);
        assert!(!props.deterministic);
    }
}

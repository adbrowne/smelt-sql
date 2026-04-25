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
/// All fields default to `false` / `Unknown` when the corresponding frontmatter
/// key is absent or unparseable.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// The function's return type requires an explicit SQL CAST in the emitted
    /// physical plan. Declared with `needs_cast: true`.
    pub needs_cast: bool,
    /// Column-level provenance parsed from the `provenance:` frontmatter key.
    ///
    /// Remains `Provenance::Unknown` when the key is absent. The Salsa layer
    /// in `smelt-db` enforces that `provenance:` is only honoured when the
    /// workspace's `smelt.yml` has `unstable_schema: true`; if the flag is
    /// absent the field is reset to `Unknown` and a diagnostic is emitted.
    pub provenance: Provenance,
}

impl Default for FunctionProperties {
    fn default() -> Self {
        FunctionProperties {
            deterministic: false,
            idempotent: false,
            append_only: false,
            needs_cast: false,
            provenance: Provenance::Unknown,
        }
    }
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

/// Join cardinality declaration for a [`LogicalNode::LeftJoin`] node.
///
/// The planner trusts this declaration without verifying it against data
/// (§20E — declared-cardinality soundness caveat). It is the caller's
/// responsibility to ensure the declaration matches the actual data.
///
/// `OneToOne` is required by [`crate::logical_plan_rules::EliminateUnusedLeftJoin`]
/// before it may elide a join; `OneToMany` always blocks elimination because
/// such a join may multiply rows on the LHS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cardinality {
    /// Each LHS row matches at most one RHS row. Join elimination is safe when
    /// no RHS column appears in the parent's projection list.
    OneToOne,
    /// Each LHS row may match many RHS rows. Eliminating this join could change
    /// the output row count, so it is never safe to elide.
    OneToMany,
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
        /// A WHERE predicate pushed down from an enclosing `Select` node by
        /// the [`crate::logical_plan_rules::PushFilterIntoTransparentFunction`]
        /// rule (Phase 33).  `None` means no filter has been pushed yet.
        /// Once set, the rule will not push again (idempotent).
        pushed_filter: Option<Plan>,
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
    /// Marks a `FunctionCall { transparent: true }` that has already been
    /// expanded by [`crate::logical_plan_rules::ExpandTransparentFunctionCalls`].
    ///
    /// Carrying this marker prevents the expansion rule from re-visiting the
    /// same node on subsequent fixed-point passes. The node carries the same
    /// identifying information as the original `FunctionCall`.
    ExpandedCall {
        /// Fully-qualified function name.
        fn_id: FnId,
        /// Column-level provenance copied from the original `FunctionCall`.
        provenance: Provenance,
        /// Properties copied from the original `FunctionCall`.
        properties: FunctionProperties,
        /// A WHERE predicate that was pushed into the original `FunctionCall`
        /// by [`crate::logical_plan_rules::PushFilterIntoTransparentFunction`]
        /// before expansion. Carrying it through expansion keeps the evidence
        /// of pushdown visible in the final plan; `None` when no filter was
        /// pushed.
        pushed_filter: Option<Plan>,
    },
    /// Wraps an inner node with an explicit SQL CAST to `target_type`.
    ///
    /// Emitted by the expansion rule when `FunctionProperties::needs_cast` is
    /// `true` on a transparent `FunctionCall`.
    Cast {
        /// The node whose output must be cast.
        inner: Plan,
        /// The target SQL type for the CAST expression.
        target_type: DataType,
    },
    /// A LEFT JOIN between two sub-plans, produced when a function internally
    /// joins a dimension table (declared via `joins:` frontmatter metadata).
    ///
    /// # Soundness caveat (§20E)
    ///
    /// The planner trusts the declared [`Cardinality`] without verifying it
    /// against data. If the actual join is not 1:1, eliminating it via
    /// [`crate::logical_plan_rules::EliminateUnusedLeftJoin`] is incorrect.
    /// It is the caller's responsibility to ensure the declaration matches
    /// runtime behaviour.
    LeftJoin {
        /// The left-hand side (driving) sub-plan.
        lhs: Plan,
        /// The right-hand side (dimension) sub-plan.
        rhs: Plan,
        /// Column names used to match rows between LHS and RHS.
        join_columns: Vec<String>,
        /// Declared join cardinality. Only [`Cardinality::OneToOne`] permits
        /// the [`crate::logical_plan_rules::EliminateUnusedLeftJoin`] rule to
        /// remove this join.
        cardinality: Cardinality,
        /// Column names that come exclusively from the RHS (the dimension table).
        ///
        /// If none of these appear in the parent's projection list the join
        /// contributes nothing to the output and can be safely elided when
        /// `cardinality == OneToOne`.
        output_columns: Vec<String>,
    },
}

/// The root of a logical plan tree. A thin alias over `Arc<LogicalNode>`.
pub type Plan = Arc<LogicalNode>;

/// Parse `deterministic`, `idempotent`, `append_only`, and `provenance` keys
/// out of a frontmatter YAML block.
///
/// This is a pure, minimal parser — we avoid pulling in a full YAML library.
/// Unknown keys are silently ignored.
///
/// Accepted shapes:
///   `deterministic: true`                              → `true`
///   `deterministic: false`                             → `false`
///   absent                                             → `false`
///   `provenance: { col: [src.a, src.b] }`             → `Declared([("col", ["src.a", "src.b"])])`
///   `provenance:` absent                               → `Provenance::Unknown`
///
/// Note: `provenance:` is an **unstable** key. The Salsa layer in `smelt-db`
/// is responsible for enforcing the `unstable_schema: true` workspace flag and
/// resetting the provenance to `Unknown` when the flag is absent.
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
        } else if let Some(rest) = trimmed.strip_prefix("needs_cast:") {
            props.needs_cast = parse_bool_value(rest.trim());
        } else if let Some(rest) = trimmed.strip_prefix("provenance:") {
            if let Some(prov) = parse_provenance_value(rest.trim()) {
                props.provenance = prov;
            }
        }
    }
    props
}

/// Parse a `provenance:` value from a single-line inline YAML map.
///
/// Accepts the shape `{ col1: [src.a, src.b], col2: [src.c] }`.
/// Returns `None` if the input cannot be parsed as a valid provenance map.
///
/// This is intentionally minimal: it handles the specific subset of YAML
/// produced by smelt frontmatter authors (single-line inline maps). Full YAML
/// parsing would pull in an external dependency we don't need.
pub fn parse_provenance_value(s: &str) -> Option<Provenance> {
    // Must start with `{` and end with `}`
    let inner = s.strip_prefix('{')?.strip_suffix('}')?;
    let mut entries: Vec<(String, Vec<String>)> = Vec::new();

    // Split on `,` that are not inside `[…]`
    let mut depth = 0usize;
    let mut start = 0usize;
    let chars: Vec<char> = inner.chars().collect();
    let mut segments: Vec<String> = Vec::new();

    for (i, &ch) in chars.iter().enumerate() {
        match ch {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                segments.push(inner[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    // Final segment
    let tail = inner[start..].trim();
    if !tail.is_empty() {
        segments.push(tail.to_string());
    }

    for seg in segments {
        // Each segment is `col: [src.a, src.b]`
        let colon_pos = seg.find(':')?;
        let col = seg[..colon_pos].trim().to_string();
        let list_str = seg[colon_pos + 1..].trim();
        // Must be `[…]`
        let list_inner = list_str.strip_prefix('[')?.strip_suffix(']')?;
        let sources: Vec<String> = list_inner
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        entries.push((col, sources));
    }

    if entries.is_empty() {
        None
    } else {
        Some(Provenance::Declared(entries))
    }
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
        assert_eq!(props.provenance, Provenance::Unknown);
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

    #[test]
    fn parse_provenance_single_output_column() {
        let yaml = "provenance: { margin: [source.revenue, source.cost] }\n";
        let props = parse_function_properties(yaml);
        assert_eq!(
            props.provenance,
            Provenance::Declared(vec![(
                "margin".to_string(),
                vec!["source.revenue".to_string(), "source.cost".to_string()],
            )])
        );
    }

    #[test]
    fn parse_provenance_multiple_output_columns() {
        let yaml = "provenance: { a: [x.col1], b: [x.col2, x.col3] }\n";
        let props = parse_function_properties(yaml);
        assert_eq!(
            props.provenance,
            Provenance::Declared(vec![
                ("a".to_string(), vec!["x.col1".to_string()]),
                (
                    "b".to_string(),
                    vec!["x.col2".to_string(), "x.col3".to_string()]
                ),
            ])
        );
    }

    #[test]
    fn parse_provenance_absent_is_unknown() {
        let yaml = "deterministic: true\n";
        let props = parse_function_properties(yaml);
        assert_eq!(props.provenance, Provenance::Unknown);
    }

    #[test]
    fn parse_provenance_value_roundtrip() {
        let result = parse_provenance_value("{ margin: [source.revenue, source.cost] }");
        assert_eq!(
            result,
            Some(Provenance::Declared(vec![(
                "margin".to_string(),
                vec!["source.revenue".to_string(), "source.cost".to_string()],
            )]))
        );
    }
}

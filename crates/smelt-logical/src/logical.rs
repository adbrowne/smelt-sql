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

use serde::Deserialize;
use smelt_types::DataType;

// The canonical types now live in smelt-core; re-export so existing callers
// of smelt-planner that import these names continue to work unchanged.
pub use smelt_core::{DeclarationKind, FrontmatterDiagnostic, FrontmatterSeverity};

/// Fully-qualified function identifier, e.g. `"some_fn"` or `"core.math.safe_divide"`.
pub type FnId = String;

/// Provenance tag attached to nodes produced by transparent function expansion.
///
/// Phase 41 adds these tags so consumers (debug printers, diagnostic
/// renderers, planner-rule audits) can tell, for any node in the spliced
/// subtree, whether it came from the caller's argument expressions or from
/// the callee's body. The model follows §16 #12 of
/// `docs/research/20260413-smelt-functions.md` (the third "Synthesized"
/// variant from that document is left to a later phase — defaults / row-
/// erasure don't have call sites in v1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvenanceTag {
    /// The node originated in the caller (an argument expression or the
    /// surrounding call site).  Phase 41 doesn't yet have argument
    /// substitution, so this variant is reserved for future use; tests
    /// pattern-match on it.
    Caller,
    /// The node originated in the callee's body (or a nested callee's body).
    /// The `FnId` identifies the callee at the splice frame.
    Callee(FnId),
}

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
    /// Declared joins parsed from the `joins:` frontmatter key.
    ///
    /// Each entry describes a single side-joined dimension table. The raw
    /// `cardinality` string is mapped to [`Cardinality`] via
    /// [`cardinality_from_str`] at the `LogicalNode::LeftJoin` construction
    /// site.
    pub joins: Vec<JoinSpec>,
}

impl Default for FunctionProperties {
    fn default() -> Self {
        FunctionProperties {
            deterministic: false,
            idempotent: false,
            append_only: false,
            needs_cast: false,
            provenance: Provenance::Unknown,
            joins: Vec::new(),
        }
    }
}

/// A single join entry parsed from a `joins:` frontmatter list.
///
/// The `cardinality` field stores the raw string from frontmatter
/// (e.g. `"1:1"`, `"1:N"`). Convert it to a [`Cardinality`] value via
/// [`cardinality_from_str`] when constructing a `LogicalNode::LeftJoin`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinSpec {
    /// The dimension table or model name being joined to.
    pub table: String,
    /// The join condition expression (raw SQL fragment text).
    pub on: String,
    /// Raw cardinality string from frontmatter; use [`cardinality_from_str`] to convert.
    pub cardinality: String,
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

/// Maps the raw `cardinality:` frontmatter string to the [`Cardinality`] enum.
///
/// Normative, fail-safe (Semantics rule 8, D-57):
/// - `"1:1"` → [`Cardinality::OneToOne`] (the only value that enables elision).
/// - Any other string → [`Cardinality::OneToMany`] (conservative; join is kept).
///
/// No error is emitted for unrecognised strings — the fail-safe is silent.
pub fn cardinality_from_str(s: &str) -> Cardinality {
    if s == "1:1" {
        Cardinality::OneToOne
    } else {
        Cardinality::OneToMany
    }
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
        /// Phase 41: the callee's parsed body subtree, populated by the
        /// `smelt-db::logical_plan` Salsa query for transparent calls whose
        /// declaring file is reachable and whose call graph contains no
        /// cycles. `None` for opaque (`smelt.extern`) calls, for unresolved
        /// references, and for calls suppressed by cycle detection. When
        /// `None`, [`crate::logical_plan_rules::ExpandTransparentFunctionCalls`]
        /// falls back to the marker-only behaviour shipped in Phase 32.
        body: Option<Plan>,
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
        /// Phase 41: the spliced callee body subtree, recursively expanded
        /// (so a transparent call inside the body is itself an `ExpandedCall`
        /// with its own `body`). Every cloned node is wrapped in
        /// [`LogicalNode::Tagged`] with `ProvenanceTag::Callee(fn_id)`.
        /// `None` when the originating `FunctionCall.body` was `None` —
        /// either the callee was opaque, the declaration could not be
        /// resolved, or the call participated in a cycle that the
        /// `smelt-db::logical_plan` cycle pre-pass aborted.
        body: Option<Plan>,
    },
    /// Phase 41 — provenance wrapper around a node produced during transparent
    /// expansion. Carrying provenance via a wrapper node keeps the addition
    /// localised: every existing `LogicalNode` field stays untouched, and
    /// rules that don't care about provenance can recurse through `Tagged`
    /// nodes transparently.
    Tagged {
        /// Where the wrapped subtree came from (caller vs callee body).
        tag: ProvenanceTag,
        /// The wrapped subtree.
        inner: Plan,
    },
    /// Phase 41 — list-valued fragment-sort splice point.  A `SpliceList`
    /// stands in for a `SelectItems<…>` (or comparable) parameter when the
    /// expander materialises a callee body whose projection list contains
    /// such a parameter.  `[]` represents the `()` (empty) case from
    /// research §20: the surrounding adjacent commas should be elided at
    /// lowering by [`crate::logical_plan_rules::ElideEmptySelectItemsSplices`].
    /// Non-empty splices simply inline their children.
    SpliceList(Vec<Plan>),
    /// Phase 41 — opaque-text placeholder for a body subtree that has not
    /// yet been lowered into structured `LogicalNode` shapes. Used as the
    /// minimum-viable representation for transparent function bodies whose
    /// SQL grammar (`SELECT ... FROM ...`) is richer than the Phase 30
    /// node set. Subsequent phases replace `Raw` occurrences with their
    /// structured equivalents (`Select`, `Cast`, `LeftJoin`, …) as the
    /// body lowering matures.
    Raw {
        /// The raw SQL text (or fragment text) of the body subtree.
        sql_text: String,
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

/// Raw deserialisation target for a single `joins:` entry. Each field is
/// optional so a partially-shaped entry can be reported as a warning rather
/// than a hard error.
#[derive(Debug, Default, Deserialize)]
struct RawJoinSpec {
    #[serde(default)]
    table: Option<String>,
    #[serde(default)]
    on: Option<String>,
    #[serde(default)]
    cardinality: Option<serde_yaml::Value>,
}

/// Lenient serde target for the validated map returned by
/// [`smelt_core::frontmatter::parse_frontmatter`].
///
/// Only the keys applicable to function/extern declarations are present in the
/// validated map; the catalogue already rejected or warned about everything
/// else. Using `#[serde(default)]` means absent keys produce the default value
/// rather than a parse error.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawFunctionProperties {
    deterministic: bool,
    idempotent: bool,
    append_only: bool,
    needs_cast: bool,
    provenance: Option<serde_yaml::Value>,
    joins: Option<serde_yaml::Value>,
}

/// Parse the frontmatter YAML block into [`FunctionProperties`] plus a list
/// of [`FrontmatterDiagnostic`]s describing parse failures and unknown/
/// inapplicable keys.
///
/// Routes through the shared [`smelt_core::frontmatter::parse_frontmatter`]
/// catalogue so that key validation is consistent across all declaration kinds:
/// - Unknown key → `Error`
/// - Catalogue-known key inapplicable to `kind` → `Warning`
/// - Applicable key with a bad value shape → `Warning` (value skipped)
///
/// Accepted shapes for function-applicable keys:
///   * `deterministic: true | false` (also `idempotent`, `append_only`,
///     `needs_cast`)
///   * `provenance: { col: [src.a, src.b] }` (inline map)
///   * Multi-line `provenance:` with nested mappings of sequence values
///   * `joins:` as a sequence of `{ table, on, cardinality }` entries
///
/// Note: `provenance:` is **unstable**. The Salsa layer in `smelt-db` enforces
/// the `unstable_schema: true` workspace flag and resets provenance to
/// `Unknown` when the flag is absent.
pub fn parse_function_properties(
    yaml_text: &str,
    kind: DeclarationKind,
) -> (FunctionProperties, Vec<FrontmatterDiagnostic>) {
    let (validated_map, mut diags) = smelt_core::frontmatter::parse_frontmatter(yaml_text, kind);

    if validated_map.is_empty() && yaml_text.trim().is_empty() {
        // Fast path: nothing to parse.
        return (FunctionProperties::default(), diags);
    }

    let raw: RawFunctionProperties =
        match serde_yaml::from_value(serde_yaml::Value::Mapping(validated_map)) {
            Ok(r) => r,
            Err(err) => {
                diags.push(FrontmatterDiagnostic {
                    severity: FrontmatterSeverity::Error,
                    message: format!("frontmatter: failed to deserialize validated map: {err}"),
                });
                return (FunctionProperties::default(), diags);
            }
        };

    let provenance = raw
        .provenance
        .and_then(|v| parse_provenance_value(&v, &mut diags))
        .unwrap_or(Provenance::Unknown);

    let joins = raw
        .joins
        .map(|v| parse_joins_value(&v, &mut diags))
        .unwrap_or_default();

    let props = FunctionProperties {
        deterministic: raw.deterministic,
        idempotent: raw.idempotent,
        append_only: raw.append_only,
        needs_cast: raw.needs_cast,
        provenance,
        joins,
    };

    (props, diags)
}

/// Parse the `provenance:` value into [`Provenance::Declared`]. Returns
/// `None` if the value does not parse as a non-empty mapping. Malformed
/// individual entries become warnings and are skipped.
fn parse_provenance_value(
    v: &serde_yaml::Value,
    diags: &mut Vec<FrontmatterDiagnostic>,
) -> Option<Provenance> {
    let map = match v {
        serde_yaml::Value::Mapping(m) => m,
        serde_yaml::Value::Null => return None,
        other => {
            diags.push(FrontmatterDiagnostic {
                severity: FrontmatterSeverity::Warning,
                message: format!(
                    "frontmatter: ignoring `provenance`: expected a mapping, got {}",
                    yaml_value_kind(other)
                ),
            });
            return None;
        }
    };

    let mut entries: Vec<(String, Vec<String>)> = Vec::new();
    for (k, v) in map {
        let col = match k.as_str() {
            Some(s) => s.to_string(),
            None => {
                diags.push(FrontmatterDiagnostic {
                    severity: FrontmatterSeverity::Warning,
                    message: format!(
                        "frontmatter: ignoring provenance entry with non-string key {k:?}"
                    ),
                });
                continue;
            }
        };
        let sources = match v {
            serde_yaml::Value::Sequence(seq) => {
                let mut out = Vec::with_capacity(seq.len());
                let mut entry_ok = true;
                for item in seq {
                    match item.as_str() {
                        Some(s) => out.push(s.to_string()),
                        None => {
                            diags.push(FrontmatterDiagnostic {
                                severity: FrontmatterSeverity::Warning,
                                message: format!(
                                    "frontmatter: ignoring provenance entry `{col}`: source list contains non-string {item:?}"
                                ),
                            });
                            entry_ok = false;
                            break;
                        }
                    }
                }
                if entry_ok {
                    Some(out)
                } else {
                    None
                }
            }
            other => {
                diags.push(FrontmatterDiagnostic {
                    severity: FrontmatterSeverity::Warning,
                    message: format!(
                        "frontmatter: ignoring provenance entry `{col}`: expected a sequence, got {}",
                        yaml_value_kind(other)
                    ),
                });
                None
            }
        };
        if let Some(sources) = sources {
            entries.push((col, sources));
        }
    }

    if entries.is_empty() {
        None
    } else {
        Some(Provenance::Declared(entries))
    }
}

/// Parse the `joins:` value into a vector of [`JoinSpec`]s. Each entry must
/// be a mapping with `table`, `on`, and `cardinality` string keys; malformed
/// entries become warnings and are skipped.
fn parse_joins_value(
    v: &serde_yaml::Value,
    diags: &mut Vec<FrontmatterDiagnostic>,
) -> Vec<JoinSpec> {
    let seq = match v {
        serde_yaml::Value::Sequence(s) => s,
        serde_yaml::Value::Null => return Vec::new(),
        other => {
            diags.push(FrontmatterDiagnostic {
                severity: FrontmatterSeverity::Warning,
                message: format!(
                    "frontmatter: ignoring `joins`: expected a sequence, got {}",
                    yaml_value_kind(other)
                ),
            });
            return Vec::new();
        }
    };

    let mut out = Vec::with_capacity(seq.len());
    for (idx, item) in seq.iter().enumerate() {
        let raw: RawJoinSpec = match serde_yaml::from_value(item.clone()) {
            Ok(r) => r,
            Err(err) => {
                diags.push(FrontmatterDiagnostic {
                    severity: FrontmatterSeverity::Warning,
                    message: format!(
                        "frontmatter: ignoring `joins[{idx}]`: failed to parse entry: {err}"
                    ),
                });
                continue;
            }
        };
        let Some(table) = raw.table else {
            diags.push(FrontmatterDiagnostic {
                severity: FrontmatterSeverity::Warning,
                message: format!("frontmatter: ignoring `joins[{idx}]`: missing `table`"),
            });
            continue;
        };
        let Some(on) = raw.on else {
            diags.push(FrontmatterDiagnostic {
                severity: FrontmatterSeverity::Warning,
                message: format!("frontmatter: ignoring `joins[{idx}]`: missing `on`"),
            });
            continue;
        };
        let cardinality = match raw.cardinality {
            Some(serde_yaml::Value::String(s)) => s,
            Some(serde_yaml::Value::Number(n)) => n.to_string(),
            Some(other) => {
                diags.push(FrontmatterDiagnostic {
                    severity: FrontmatterSeverity::Warning,
                    message: format!(
                        "frontmatter: ignoring `joins[{idx}]`: cardinality must be a string, got {}",
                        yaml_value_kind(&other)
                    ),
                });
                continue;
            }
            // `cardinality` is optional — an absent entry means the join's
            // cardinality is unknown. Phase 51's DeclaredCardinalityUnverifiable
            // warning fires only when a non-empty string is present.
            None => String::new(),
        };
        out.push(JoinSpec {
            table,
            on,
            cardinality,
        });
    }
    out
}

fn yaml_value_kind(v: &serde_yaml::Value) -> &'static str {
    match v {
        serde_yaml::Value::Null => "null",
        serde_yaml::Value::Bool(_) => "boolean",
        serde_yaml::Value::Number(_) => "number",
        serde_yaml::Value::String(_) => "string",
        serde_yaml::Value::Sequence(_) => "sequence",
        serde_yaml::Value::Mapping(_) => "mapping",
        serde_yaml::Value::Tagged(_) => "tagged value",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// U4 test 1 — boolean keys parse via the unified catalogue path.
    #[test]
    fn parses_simple_boolean_properties() {
        // Empty input: defaults, no diagnostics.
        let (props, diags) = parse_function_properties("", DeclarationKind::Define);
        assert_eq!(props, FunctionProperties::default());
        assert!(diags.is_empty());

        // All four bools: true.
        let yaml = "deterministic: true\nidempotent: true\nappend_only: true\nneeds_cast: true\n";
        let (props, diags) = parse_function_properties(yaml, DeclarationKind::Define);
        assert!(props.deterministic);
        assert!(props.idempotent);
        assert!(props.append_only);
        assert!(props.needs_cast);
        assert_eq!(props.provenance, Provenance::Unknown);
        assert!(props.joins.is_empty());
        assert!(diags.is_empty());
    }

    /// U4 test 2 — single-line inline-map provenance still parses.
    #[test]
    fn parses_inline_provenance_map() {
        let yaml = "provenance: { margin: [source.revenue, source.cost] }\n";
        let (props, diags) = parse_function_properties(yaml, DeclarationKind::Define);
        assert_eq!(
            props.provenance,
            Provenance::Declared(vec![(
                "margin".to_string(),
                vec!["source.revenue".to_string(), "source.cost".to_string()],
            )])
        );
        assert!(diags.is_empty());
    }

    /// U4 test 3 — multi-line block-style provenance.
    #[test]
    fn parses_multi_line_provenance_map() {
        let yaml = r#"provenance:
  margin:
    - source.revenue
    - source.cost
  ratio:
    - source.numerator
    - source.denominator
"#;
        let (props, diags) = parse_function_properties(yaml, DeclarationKind::Define);
        let Provenance::Declared(entries) = props.provenance else {
            panic!("expected Declared, got {:?}", props.provenance);
        };
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "margin");
        assert_eq!(
            entries[0].1,
            vec!["source.revenue".to_string(), "source.cost".to_string()]
        );
        assert_eq!(entries[1].0, "ratio");
        assert_eq!(
            entries[1].1,
            vec![
                "source.numerator".to_string(),
                "source.denominator".to_string()
            ]
        );
        assert!(diags.is_empty());
    }

    /// U4 test 4 — `joins:` block parses into [`JoinSpec`]s.
    #[test]
    fn parses_joins_block_with_nested_map() {
        let yaml = r#"joins:
  - table: dim_customer
    on: orders.customer_id = dim_customer.customer_id
    cardinality: "1:1"
"#;
        let (props, diags) = parse_function_properties(yaml, DeclarationKind::Define);
        assert_eq!(
            props.joins,
            vec![JoinSpec {
                table: "dim_customer".to_string(),
                on: "orders.customer_id = dim_customer.customer_id".to_string(),
                cardinality: "1:1".to_string(),
            }]
        );
        assert!(diags.is_empty());
    }

    /// U4 test 5 — malformed YAML yields a default plus a single Error.
    #[test]
    fn malformed_yaml_emits_diagnostic_not_panic() {
        let yaml = "provenance: {unterminated\n";
        let (props, diags) = parse_function_properties(yaml, DeclarationKind::Define);
        assert_eq!(props, FunctionProperties::default());
        assert_eq!(diags.len(), 1, "expected one diagnostic, got {diags:?}");
        assert_eq!(diags[0].severity, FrontmatterSeverity::Error);
        assert!(
            diags[0].message.contains("frontmatter"),
            "diagnostic message should mention 'frontmatter': {}",
            diags[0].message
        );
    }

    /// U4 test 6 — unknown top-level keys are Errors (via catalogue policy),
    /// not Warnings. The rest of the known keys still parse.
    #[test]
    fn unknown_keys_produce_errors() {
        let yaml = "deterministic: true\nunknown_property: foo\n";
        let (props, diags) = parse_function_properties(yaml, DeclarationKind::Define);
        assert!(props.deterministic, "known key must still be parsed");
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == FrontmatterSeverity::Error)
            .collect();
        assert_eq!(errors.len(), 1, "expected exactly one error, got {diags:?}");
        assert!(
            errors[0].message.contains("unknown_property"),
            "error should name the unknown key: {}",
            errors[0].message
        );
    }

    /// U4 test 7 — a model-only key on a Define declaration is a Warning
    /// (inapplicable-kind), not an Error; block is retained minus that key.
    #[test]
    fn model_key_on_define_is_warning() {
        let yaml = "deterministic: true\nmaterialization: table\n";
        let (props, diags) = parse_function_properties(yaml, DeclarationKind::Define);
        assert!(props.deterministic, "function key must still parse");
        let warnings: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == FrontmatterSeverity::Warning)
            .collect();
        assert_eq!(
            warnings.len(),
            1,
            "expected one warning for the model-only key, got {diags:?}"
        );
        assert!(warnings[0].message.contains("materialization"));
    }

    /// U4 test 8 — `joins:` absent yields an empty vec.
    #[test]
    fn joins_absent_yields_empty_vec() {
        let (props, diags) = parse_function_properties("", DeclarationKind::Define);
        assert!(props.joins.is_empty());
        assert!(diags.is_empty());
    }

    /// U4 test 9 — multiple-output-columns inline provenance.
    #[test]
    fn parses_provenance_multiple_output_columns_inline() {
        let yaml = "provenance: { a: [x.col1], b: [x.col2, x.col3] }\n";
        let (props, diags) = parse_function_properties(yaml, DeclarationKind::Define);
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
        assert!(diags.is_empty());
    }

    /// U4 test 10 — provenance absent stays Unknown.
    #[test]
    fn parses_provenance_absent_is_unknown() {
        let yaml = "deterministic: true\n";
        let (props, diags) = parse_function_properties(yaml, DeclarationKind::Define);
        assert_eq!(props.provenance, Provenance::Unknown);
        assert!(props.deterministic);
        assert!(diags.is_empty());
    }

    /// U4 test 11 — Extern kind parses the same function keys cleanly.
    #[test]
    fn extern_kind_parses_function_keys() {
        let yaml = "deterministic: true\nidempotent: false\n";
        let (props, diags) = parse_function_properties(yaml, DeclarationKind::Extern);
        assert!(props.deterministic);
        assert!(!props.idempotent);
        assert!(diags.is_empty());
    }

    /// D-57 — exact `"1:1"` string maps to `OneToOne`.
    #[test]
    fn cardinality_exact_match_1_1() {
        assert_eq!(cardinality_from_str("1:1"), Cardinality::OneToOne);
    }

    /// D-57 — every string other than `"1:1"` maps to `OneToMany` (fail-safe).
    #[test]
    fn cardinality_unrecognised_never_one_to_one() {
        let cases = [
            "",
            "1:N",
            "N:1",
            "N:M",
            "one_to_one",
            "1 :1",
            "1:1 ",
            "ONE_TO_ONE",
        ];
        for s in cases {
            assert_ne!(
                cardinality_from_str(s),
                Cardinality::OneToOne,
                "'{s}' should not map to OneToOne"
            );
        }
    }
}

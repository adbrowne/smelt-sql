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
pub use smelt_core::{FrontmatterDiagnostic, FrontmatterSeverity};

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
    /// Each entry describes a single side-joined dimension table. Phase 43
    /// only parses these into a [`JoinSpec`] vector — they are not yet
    /// consumed by the logical-plan-rules pipeline. The `cardinality` field
    /// is kept as a raw string for v1; mapping into [`Cardinality`] is a
    /// future phase.
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
/// V1 representation: `cardinality` is the raw string from frontmatter
/// (e.g. `"1:1"`, `"1:N"`). Mapping to the structured [`Cardinality`] enum
/// is deferred to a later phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinSpec {
    /// The dimension table or model name being joined to.
    pub table: String,
    /// The join condition expression (raw SQL fragment text).
    pub on: String,
    /// Declared cardinality, kept as a raw string for v1.
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

/// Known top-level frontmatter keys honoured by the parser. Any other key
/// produces a [`FrontmatterSeverity::Warning`] diagnostic. Keys consumed by
/// other passes (e.g. `backends:`) live alongside the function-properties
/// keys and are not currently surfaced as warnings here — see
/// [`is_known_top_level_key`].
const KNOWN_KEYS: &[&str] = &[
    "deterministic",
    "idempotent",
    "append_only",
    "needs_cast",
    "provenance",
    "joins",
    // Other frontmatter keys consumed elsewhere (backends-narrowing check,
    // future planner extensions). Listed here so we do not warn about them.
    "backends",
    "incremental",
    "materialization",
    "tags",
];

fn is_known_top_level_key(key: &str) -> bool {
    KNOWN_KEYS.contains(&key)
}

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

/// Parse the frontmatter YAML block into [`FunctionProperties`] plus a list
/// of [`FrontmatterDiagnostic`]s describing parse failures and unknown keys.
///
/// Accepted shapes (any indentation style serde_yaml accepts):
///   * `deterministic: true | false` (also `idempotent`, `append_only`,
///     `needs_cast`)
///   * `provenance: { col: [src.a, src.b] }` (inline map)
///   * Multi-line `provenance:` with nested mappings of sequence values
///   * `joins:` as a sequence of `{ table, on, cardinality }` entries
///
/// On a top-level YAML parse failure the returned `FunctionProperties` is the
/// default and a single [`FrontmatterSeverity::Error`] diagnostic carries the
/// underlying serde_yaml error message. Unknown keys become individual
/// [`FrontmatterSeverity::Warning`] diagnostics; the rest of the YAML still
/// parses.
///
/// Note: `provenance:` is an **unstable** key. The Salsa layer in `smelt-db`
/// is responsible for enforcing the `unstable_schema: true` workspace flag and
/// resetting the provenance to `Unknown` when the flag is absent.
pub fn parse_function_properties(
    yaml_text: &str,
) -> (FunctionProperties, Vec<FrontmatterDiagnostic>) {
    let mut diags: Vec<FrontmatterDiagnostic> = Vec::new();
    let mut props = FunctionProperties::default();

    let trimmed = yaml_text.trim();
    if trimmed.is_empty() {
        return (props, diags);
    }

    let value: serde_yaml::Value = match serde_yaml::from_str(yaml_text) {
        Ok(v) => v,
        Err(err) => {
            diags.push(FrontmatterDiagnostic {
                severity: FrontmatterSeverity::Error,
                message: format!("frontmatter: failed to parse YAML: {err}"),
            });
            return (props, diags);
        }
    };

    let mapping = match value {
        serde_yaml::Value::Mapping(m) => m,
        // Empty document (`null`) is fine — same as empty input.
        serde_yaml::Value::Null => return (props, diags),
        other => {
            diags.push(FrontmatterDiagnostic {
                severity: FrontmatterSeverity::Error,
                message: format!(
                    "frontmatter: expected a top-level mapping, found {}",
                    yaml_value_kind(&other)
                ),
            });
            return (props, diags);
        }
    };

    for (k, v) in mapping {
        let key = match k.as_str() {
            Some(s) => s.to_string(),
            None => {
                diags.push(FrontmatterDiagnostic {
                    severity: FrontmatterSeverity::Warning,
                    message: format!("frontmatter: ignoring non-string key {k:?}"),
                });
                continue;
            }
        };
        match key.as_str() {
            "deterministic" => {
                if let Some(b) = parse_bool_value(&v) {
                    props.deterministic = b;
                } else {
                    diags.push(FrontmatterDiagnostic {
                        severity: FrontmatterSeverity::Warning,
                        message: format!(
                            "frontmatter: ignoring `deterministic`: expected a boolean, got {}",
                            yaml_value_kind(&v)
                        ),
                    });
                }
            }
            "idempotent" => {
                if let Some(b) = parse_bool_value(&v) {
                    props.idempotent = b;
                } else {
                    diags.push(FrontmatterDiagnostic {
                        severity: FrontmatterSeverity::Warning,
                        message: format!(
                            "frontmatter: ignoring `idempotent`: expected a boolean, got {}",
                            yaml_value_kind(&v)
                        ),
                    });
                }
            }
            "append_only" => {
                if let Some(b) = parse_bool_value(&v) {
                    props.append_only = b;
                } else {
                    diags.push(FrontmatterDiagnostic {
                        severity: FrontmatterSeverity::Warning,
                        message: format!(
                            "frontmatter: ignoring `append_only`: expected a boolean, got {}",
                            yaml_value_kind(&v)
                        ),
                    });
                }
            }
            "needs_cast" => {
                if let Some(b) = parse_bool_value(&v) {
                    props.needs_cast = b;
                } else {
                    diags.push(FrontmatterDiagnostic {
                        severity: FrontmatterSeverity::Warning,
                        message: format!(
                            "frontmatter: ignoring `needs_cast`: expected a boolean, got {}",
                            yaml_value_kind(&v)
                        ),
                    });
                }
            }
            "provenance" => {
                if let Some(prov) = parse_provenance_value(&v, &mut diags) {
                    props.provenance = prov;
                }
            }
            "joins" => {
                props.joins = parse_joins_value(&v, &mut diags);
            }
            other => {
                if !is_known_top_level_key(other) {
                    diags.push(FrontmatterDiagnostic {
                        severity: FrontmatterSeverity::Warning,
                        message: format!("frontmatter: ignoring unknown key `{other}`"),
                    });
                }
                // Known-but-not-here keys (e.g. `backends`) are silently
                // skipped — they belong to other passes.
            }
        }
    }

    (props, diags)
}

/// Decode a YAML value as a boolean, accepting both native booleans and the
/// string forms `"true"`, `"false"`, `"yes"`, `"no"`, `"1"`, `"0"` for
/// backward compatibility with the previous line-walker behaviour.
fn parse_bool_value(v: &serde_yaml::Value) -> Option<bool> {
    match v {
        serde_yaml::Value::Bool(b) => Some(*b),
        serde_yaml::Value::String(s) => match s.as_str() {
            "true" | "yes" | "1" => Some(true),
            "false" | "no" | "0" => Some(false),
            _ => None,
        },
        serde_yaml::Value::Number(n) => n.as_i64().and_then(|i| match i {
            1 => Some(true),
            0 => Some(false),
            _ => None,
        }),
        _ => None,
    }
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

    /// Phase 43 test 1 — boolean keys parse via the serde_yaml-backed path,
    /// including the new `needs_cast` key. Subsumes the historical
    /// `parse_properties_defaults_to_false`, `parse_properties_deterministic_true`,
    /// and `parse_properties_all_keys` tests.
    #[test]
    fn parses_simple_boolean_properties() {
        // Empty input: defaults, no diagnostics.
        let (props, diags) = parse_function_properties("");
        assert_eq!(props, FunctionProperties::default());
        assert!(diags.is_empty());

        // All four bools: true.
        let yaml = "deterministic: true\nidempotent: true\nappend_only: true\nneeds_cast: true\n";
        let (props, diags) = parse_function_properties(yaml);
        assert!(props.deterministic);
        assert!(props.idempotent);
        assert!(props.append_only);
        assert!(props.needs_cast);
        assert_eq!(props.provenance, Provenance::Unknown);
        assert!(props.joins.is_empty());
        assert!(diags.is_empty());
    }

    /// Phase 43 test 2 — single-line inline-map provenance still parses.
    /// Direct port of the legacy `parse_provenance_single_output_column` test.
    #[test]
    fn parses_inline_provenance_map() {
        let yaml = "provenance: { margin: [source.revenue, source.cost] }\n";
        let (props, diags) = parse_function_properties(yaml);
        assert_eq!(
            props.provenance,
            Provenance::Declared(vec![(
                "margin".to_string(),
                vec!["source.revenue".to_string(), "source.cost".to_string()],
            )])
        );
        assert!(diags.is_empty());
    }

    /// Phase 43 test 3 — multi-line block-style provenance, which the old
    /// line-walker could not handle.
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
        let (props, diags) = parse_function_properties(yaml);
        let Provenance::Declared(entries) = props.provenance else {
            panic!("expected Declared, got {:?}", props.provenance);
        };
        // serde_yaml's Mapping preserves insertion order, so we expect both
        // entries in declaration order.
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

    /// Phase 43 test 4 — `joins:` block parses into [`JoinSpec`]s with the
    /// raw cardinality string preserved.
    #[test]
    fn parses_joins_block_with_nested_map() {
        let yaml = r#"joins:
  - table: dim_customer
    on: orders.customer_id = dim_customer.customer_id
    cardinality: "1:1"
"#;
        let (props, diags) = parse_function_properties(yaml);
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

    /// Phase 43 test 5 — malformed YAML must yield a default
    /// `FunctionProperties` plus a single Error diagnostic, never panic.
    #[test]
    fn malformed_yaml_emits_diagnostic_not_panic() {
        let yaml = "provenance: {unterminated\n";
        let (props, diags) = parse_function_properties(yaml);
        assert_eq!(props, FunctionProperties::default());
        assert_eq!(diags.len(), 1, "expected one diagnostic, got {diags:?}");
        assert_eq!(diags[0].severity, FrontmatterSeverity::Error);
        assert!(
            diags[0].message.contains("frontmatter"),
            "diagnostic message should mention 'frontmatter': {}",
            diags[0].message
        );
    }

    /// Phase 43 test 6 — unknown top-level keys produce a Warning, but the
    /// rest of the frontmatter still parses cleanly. Subsumes the historical
    /// `parse_properties_ignores_unknown_keys` test's "deterministic still
    /// parses" assertion.
    #[test]
    fn unknown_keys_warned_not_errored() {
        let yaml = "deterministic: true\nunknown_property: foo\n";
        let (props, diags) = parse_function_properties(yaml);
        assert!(props.deterministic);
        let warnings: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == FrontmatterSeverity::Warning)
            .collect();
        assert_eq!(
            warnings.len(),
            1,
            "expected exactly one warning, got {diags:?}"
        );
        assert!(
            warnings[0].message.contains("unknown_property"),
            "warning should name the unknown key: {}",
            warnings[0].message
        );
    }

    /// Phase 43 test 7 — `joins:` absent yields an empty vec (regression
    /// guard for the new field's default).
    #[test]
    fn joins_absent_yields_empty_vec() {
        let (props, diags) = parse_function_properties("");
        assert!(props.joins.is_empty());
        assert!(diags.is_empty());
    }

    /// Phase 43 test 8 — multiple-output-columns inline provenance. Direct
    /// port of the legacy `parse_provenance_multiple_output_columns` test.
    #[test]
    fn parses_provenance_multiple_output_columns_inline() {
        let yaml = "provenance: { a: [x.col1], b: [x.col2, x.col3] }\n";
        let (props, diags) = parse_function_properties(yaml);
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

    /// Phase 43 test 9 — provenance absent stays Unknown. Direct port of
    /// the legacy `parse_provenance_absent_is_unknown` test.
    #[test]
    fn parses_provenance_absent_is_unknown() {
        let yaml = "deterministic: true\n";
        let (props, diags) = parse_function_properties(yaml);
        assert_eq!(props.provenance, Provenance::Unknown);
        assert!(props.deterministic);
        assert!(diags.is_empty());
    }
}

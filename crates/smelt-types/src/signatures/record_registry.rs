use super::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// ============================================================================
// Record registry (Phase E1)
// ============================================================================

/// Diagnostic code for the record registry builder.
///
/// This is a local enum living in `smelt-types` so that `signatures.rs` and
/// `build_record_registry` can produce typed sentinels without depending on
/// `smelt-db::DiagnosticCode` (which would create a circular crate dependency).
/// The wiring layer in `smelt-db` translates these into `DiagnosticCode` values
/// for the LSP accumulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordRegistryCode {
    /// A second `smelt.record` declaration in the workspace shares an existing
    /// record's name. First-declaration-wins; the sentinel anchors at the
    /// second declaration's `name_span`.
    SmeltRecordRedefinition,
    /// A field's declared type contains a meta-only witness type that is not
    /// user-writable (`ColumnRef`, `ModelRef`, `SourceRef`, `Lambda`). Anchored
    /// at the offending field's type span.
    RecordFieldTypeForbidden,
    /// A record declaration references its own name directly or transitively
    /// through other record declarations, forming a cycle. v1 records must
    /// form a DAG. Anchored at the cycle-introducing field-type span.
    RecordCyclicDeclaration,
}

/// A diagnostic produced by the record registry builder.
///
/// Carries the typed [`RecordRegistryCode`] (for pattern-matching), the source
/// span (for diagnostic anchoring), and a pre-rendered message string.
#[derive(Debug, Clone)]
pub struct DiagnosticSentinel {
    /// The registry-layer diagnostic code.
    pub code: RecordRegistryCode,
    /// Source span of the offending token (e.g. the second declaration's name
    /// or the forbidden field-type expression). May be a zero-length span when
    /// the syntactic position was not tracked.
    pub span: smelt_parser::TextRange,
    /// Pre-rendered diagnostic message per the spec's message format.
    pub message: String,
}

/// A single `smelt.record` declaration parsed from source.
///
/// Phase E1: this struct carries the declaration's name, field list (with
/// per-field type and source span), the name-token span (for
/// `SmeltRecordRedefinition` anchoring), and the source-file path (included in
/// the redefinition message).
///
/// Pure — no Salsa dependency. Produced by the Phase 2 parser and consumed by
/// `build_record_registry`.
#[derive(Debug, Clone, PartialEq)]
pub struct SmeltRecordDeclaration {
    /// The declared record name (e.g. `"SourceEntry"`).
    pub name: String,
    /// Ordered field list: `(field_name, field_type, type_span)`.
    /// `type_span` anchors `RecordFieldTypeForbidden` and
    /// `RecordCyclicDeclaration` sentinels.
    pub fields: Vec<(String, SmeltType, smelt_parser::TextRange)>,
    /// Span of the name token in `smelt.record TypeName = {…}`.
    /// Used to anchor `SmeltRecordRedefinition` at the second declaration's
    /// name token.
    pub name_span: smelt_parser::TextRange,
    /// Workspace-relative source file path for the first-declaration message.
    pub source_path: Arc<str>,
}

/// Map from declared record name to its declaration. The authoritative
/// declaration for each name (first-wins on redefinition).
///
/// Phase E1: built by `build_record_registry` and passed into the inference
/// layer (`TypeContext`) in Phase 3/5.
#[derive(Debug)]
pub struct RecordRegistry {
    inner: HashMap<String, SmeltRecordDeclaration>,
}

impl RecordRegistry {
    /// Look up a record declaration by name.
    pub fn lookup(&self, name: &str) -> Option<&SmeltRecordDeclaration> {
        self.inner.get(name)
    }

    /// All declared record names (in unspecified order).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.inner.keys().map(|s| s.as_str())
    }

    /// Create an empty registry (no declarations). Used as the default for
    /// pre-Phase-5 callers that have not wired the Salsa side yet.
    pub fn empty() -> Self {
        RecordRegistry {
            inner: HashMap::new(),
        }
    }
}

impl Default for RecordRegistry {
    fn default() -> Self {
        Self::empty()
    }
}

/// Returns `true` if the given `SmeltType` directly or transitively references
/// any of the record names in `declared_names`. Used by cycle detection to
/// identify field types that create edges in the record DAG.
fn field_type_references_record(ty: &SmeltType, declared_names: &HashSet<String>) -> Vec<String> {
    let mut refs = Vec::new();
    collect_record_references(ty, declared_names, &mut refs);
    refs
}

fn collect_record_references(
    ty: &SmeltType,
    declared_names: &HashSet<String>,
    out: &mut Vec<String>,
) {
    match ty {
        SmeltType::Record { name: Some(n), .. } if declared_names.contains(n) => {
            out.push(n.clone());
        }
        SmeltType::Record { fields, .. } => {
            for v in fields.values() {
                collect_record_references(v, declared_names, out);
            }
        }
        SmeltType::List(inner) => collect_record_references(inner, declared_names, out),
        SmeltType::Map { key, value } => {
            collect_record_references(key, declared_names, out);
            collect_record_references(value, declared_names, out);
        }
        _ => {}
    }
}

/// Build the workspace record registry from a list of parsed declarations.
///
/// **Algorithm:**
/// 1. Walk declarations in order. For each name:
///    - If already seen: emit `SmeltRecordRedefinition` at the second
///      declaration's `name_span`; skip the duplicate.
///    - Otherwise: record as authoritative.
/// 2. Validate each authoritative declaration's field types:
///    - Any field type containing `ColumnRef`, `ModelRef`, `SourceRef`, or
///      `Lambda` emits `RecordFieldTypeForbidden` at the field's type span.
/// 3. Cycle detection via DFS over the graph where nodes are declared record
///    names and edges are "field type references another declared record name":
///    - Any name reachable from itself (directly or via a chain) emits
///      `RecordCyclicDeclaration` at the introducing edge's field-type span.
///    - DFS is over the *directed* graph; back-edges (Gray → Gray in the DFS
///      coloring) detect cycles. We emit **one sentinel per cyclic target name**:
///      `cycle_emitted` is keyed on the back-edge's target, so the first
///      back-edge to a given record fires the sentinel and any subsequent
///      back-edges into the same target are suppressed. This means a single
///      record participating in several overlapping cycles (e.g. `A↔B` and
///      `A↔B↔C`) yields one sentinel per cyclic record rather than one per
///      distinct cycle path — sufficient to mark the offending records as
///      cyclic without flooding the user with overlapping reports.
///
/// **Returns:** `(RecordRegistry, Vec<DiagnosticSentinel>)`.
/// The registry contains only authoritative (first-wins) declarations.
/// The sentinel list carries redefinition, forbidden-type, and cycle errors.
///
/// Pure — no Salsa, no I/O.
pub fn build_record_registry(
    decls: &[SmeltRecordDeclaration],
) -> (RecordRegistry, Vec<DiagnosticSentinel>) {
    let mut sentinels: Vec<DiagnosticSentinel> = Vec::new();
    let mut registry_map: HashMap<String, SmeltRecordDeclaration> = HashMap::new();

    // Step 1: collect authoritative declarations (first-wins on redefinition).
    for decl in decls {
        if let Some(existing) = registry_map.get(&decl.name) {
            // Redefinition: emit sentinel anchored at the second declaration's name_span.
            sentinels.push(DiagnosticSentinel {
                code: RecordRegistryCode::SmeltRecordRedefinition,
                span: decl.name_span,
                message: format!(
                    "record `{}` is already declared in {}; record names must be unique workspace-wide",
                    decl.name,
                    existing.source_path,
                ),
            });
        } else {
            registry_map.insert(decl.name.clone(), decl.clone());
        }
    }

    // Step 2: validate field types for forbidden witnesses.
    for decl in registry_map.values() {
        for (_, field_ty, type_span) in &decl.fields {
            // Check if the field type itself is forbidden (not just recursively).
            // We check the immediate type and its components.
            if let Some(forbidden_name) = find_forbidden_type_name(field_ty) {
                sentinels.push(DiagnosticSentinel {
                    code: RecordRegistryCode::RecordFieldTypeForbidden,
                    span: *type_span,
                    message: format!(
                        "record field types may not reference {forbidden_name}; reflection witnesses are not user-writable"
                    ),
                });
            }
        }
    }

    // Step 3: cycle detection via DFS.
    // Build the adjacency graph: for each declared name, the set of other
    // declared names directly or transitively referenced in its field types.
    let declared_names: HashSet<String> = registry_map.keys().cloned().collect();

    // DFS cycle detection using iterative approach with explicit color tracking.
    // Nodes are record names (String). Colors: White=unvisited, Gray=in-stack, Black=done.
    //
    // We use String keys throughout to avoid lifetime complexity.
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Gray,
        Black,
    }

    let mut color: HashMap<String, Color> = HashMap::new();
    let mut cycle_emitted: HashSet<String> = HashSet::new();

    // Iterate in deterministic (sorted) order.
    let mut sorted_names: Vec<String> = declared_names.iter().cloned().collect();
    sorted_names.sort();

    // Iterative DFS using an explicit call stack to avoid Rust recursive fn lifetime issues.
    // Each stack frame: (node_name, edge_list_index, edge_targets).
    // We collect edges lazily per frame.
    for start in sorted_names {
        if color.get(&start).copied().unwrap_or(Color::White) != Color::White {
            continue;
        }

        // DFS stack: each entry is (node, edge_list, current_edge_index).
        type DfsEdge = (String, smelt_parser::TextRange);
        type DfsFrame = (String, Vec<DfsEdge>, usize);
        let mut dfs_stack: Vec<DfsFrame> = Vec::new();

        color.insert(start.clone(), Color::Gray);

        // Build edges for start node.
        let start_edges = {
            let mut edges: Vec<(String, smelt_parser::TextRange)> = Vec::new();
            if let Some(decl) = registry_map.get(&start) {
                for (_, field_ty, span) in &decl.fields {
                    let refs = field_type_references_record(field_ty, &declared_names);
                    for r in refs {
                        edges.push((r, *span));
                    }
                }
            }
            edges.sort_by(|a, b| a.0.cmp(&b.0));
            edges
        };
        dfs_stack.push((start, start_edges, 0));

        'dfs: while let Some(frame) = dfs_stack.last_mut() {
            let (node, edges, idx) = frame;
            if *idx >= edges.len() {
                // All edges processed — mark Black.
                let node_done = node.clone();
                dfs_stack.pop();
                color.insert(node_done, Color::Black);
                continue 'dfs;
            }

            let (target, span) = edges[*idx].clone();
            *idx += 1;

            let target_color = color.get(&target).copied().unwrap_or(Color::White);
            match target_color {
                Color::White => {
                    // Push new frame.
                    color.insert(target.clone(), Color::Gray);
                    let target_edges = {
                        let mut edges: Vec<(String, smelt_parser::TextRange)> = Vec::new();
                        if let Some(decl) = registry_map.get(&target) {
                            for (_, field_ty, fspan) in &decl.fields {
                                let refs = field_type_references_record(field_ty, &declared_names);
                                for r in refs {
                                    edges.push((r, *fspan));
                                }
                            }
                        }
                        edges.sort_by(|a, b| a.0.cmp(&b.0));
                        edges
                    };
                    dfs_stack.push((target, target_edges, 0));
                }
                Color::Gray => {
                    // Back-edge → cycle detected.
                    if !cycle_emitted.contains(&target) {
                        cycle_emitted.insert(target.clone());
                        sentinels.push(DiagnosticSentinel {
                            code: RecordRegistryCode::RecordCyclicDeclaration,
                            span,
                            message: format!(
                                "record `{target}` forms a cycle; recursive record declarations are not supported in v1"
                            ),
                        });
                    }
                }
                Color::Black => {}
            }
        }
    }

    (
        RecordRegistry {
            inner: registry_map,
        },
        sentinels,
    )
}

/// Find the name of the first forbidden type in `ty`, if any.
/// Returns the type name (`"ColumnRef"`, `"ModelRef"`, `"SourceRef"`, `"Lambda"`)
/// or `None` if no forbidden type is present.
fn find_forbidden_type_name(ty: &SmeltType) -> Option<String> {
    match ty {
        SmeltType::ColumnRef => Some("ColumnRef".to_string()),
        SmeltType::ModelRef => Some("ModelRef".to_string()),
        SmeltType::SourceRef => Some("SourceRef".to_string()),
        SmeltType::Lambda(params, _) => {
            // Also check parameter types for forbidden type references.
            params
                .iter()
                .find_map(find_forbidden_type_name)
                .or(Some("Lambda".to_string()))
        }
        SmeltType::List(inner) => find_forbidden_type_name(inner),
        SmeltType::Record { fields, .. } => fields.values().find_map(find_forbidden_type_name),
        SmeltType::Map { key, value } => {
            find_forbidden_type_name(key).or_else(|| find_forbidden_type_name(value))
        }
        _ => None,
    }
}

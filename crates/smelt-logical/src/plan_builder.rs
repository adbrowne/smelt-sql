use std::sync::Arc;

use crate::logical::{FunctionProperties, LogicalNode, Plan, Provenance};

/// Pre-gathered inputs for one `smelt.fn.*` call site, collected by the Salsa
/// query before passing to the pure plan builder.
pub struct FnCallInput {
    pub fn_id: String,
    pub transparent: bool,
    pub properties: FunctionProperties,
    /// Resolved provenance: either the declared provenance (when the workspace
    /// opted in to `unstable_schema`) or `Unknown`.
    pub provenance: Provenance,
    /// Phase 41: the callee's body text, captured eagerly by the Salsa query.
    /// `None` for opaque calls, unresolved references, and calls suppressed by
    /// the cycle pre-pass.  When `Some`, the body is attached to the
    /// `FunctionCall` plan node as a `LogicalNode::Raw { sql_text }` subtree;
    /// the Phase 41 expansion rule clones it into the resulting `ExpandedCall`.
    pub body_text: Option<String>,
}

/// Pure plan builder — takes no `db` reference and calls no Salsa queries.
///
/// Constructs a minimal `Select` root with the first collected `FunctionCall`
/// as its `from` child. Phase 32+ replaces this with a full projection tree.
pub fn build_logical_plan_pure(call_inputs: Vec<FnCallInput>) -> Plan {
    let fn_call_nodes: Vec<Arc<LogicalNode>> = call_inputs
        .into_iter()
        .map(|input| {
            let body = input
                .body_text
                .map(|t| Arc::new(LogicalNode::Raw { sql_text: t }));
            Arc::new(LogicalNode::FunctionCall {
                fn_id: input.fn_id,
                args: Vec::new(), // Phase 30 stub — arg sub-plans deferred to Phase 32+
                transparent: input.transparent,
                provenance: input.provenance,
                properties: input.properties,
                pushed_filter: None,
                body,
            })
        })
        .collect();

    if fn_call_nodes.is_empty() {
        Arc::new(LogicalNode::Select {
            projections: Vec::new(),
            from: None,
            filter: None,
        })
    } else {
        let first = fn_call_nodes.into_iter().next().unwrap();
        Arc::new(LogicalNode::Select {
            projections: Vec::new(),
            from: Some(first),
            filter: None,
        })
    }
}

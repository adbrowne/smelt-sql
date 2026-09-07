//! Workspace-wide function bodies, the function call graph, and call-cycle
//! detection. `find_function_call_cycles` is pure (Tarjan over a supplied
//! graph); the Salsa-tracked entries around it exist for incrementality.

use std::collections::HashMap;
use std::sync::Arc;

use smelt_parser::File as AstFile;

use smelt_parser::ast::SmeltPathCall;

use crate::*;

/// Phase 41: workspace-wide map from `fn_id` → body text for every
/// `smelt.define`. Opaque externs are not included (they have no body).
///
/// Salsa-tracked so the cycle pre-pass and body-attachment paths share one
/// cache entry per workspace.  The return is wrapped in `Arc` to satisfy
/// Salsa's interning / equality requirements (the same shape used by
/// `all_models`).
#[salsa::tracked]
pub(crate) fn workspace_function_bodies(
    db: &dyn salsa::Database,
    workspace: Workspace,
) -> Arc<std::collections::HashMap<String, String>> {
    let mut out = std::collections::HashMap::new();
    for f in workspace.files(db).iter().copied() {
        let parse = parse_file(db, f);
        let syntax = parse.syntax();
        let Some(ast) = AstFile::cast(syntax) else {
            continue;
        };
        for define in ast.defines() {
            let Some(name) = define.name() else { continue };
            let Some(body) = define.body() else { continue };
            let body_text = body.syntax().text().to_string();
            // First wins on duplicates — duplicate-define diagnostics catch
            // the second occurrence elsewhere.
            out.entry(name).or_insert(body_text);
        }
    }
    Arc::new(out)
}

/// Workspace-wide call graph for `smelt.define` declarations.
///
/// Returns a map from caller `fn_id` → callees (set of `fn_id`s reached from
/// the body's `smelt.functions.*` call sites). Externs and unresolved references
/// are dropped — they are sinks in the graph.  Salsa-tracked so each
/// workspace pays the walk once per parse-graph epoch.
#[salsa::tracked]
pub(crate) fn workspace_function_call_graph(
    db: &dyn salsa::Database,
    workspace: Workspace,
) -> Arc<std::collections::HashMap<String, Vec<String>>> {
    let mut out: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for f in workspace.files(db).iter().copied() {
        let parse = parse_file(db, f);
        let syntax = parse.syntax();
        let Some(ast) = AstFile::cast(syntax) else {
            continue;
        };
        for define in ast.defines() {
            let Some(caller) = define.name() else {
                continue;
            };
            let Some(body) = define.body() else { continue };
            let mut callees: Vec<String> = body
                .syntax()
                .descendants()
                .filter_map(SmeltPathCall::cast)
                .filter_map(|c| c.segments().last().cloned())
                .filter(|s| !s.is_empty())
                .collect();
            callees.sort();
            callees.dedup();
            out.entry(caller).or_insert(callees);
        }
    }
    Arc::new(out)
}

/// Phase 41 — pure DFS cycle detector over the workspace call graph.
/// Returns the set of `fn_id`s that participate in any cycle.
pub fn find_function_call_cycles(
    graph: &std::collections::HashMap<String, Vec<String>>,
) -> std::collections::HashSet<String> {
    use std::collections::HashSet;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Color {
        White,
        Grey,
        Black,
    }
    let mut color: HashMap<&str, Color> = HashMap::new();
    let mut in_cycle: HashSet<String> = HashSet::new();

    for node in graph.keys() {
        color.insert(node.as_str(), Color::White);
    }

    fn dfs<'a>(
        node: &'a str,
        graph: &'a std::collections::HashMap<String, Vec<String>>,
        color: &mut HashMap<&'a str, Color>,
        stack: &mut Vec<&'a str>,
        in_cycle: &mut HashSet<String>,
    ) {
        color.insert(node, Color::Grey);
        stack.push(node);
        if let Some(callees) = graph.get(node) {
            for callee in callees {
                let key = callee.as_str();
                match color.get(key).copied().unwrap_or(Color::White) {
                    Color::White => {
                        if graph.contains_key(callee) {
                            dfs(key, graph, color, stack, in_cycle);
                        } else {
                            // sink: not in graph
                            color.insert(key, Color::Black);
                        }
                    }
                    Color::Grey => {
                        // Found a back-edge — every Grey node from `key` to
                        // the top of `stack` is on the cycle.
                        let mut on_cycle = false;
                        for &s in stack.iter() {
                            if s == key {
                                on_cycle = true;
                            }
                            if on_cycle {
                                in_cycle.insert(s.to_string());
                            }
                        }
                    }
                    Color::Black => {}
                }
            }
        }
        stack.pop();
        color.insert(node, Color::Black);
    }

    let nodes: Vec<&str> = graph.keys().map(|s| s.as_str()).collect();
    for node in nodes {
        if matches!(
            color.get(node).copied().unwrap_or(Color::White),
            Color::White
        ) {
            let mut stack: Vec<&str> = Vec::new();
            dfs(node, graph, &mut color, &mut stack, &mut in_cycle);
        }
    }

    in_cycle
}

/// Cached union of cycle-participant `fn_id`s for the current workspace.
#[salsa::tracked]
pub(crate) fn function_call_cycle_fn_ids(
    db: &dyn salsa::Database,
    workspace: Workspace,
) -> Arc<std::collections::HashSet<String>> {
    let graph = workspace_function_call_graph(db, workspace);
    Arc::new(find_function_call_cycles(graph.as_ref()))
}

/// Phase 41 — emit [`DiagnosticCode::FunctionCallCycle`] for every
/// `smelt.define` in `file` whose `fn_id` is reachable inside a cycle in the
/// workspace call graph. Anchored at the declaration's name range.
pub fn function_call_cycle_diagnostics_for_file(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<Diagnostic> {
    let cycle_set = function_call_cycle_fn_ids(db, workspace);
    if cycle_set.is_empty() {
        return Vec::new();
    }

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let Some(ast) = AstFile::cast(syntax) else {
        return Vec::new();
    };

    let sigs = file_signature_inputs(db, file);

    let mut out = Vec::new();
    for define in ast.defines() {
        let Some(name) = define.name() else { continue };
        if !cycle_set.contains(&name) {
            continue;
        }
        let range = sigs
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.name_range)
            .unwrap_or(rowan::TextRange::empty(rowan::TextSize::from(0)));
        out.push(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!(
                "function `{name}` participates in a cyclic call graph; \
                 transparent expansion is suppressed for this function and \
                 every other function on the cycle"
            ),
            range,
            code: Some(DiagnosticCode::FunctionCallCycle),
            data: None,
        });
    }
    out
}

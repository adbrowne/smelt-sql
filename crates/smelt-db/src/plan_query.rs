//! The `logical_plan` Salsa query — a thin wrapper that gathers function-call
//! inputs and hands them to `smelt_logical::build_logical_plan_pure`.

use smelt_parser::File as AstFile;

use crate::*;

// ============================================================================
// Phase 30 — Logical plan construction
// ============================================================================
//
// The Salsa thin wrapper lives here; it gathers inputs from mixed Salsa queries
// (resolve_function, file_signature_inputs, parse_file, project_unstable_schema,
// workspace_function_bodies, function_call_cycle_fn_ids) and delegates to the
// pure builder `smelt_logical::build_logical_plan_pure` (Salsa-purity rule).

/// Build a [`smelt_logical::Plan`] from a single source file.
///
/// This tracked query gathers all Salsa inputs — the parsed AST, resolved
/// signatures, and per-declaration frontmatter — then delegates to the pure
/// helper [`smelt_logical::build_logical_plan_pure`] which takes no `db` reference.
///
/// Returns `None` when the file does not parse as a valid SQL model.
#[salsa::tracked]
pub fn logical_plan(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Option<smelt_logical::Plan> {
    use smelt_logical::Provenance;

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let ast = AstFile::cast(syntax)?;

    // Determine whether the workspace has opted in to unstable schema features.
    // Uses the Salsa-tracked ProjectInput so changes to smelt.yml invalidate
    // this query via Salsa's dependency graph (no raw filesystem I/O here).
    let project_root = file.project_root(db).clone();
    let unstable_schema = find_project(db, workspace, &project_root)
        .map(|p| project_unstable_schema(db, p))
        .unwrap_or(false);

    // Phase 41: workspace-wide body capture + cycle pre-pass.  The body map
    // lets the call-site loop attach `LogicalNode::Raw` subtrees without
    // re-walking the workspace per call; the cycle set tells us which
    // transparent calls must skip body attachment so the planner does not
    // attempt to inline a non-terminating expansion.
    let bodies = workspace_function_bodies(db, workspace);
    let cycle_set = function_call_cycle_fn_ids(db, workspace);

    // Walk the CST to collect all smelt.functions.* (path-form) call sites.
    let call_inputs: Vec<smelt_logical::FnCallInput> = ast
        .syntax()
        .descendants()
        .filter_map(smelt_parser::ast::SmeltPathCall::cast)
        .map(|call| {
            let segments = call.segments();
            let fn_id = segments.last().cloned().unwrap_or_default();

            // Per docs/specs/architecture.md → "Project isolation rule":
            // resolve only against functions declared in the same project as
            // the calling file. Multi-project workspaces (e.g. a monorepo
            // opened in VSCode) must not see cross-project signatures.
            let sig_opt = if fn_id.is_empty() {
                None
            } else {
                find_project(db, workspace, &project_root).and_then(|project| {
                    resolve_function(db, workspace, project, fn_id.clone())
                        .map(|arc| (*arc).clone())
                })
            };

            let transparent = sig_opt
                .as_ref()
                .map(|sig| sig.origin == smelt_types::SigOrigin::Define)
                .unwrap_or(false);

            // Locate the declaring file and read its frontmatter via Salsa.
            let mut properties = sig_opt
                .as_ref()
                .and_then(|_| {
                    workspace
                        .files(db)
                        .iter()
                        .copied()
                        .find(|f| {
                            file_signature_inputs(db, *f)
                                .iter()
                                .any(|s| s.name == fn_id)
                        })
                        .and_then(|decl_file| {
                            let decl_parse = parse_file(db, decl_file);
                            let decl_syntax = decl_parse.syntax();
                            let decl_ast = AstFile::cast(decl_syntax)?;
                            let decl_raw = decl_file.text(db).clone();
                            let fm_with_kind = decl_ast
                                .defines()
                                .find(|d| d.name().as_deref() == Some(fn_id.as_str()))
                                .and_then(|d| {
                                    d.frontmatter(&decl_raw)
                                        .map(|fm| (fm, smelt_core::DeclarationKind::Define))
                                })
                                .or_else(|| {
                                    decl_ast
                                        .externs()
                                        .find(|e| e.name().as_deref() == Some(fn_id.as_str()))
                                        .and_then(|e| {
                                            e.frontmatter(&decl_raw)
                                                .map(|fm| (fm, smelt_core::DeclarationKind::Extern))
                                        })
                                });
                            // Ignore frontmatter diagnostics here — they are surfaced via
                            // `provenance_unstable_diagnostics_for_file` (called from
                            // `check_file_diagnostics`), which has the declaration's name range
                            // for proper anchoring. The logical-plan path only needs the props.
                            fm_with_kind.map(|(text, kind)| {
                                smelt_logical::parse_function_properties(&text, kind).0
                            })
                        })
                })
                .unwrap_or_default();

            // Phase 31: enforce unstable_schema gate on `provenance:`.
            // If the function declared provenance but the workspace flag is
            // absent, silently return Unknown here. The diagnostic is emitted
            // by `provenance_unstable_diagnostics_for_file`, which is called
            // from `check_file_diagnostics` so it surfaces through
            // `file_diagnostics`.
            let resolved_provenance =
                if matches!(properties.provenance, Provenance::Declared(_)) && !unstable_schema {
                    Provenance::Unknown
                } else {
                    // Either the flag is set (use declared provenance) or
                    // provenance is already Unknown (pass through).
                    std::mem::replace(&mut properties.provenance, Provenance::Unknown)
                };

            // Phase 41: attach body text for transparent calls whose declaring
            // function is not in a cycle.  Opaque (`smelt.extern`) calls and
            // cycle participants leave `body_text: None` so the expansion
            // rule falls back to the marker-only behaviour from Phase 32.
            let body_text = if transparent && !cycle_set.contains(&fn_id) {
                bodies.get(&fn_id).cloned()
            } else {
                None
            };

            smelt_logical::FnCallInput {
                fn_id,
                transparent,
                properties,
                provenance: resolved_provenance,
                body_text,
            }
        })
        .collect();

    Some(smelt_logical::build_logical_plan_pure(call_inputs))
}

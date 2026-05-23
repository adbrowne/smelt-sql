//! Pre-resolved `smelt.define` body extraction for `smelt.fn.*` expansion.
//!
//! At compile time, every `smelt.fn.<name>(args)` call site in a model is
//! expanded by substituting the function body's text. The substitution table
//! is built once per project from the workspace's `smelt.define` declarations
//! and shared across every backend, so that all targets expand calls
//! identically.
//!
//! Two variants of the builder exist:
//! - [`build_fn_body_map`] reads from a Salsa `Database`, used by the run path.
//! - [`build_fn_body_map_from_model_files`] reads from plain `ModelFile`
//!   slices, used by `smelt test` and other non-Salsa contexts.
//!
//! Both produce the same shape: `function_name -> (params, body_sql)`.

use smelt_core::ModelFile;
use smelt_parser::ast::File;
use std::collections::HashMap;

/// Type for the pre-resolved function-body map:
///   fn_name → (params, body_sql)
/// where `params` is a Vec of `(param_name, optional_default_sql)` pairs in
/// declaration order.
pub type FnBodyMap = HashMap<String, (Vec<(String, Option<String>)>, String)>;

/// Walk every file in `workspace` and extract `smelt.define` bodies as a
/// [`FnBodyMap`] keyed by leaf function name.
///
/// The `(param_names, body_sql)` payload is what `SqlCompiler`'s
/// `smelt.fn.*` expander substitutes into call sites at print time. Body
/// extraction uses the parser's `DEFINE_BODY` `text_range`, which spans the
/// surrounding parens (e.g. `(CASE WHEN ... END)`); substituting a
/// parenthesised expression at the call site preserves precedence.
///
/// Pure: takes an immutable `&Database` and returns plain data. The
/// orchestration layer (`smelt-cli`'s `commands/run.rs`,
/// `commands/backbuild.rs`, and `smelt-ui`'s `run_manager.rs`) is the only
/// place that calls into Salsa to build the inputs for this helper, per the
/// pure-function rule in CLAUDE.md.
///
/// On a workspace-level duplicate function name (already a separate
/// diagnostic via `workspace_function_diagnostics`), later entries silently
/// overwrite earlier ones in iteration order. "First declaration wins" is a
/// diagnostic concern, not a runtime one.
///
/// Models without `smelt.define` declarations contribute zero entries; an
/// empty `functions/` directory yields an empty map.
pub fn build_fn_body_map(db: &smelt_db::Database, workspace: smelt_db::Workspace) -> FnBodyMap {
    let mut out: FnBodyMap = HashMap::new();
    for file in workspace.files(db).iter().copied() {
        let parse = smelt_db::parse_file(db, file);
        let Some(ast) = File::cast(parse.syntax()) else {
            continue;
        };
        // `parse_file` strips frontmatter while preserving byte offsets, so
        // text-range offsets index into either the raw or stripped text
        // identically. We use the raw `file.text(db)` here so the extracted
        // body is what users see in their source files.
        let text = file.text(db);
        for define in ast.defines() {
            let Some(name) = define.name() else { continue };
            let Some(body) = define.body() else { continue };
            let range = body.syntax().text_range();
            let start = usize::from(range.start());
            let end = usize::from(range.end());
            if end > text.len() || start > end {
                continue;
            }
            let body_sql = text[start..end].to_string();
            let params: Vec<(String, Option<String>)> = define
                .param_list()
                .map(|pl| {
                    pl.params()
                        .filter_map(|p| {
                            let pname = p.name()?;
                            // Extract the default value SQL text from the
                            // DEFAULT_VALUE node's text range in the raw source.
                            let default_sql = p.default_value().and_then(|dv| {
                                let r = dv.text_range();
                                let s = usize::from(r.start());
                                let e = usize::from(r.end());
                                if e <= text.len() && s < e {
                                    // The DEFAULT_VALUE node spans `= <expr>`;
                                    // strip the leading `=` and whitespace to get
                                    // just the expression.
                                    let raw = text[s..e].trim_start();
                                    let expr = raw.strip_prefix('=').unwrap_or(raw).trim();
                                    Some(expr.to_string())
                                } else {
                                    None
                                }
                            });
                            Some((pname, default_sql))
                        })
                        .collect()
                })
                .unwrap_or_default();
            out.insert(name, (params, body_sql));
        }
    }
    out
}

/// Like [`build_fn_body_map`] but operates on plain `ModelFile` slices without
/// requiring a Salsa database.  Used by `smelt test` to expand function call
/// nodes in test SQL without the full run-command infrastructure.
///
/// Text ranges are indexed into `file.content` (the raw source including
/// frontmatter), mirroring the logic in [`build_fn_body_map`] which uses
/// `file.text(db)` on the raw Salsa-stored text.
pub fn build_fn_body_map_from_model_files(files: &[ModelFile]) -> FnBodyMap {
    let mut out: FnBodyMap = HashMap::new();
    for model_file in files {
        let text = &model_file.content;
        let parse = smelt_parser::parse(text);
        let Some(ast) = File::cast(parse.syntax()) else {
            continue;
        };
        for define in ast.defines() {
            let Some(name) = define.name() else { continue };
            let Some(body) = define.body() else { continue };
            let range = body.syntax().text_range();
            let start = usize::from(range.start());
            let end = usize::from(range.end());
            if end > text.len() || start > end {
                continue;
            }
            let body_sql = text[start..end].to_string();
            let params: Vec<(String, Option<String>)> = define
                .param_list()
                .map(|pl| {
                    pl.params()
                        .filter_map(|p| {
                            let pname = p.name()?;
                            let default_sql = p.default_value().and_then(|dv| {
                                let r = dv.text_range();
                                let s = usize::from(r.start());
                                let e = usize::from(r.end());
                                if e <= text.len() && s < e {
                                    let raw = text[s..e].trim_start();
                                    let expr = raw.strip_prefix('=').unwrap_or(raw).trim();
                                    Some(expr.to_string())
                                } else {
                                    None
                                }
                            });
                            Some((pname, default_sql))
                        })
                        .collect()
                })
                .unwrap_or_default();
            out.insert(name, (params, body_sql));
        }
    }
    out
}

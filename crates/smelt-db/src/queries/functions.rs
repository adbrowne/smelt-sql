//! Per-file function signature / body / resolution queries.
//!
//! Per §20H of `docs/research/20260413-smelt-functions.md`, these queries are
//! split so that body edits do not invalidate signature consumers.

use std::sync::Arc;

use smelt_parser::{self, File as AstFile};
use smelt_types::signatures::{extract_function_signatures_with_raw, FunctionSig};

use crate::queries::parse::parse_file;
use crate::{SourceFile, Workspace};

// ============================================================================
// Function signature index (Phase 3, smelt-functions Step 1)
// ============================================================================
//
// Per §20H of `docs/research/20260413-smelt-functions.md`, signature lookups
// (used by downstream type-checking) must not be invalidated by edits to a
// function *body*. Split:
//   - `file_signature_inputs` / `functions_in_file` — signatures only. Its
//     return value is content-equal across body-only edits, so Salsa's
//     by-value backdating stops the re-run cascade at the boundary.
//   - `function_body` — CST of the body expression, re-computed on any edit
//     but independent of the signature query.
//
// All of these are thin wrappers over the pure
// `smelt_types::signatures::extract_function_signatures` function — per the
// pure-function rule in CLAUDE.md.

/// Extract function signatures from a single file. Pure-function wrapper
/// around `smelt_types::signatures::extract_function_signatures`.
///
/// This query's output only changes when *signature* tokens change. Body
/// edits do not affect the returned `Vec<FunctionSig>`, so Salsa's durability
/// check prevents downstream consumers from re-running. This is the §20H
/// invalidation hinge.
#[salsa::tracked]
pub fn file_signature_inputs(db: &dyn salsa::Database, file: SourceFile) -> Arc<Vec<FunctionSig>> {
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let text_raw = file.text(db);
    let clean_text = smelt_parser::strip_frontmatter(text_raw);
    if let Some(ast) = AstFile::cast(syntax) {
        Arc::new(extract_function_signatures_with_raw(
            &ast,
            &clean_text,
            text_raw,
        ))
    } else {
        Arc::new(Vec::new())
    }
}

/// All function signatures declared in `file`, in declaration order.
///
/// Exposed as a distinct public name from `file_signature_inputs` per the
/// plan; internally it is the same query.
#[salsa::tracked]
pub fn functions_in_file(db: &dyn salsa::Database, file: SourceFile) -> Arc<Vec<FunctionSig>> {
    file_signature_inputs(db, file)
}

/// Look up a single function's signature by name within one file.
///
/// Memoized by `(file, name)`. Re-uses `file_signature_inputs` so edits to
/// other declarations in the same file don't necessarily invalidate this
/// lookup either (though Salsa's current implementation cannot detect that
/// granularity — it still goes through `file_signature_inputs`'s output).
#[salsa::tracked]
pub fn function_signature(
    db: &dyn salsa::Database,
    file: SourceFile,
    name: String,
) -> Option<Arc<FunctionSig>> {
    let sigs = file_signature_inputs(db, file);
    sigs.iter()
        .find(|s| s.name == name)
        .map(|s| Arc::new(s.clone()))
}

/// Byte range of a function body in the stripped source text.
///
/// Rowan's `SyntaxNode` is `!Send`, so we cannot store it in a Salsa tracked
/// output directly. Instead, this query returns the byte range of the body
/// within the parsed (frontmatter-stripped) source. Callers can re-parse or
/// re-read the CST via `parse_file` and locate the body using this range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BodyRange {
    /// Inclusive start byte offset into the stripped source.
    pub start: u32,
    /// Exclusive end byte offset into the stripped source.
    pub end: u32,
}

/// Byte range of `name`'s body in `file`'s stripped source text, if any.
///
/// Depends directly on `parse_file` — not on `file_signature_inputs` — so
/// that body-only edits invalidate this query without invalidating the
/// signature query. (A body edit changes the `Parse` output, which changes
/// the body's text range if body length changed, and re-parsing anyway
/// — in practice this query re-computes on any file edit. The invariant
/// that matters is the asymmetric direction: `function_signature`
/// is *not* invalidated by body edits.)
#[salsa::tracked]
pub fn function_body(
    db: &dyn salsa::Database,
    file: SourceFile,
    name: String,
) -> Option<BodyRange> {
    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let ast = AstFile::cast(syntax)?;
    for define in ast.defines() {
        if define.name().as_deref() == Some(name.as_str()) {
            let body = define.body()?;
            let range = body.syntax().text_range();
            return Some(BodyRange {
                start: u32::from(range.start()),
                end: u32::from(range.end()),
            });
        }
    }
    None
}

/// Resolve a function name to the first matching `FunctionSig` declared
/// **inside `project`**. Files are enumerated in sorted-by-path order for
/// deterministic results when a project declares the same name twice.
///
/// Project-scoped per `docs/specs/architecture.md` → "Project isolation
/// rule": a workspace folder may contain multiple smelt projects, and each
/// project is a closed resolution scope. Callers thread the project
/// through from the file under analysis
/// (`source_file.project_root(db)` → `find_project(workspace, root)`).
#[salsa::tracked]
pub fn resolve_function(
    db: &dyn salsa::Database,
    workspace: Workspace,
    project: crate::ProjectInput,
    name: String,
) -> Option<Arc<FunctionSig>> {
    let project_root = project.root(db);
    let mut files: Vec<SourceFile> = workspace
        .files(db)
        .iter()
        .copied()
        .filter(|f| f.project_root(db) == project_root)
        .collect();
    files.sort_by(|a, b| a.path(db).cmp(b.path(db)));
    for f in files {
        let sigs = file_signature_inputs(db, f);
        if let Some(sig) = sigs.iter().find(|s| s.name == name) {
            return Some(Arc::new(sig.clone()));
        }
    }
    None
}

/// Byte range of a function's name token within the file's stripped source.
///
/// Powers LSP goto-definition: clicking `sessionize` in
/// `smelt.functions.sessionize(...)` lands the cursor on the `sessionize`
/// identifier inside `smelt.define sessionize(...)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NameRange {
    pub start: u32,
    pub end: u32,
}

/// Resolve a function name to the file that declares it (within `project`)
/// and the byte range of the name token within that file's stripped source.
///
/// Project-scoped — see [`resolve_function`] for the rationale. Iterates
/// the project's files in the same sorted-by-path order as `resolve_function`
/// so the same file wins on intra-project collisions.
#[salsa::tracked]
pub fn resolve_function_path(
    db: &dyn salsa::Database,
    workspace: Workspace,
    project: crate::ProjectInput,
    name: String,
) -> Option<(SourceFile, NameRange)> {
    let project_root = project.root(db);
    let mut files: Vec<SourceFile> = workspace
        .files(db)
        .iter()
        .copied()
        .filter(|f| f.project_root(db) == project_root)
        .collect();
    files.sort_by(|a, b| a.path(db).cmp(b.path(db)));
    for f in files {
        let parse = parse_file(db, f);
        let Some(ast) = AstFile::cast(parse.syntax()) else {
            continue;
        };
        for define in ast.defines() {
            if define.name().as_deref() == Some(name.as_str()) {
                if let Some(range) = define.name_range() {
                    return Some((
                        f,
                        NameRange {
                            start: u32::from(range.start()),
                            end: u32::from(range.end()),
                        },
                    ));
                }
            }
        }
    }
    None
}

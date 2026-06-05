//! Diagnostic-parity gate (analysis ↔ build).
//!
//! `gate_diagnostics` is the single shared pre-execution check both run paths
//! (the CLI `run`/`build` command and [`crate::execute_project`]) call before
//! compiling or executing any model. It runs the *same* analyzer surface the
//! LSP publishes — [`smelt_db::file_diagnostics`] plus
//! [`smelt_db::check_type_diagnostics`] — over the given files and refuses the
//! build on any `Error`-severity diagnostic. The blocking set is exactly
//! `severity == Error` (not a code allow-list), so a workspace the editor flags
//! red is a workspace `smelt build` rejects.
//!
//! Spec: `docs/specs/architecture.md` §"Diagnostic parity rule (analysis ↔
//! build)".
//!
//! This is also the build gate's single `TextRange → (line, column)` conversion
//! boundary, backed by [`line_index::LineIndex`] (Diagnostic range encoding
//! rule): callers print [`GateDiagnostic`]s, which already carry resolved
//! 1-based positions.

use std::fmt;
use std::path::{Path, PathBuf};

use line_index::LineIndex;
use smelt_db::{
    check_type_diagnostics, file_diagnostics, project_address_collisions,
    project_source_diagnostics, Database, Diagnostic, DiagnosticAcc, DiagnosticCode,
    DiagnosticSeverity, Workspace,
};

/// A single `Error`-severity diagnostic that blocks the build, with its source
/// position already resolved to 1-based `(line, column)`.
#[derive(Debug, Clone)]
pub struct GateDiagnostic {
    /// Path of the file the diagnostic was reported against.
    pub path: PathBuf,
    /// 1-based line of the diagnostic's start.
    pub line: u32,
    /// 1-based column of the diagnostic's start.
    pub col: u32,
    /// The diagnostic code, when the producer assigned one.
    pub code: Option<DiagnosticCode>,
    /// Human-readable message.
    pub message: String,
}

impl fmt::Display for GateDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = match &self.code {
            Some(c) => format!("[{c:?}]"),
            None => String::new(),
        };
        write!(
            f,
            "{}:{}:{}: error{code}: {}",
            self.path.display(),
            self.line,
            self.col,
            self.message
        )
    }
}

/// Collect every `Error`-severity analyzer diagnostic over `files` (the LSP
/// surface: `file_diagnostics` + `check_type_diagnostics`). Returns `Ok(())`
/// when none are found, otherwise the aggregated blocking set with positions
/// resolved once here.
///
/// `files` are the source-file paths to gate — both the selected materializable
/// models and any function-definition files (function-body diagnostics such as
/// `CteCycle` are reported against the `smelt.define` file, not its callers).
/// Paths not registered in `db` are skipped (the caller's superset of paths is
/// tolerated).
pub fn gate_diagnostics(
    db: &Database,
    workspace: Workspace,
    files: &[PathBuf],
) -> Result<(), Vec<GateDiagnostic>> {
    let mut errors: Vec<GateDiagnostic> = Vec::new();

    for path in files {
        let Some(file) = db.source_file(path) else {
            continue;
        };
        let text = file.text(db);
        let line_index = LineIndex::new(text);

        let mut diags: Vec<Diagnostic> = file_diagnostics(db, workspace, file);
        diags.extend(
            check_type_diagnostics::accumulated::<DiagnosticAcc>(db, workspace, file)
                .into_iter()
                .map(|d| d.0.clone()),
        );

        for d in diags {
            if d.severity != DiagnosticSeverity::Error {
                continue;
            }
            let lc = line_index.line_col(d.range.start());
            errors.push(GateDiagnostic {
                path: path.clone(),
                line: lc.line + 1,
                col: lc.col + 1,
                code: d.code,
                message: d.message,
            });
        }
    }

    // Per-entity source YAML diagnostics are project-scoped: the `.yml` files
    // are not `SourceFile` inputs, so a malformed source is invisible to the
    // per-file loop above. Gate the sources of every project that owns at least
    // one gated file (so `--select`-ing a model still enforces its project's
    // sources, but an unrelated project in a multi-project workspace is not
    // dragged in). A malformed source surfaces here as a `MalformedSource` /
    // `SourceTypeError` Error (BUG-032; `architecture.md` §"Diagnostic parity
    // rule"). These diagnostics are anchored at the source file head (offset 0),
    // so `(line, col)` is unconditionally `(1, 1)`.
    for project in workspace.projects(db).iter().copied() {
        let root: &Path = project.root(db).as_path();
        if !files.iter().any(|f| f.starts_with(root)) {
            continue;
        }
        for sd in project_source_diagnostics(db, project).iter() {
            if sd.diagnostic.severity != DiagnosticSeverity::Error {
                continue;
            }
            errors.push(GateDiagnostic {
                path: sd.path.clone(),
                line: 1,
                col: 1,
                code: sd.diagnostic.code,
                message: sd.diagnostic.message.clone(),
            });
        }
        // Address-collision diagnostics: same project scope, same gating rule.
        for cd in project_address_collisions(db, project).iter() {
            if cd.diagnostic.severity != DiagnosticSeverity::Error {
                continue;
            }
            errors.push(GateDiagnostic {
                path: cd.path.clone(),
                line: 1,
                col: 1,
                code: cd.diagnostic.code,
                message: cd.diagnostic.message.clone(),
            });
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Format an aggregated blocking set into a single multi-line error message
/// suitable for `anyhow::anyhow!`. Each diagnostic is printed as
/// `path:line:col: error[Code]: message`.
pub fn format_gate_errors(errors: &[GateDiagnostic]) -> String {
    let mut out = format!(
        "Build refused: {} Error-severity diagnostic(s) — `smelt build` enforces \
         the same analyzer surface the LSP reports (see `smelt docs show \
         concepts/diagnostics`).",
        errors.len()
    );
    for e in errors {
        out.push('\n');
        out.push_str(&e.to_string());
    }
    out
}

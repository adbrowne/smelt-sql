//! Phase 43 regression — frontmatter parse-error / unknown-key diagnostics
//! must surface even on workspaces that opt into `unstable_schema: true`.
//!
//! Before Phase 43 was reworked, the diagnostic emit path was bundled into
//! `provenance_unstable_diagnostics_for_file`, which short-circuits when
//! `unstable_schema` is `true`. As a result, the `examples/functions_demo`
//! workspace (and any production workspace using the unstable schema flag)
//! lost all malformed-YAML / unknown-key warnings on `smelt.define` and
//! `smelt.extern` declarations.
//!
//! These tests pin the corrected behaviour: the new pure helper
//! `frontmatter_parse_diagnostics_for_file` runs unconditionally, decoupled
//! from the unstable-schema gate.

use std::path::PathBuf;

use smelt_db::{
    file_diagnostics, Database, Diagnostic, DiagnosticCode, DiagnosticSeverity, SourceFile,
    Workspace,
};

/// Build a fresh `Database` seeded with a project (whose `smelt.yml` is
/// passed verbatim) and the given source files.
fn build_db(
    project_root: PathBuf,
    smelt_yml: &str,
    files: &[(PathBuf, &str)],
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());
    db.set_project_smelt_yml(&project_root, smelt_yml.to_string());

    let mut handles = Vec::with_capacity(files.len());
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), (*content).to_string(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

fn diags_with_code(
    db: &Database,
    ws: Workspace,
    file: SourceFile,
    code: DiagnosticCode,
) -> Vec<Diagnostic> {
    file_diagnostics(db, ws, file)
        .into_iter()
        .filter(|d| d.code == Some(code))
        .collect()
}

/// `smelt.yml` text that opts the project into the unstable-schema feature.
/// Phase 43's regression hinges on parse diagnostics still firing under this
/// configuration — exactly what `examples/functions_demo/smelt.yml` uses.
const UNSTABLE_SMELT_YML: &str = "name: phase43_regression
version: 1
model_paths:
  - models
targets:
  dev:
    type: duckdb
    schema: main
default_materialization: view
unstable_schema: true
";

#[test]
fn malformed_frontmatter_yaml_under_unstable_schema_emits_error() {
    // Mirrors `examples/broken/models/fn_frontmatter_malformed.sql` — the
    // YAML has an unterminated flow-sequence so serde_yaml fails the parse
    // outright. The fix must surface a `FrontmatterParseError` even though
    // the workspace has `unstable_schema: true` set.
    let root = PathBuf::from("/fake/phase43_malformed");
    let fn_path = root.join("models").join("fn_malformed.sql");
    let fn_src = "\
---
deterministic: true
provenance: { margin: [source.revenue, source.cost
---
smelt.define fn_malformed(source: TableExpr<{revenue: Numeric, cost: Numeric}>) -> TableExpr AS (
  SELECT source.*, revenue - cost AS margin FROM source
)
";

    let (db, ws, files) = build_db(root, UNSTABLE_SMELT_YML, &[(fn_path, fn_src)]);
    let file = files[0];

    let parse_diags = diags_with_code(&db, ws, file, DiagnosticCode::FrontmatterParseError);
    assert!(
        parse_diags
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error),
        "expected at least one Error-severity FrontmatterParseError; got {:#?}",
        file_diagnostics(&db, ws, file)
    );
}

#[test]
fn unknown_frontmatter_key_under_unstable_schema_emits_warning() {
    // Mirrors `examples/broken/models/fn_frontmatter_unknown_key.sql` — the
    // YAML parses cleanly but contains an `unknown_property:` key that's not
    // in the allow-list. The parser surfaces a Warning-severity diagnostic
    // and the rest of the frontmatter is honoured. This must still fire on
    // `unstable_schema: true` workspaces.
    let root = PathBuf::from("/fake/phase43_unknown_key");
    let fn_path = root.join("models").join("fn_unknown_key.sql");
    let fn_src = "\
---
deterministic: true
unknown_property: foo
---
smelt.define fn_unknown_key(source: TableExpr<{revenue: Numeric, cost: Numeric}>) -> TableExpr AS (
  SELECT source.*, revenue - cost AS margin FROM source
)
";

    let (db, ws, files) = build_db(root, UNSTABLE_SMELT_YML, &[(fn_path, fn_src)]);
    let file = files[0];

    let parse_diags = diags_with_code(&db, ws, file, DiagnosticCode::FrontmatterParseError);
    assert!(
        parse_diags
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Warning),
        "expected at least one Warning-severity FrontmatterParseError; got {:#?}",
        file_diagnostics(&db, ws, file)
    );
}

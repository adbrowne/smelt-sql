//! Parse + reference extraction queries (Salsa-tracked wrappers around
//! `smelt-parser`).
//!
//! These queries form the bottom layer of the smelt-db query graph: every
//! semantic query parses the file through one of these wrappers so the parse
//! result is cached once per file revision.

use std::path::PathBuf;
use std::sync::Arc;

use smelt_core::metadata::{extract_file_metadata, FileMetadata};
use smelt_parser::{self, ast::SmeltPathRef, File as AstFile};

use crate::{Model, SourceFile, SourceLocation};

// ============================================================================
// Syntax queries
// ============================================================================

#[salsa::tracked(returns(ref))]
pub fn parse_file(db: &dyn salsa::Database, file: SourceFile) -> smelt_parser::Parse {
    let text = file.text(db);

    // Generator files route their body through the meta-language expression
    // parser rather than the SQL SELECT parser.  We detect the generator
    // variant here so that `parse_file` always returns the correct CST.
    match extract_file_metadata(text) {
        Ok(FileMetadata::Generator { body_offset, .. }) => {
            // Replace frontmatter with comment lines (preserving byte offsets)
            // and parse the body as a meta-language expression.
            let stripped = smelt_parser::strip_frontmatter(text);
            smelt_parser::parse_meta_expression_from_offset(&stripped, body_offset)
        }
        _ => {
            // Standard path: strip frontmatter (replacing with -- comments)
            // and parse as a SQL / smelt-define file.
            let clean_text = smelt_parser::strip_frontmatter(text);
            smelt_parser::parse(&clean_text)
        }
    }
}

#[salsa::tracked]
pub fn parse_model(db: &dyn salsa::Database, file: SourceFile) -> Option<Arc<Model>> {
    let path = file.path(db).clone();
    // Extract model name: from virtual path suffix (multi-model) or file stem (single-model)
    let path_str = path.to_str().unwrap_or("");
    let (model_name, source_path) = if let Some((file_part, name)) = path_str.rsplit_once("::") {
        (name.to_string(), PathBuf::from(file_part))
    } else {
        (path.file_stem()?.to_str()?.to_string(), path.clone())
    };

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    let ast = AstFile::cast(syntax)?;
    // A valid model body is either a SELECT_STMT or a PIPE_QUERY (FROM-first pipe query).
    if !ast.has_query_body() {
        return None;
    }

    Some(Arc::new(Model {
        name: model_name,
        path,
        source_path,
    }))
}

/// Path-form refs (`smelt.<path>` in value position) extracted with
/// their resolution metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathRefLocation {
    pub path: Vec<String>,
    pub range: rowan::TextRange,
    /// True when this ref appears in a `TableExpr` (FROM/JOIN) position.
    /// Phase 2a uses this to gate kind-mismatch diagnostics: a path
    /// resolving to a `Test` is invalid in `TableExpr` positions.
    pub in_table_expr_position: bool,
}

/// Extract every unified `smelt.<path>` value-form ref from a file.
///
/// Surfaces the unified path-form value refs (no `(`) so the diagnostic
/// pass can validate them through [`resolve_ref_path`]. Call-form path
/// refs are not included here — they're consumed by the function-call
/// pipeline.
#[salsa::tracked]
pub fn model_path_refs(db: &dyn salsa::Database, file: SourceFile) -> Arc<Vec<PathRefLocation>> {
    let parse = parse_file(db, file);
    let syntax = parse.syntax();

    let Some(ast) = AstFile::cast(syntax) else {
        return Arc::new(Vec::new());
    };

    let mut out = Vec::new();
    for path_ref in ast.syntax().descendants().filter_map(SmeltPathRef::cast) {
        // Skip nested path refs that are part of a SmeltPathCall.
        if path_ref
            .syntax()
            .ancestors()
            .skip(1)
            .any(|a| smelt_parser::ast::SmeltPathCall::cast(a).is_some())
        {
            continue;
        }
        let in_table_expr_position = is_in_table_expr_position(path_ref.syntax());
        let path = path_ref.segments();
        let range = path_ref.text_range();
        out.push(PathRefLocation {
            path,
            range,
            in_table_expr_position,
        });
    }
    Arc::new(out)
}

/// True when this `SMELT_PATH_REF` node sits in a `TableExpr` position
/// — i.e. it's the body of a TableRef under a FROM/JOIN clause.
fn is_in_table_expr_position(node: &smelt_parser::syntax_kind::SyntaxNode) -> bool {
    use smelt_parser::syntax_kind::SyntaxKind as Sk;
    node.ancestors()
        .any(|a| matches!(a.kind(), Sk::TABLE_REF | Sk::FROM_CLAUSE | Sk::JOIN_CLAUSE))
}

/// Legacy `smelt.sources.schema.table` locations.
///
/// Phase 4: `smelt.source()` is now a parse error; this query always returns
/// an empty vec. Kept so callers compile without changes during the migration.
/// Will be removed in a follow-up cleanup.
#[salsa::tracked]
pub fn model_sources(db: &dyn salsa::Database, file: SourceFile) -> Arc<Vec<SourceLocation>> {
    // smelt.source() is a parse error — no legacy SourceCall nodes can appear.
    // intentionally ignored: suppress unused parameter warnings on this migration stub.
    let _ = (db, file);
    Arc::new(Vec::new())
}

#[cfg(test)]
mod tests {
    use line_index::LineIndex;
    use smelt_parser::SyntaxKind;

    use crate::{Database, DiagnosticCode, SourceFile};

    /// Lightweight position for test assertions.
    #[derive(Debug, Clone, Copy)]
    struct LcPos {
        pub line: u32,
        pub column: u32,
    }

    /// Lightweight range for test assertions.
    #[derive(Debug, Clone, Copy)]
    struct Lc {
        pub start: LcPos,
        pub end: LcPos,
    }

    /// Convert a `TextRange` (byte offsets) to `(line, col)` for assertion
    /// readability. Uses `LineIndex` (byte-based columns; ASCII-safe).
    fn lc(source: &str, range: &rowan::TextRange) -> Lc {
        let li = LineIndex::new(source);
        let s = li.line_col(range.start());
        let e = li.line_col(range.end());
        Lc {
            start: LcPos {
                line: s.line,
                column: s.col,
            },
            end: LcPos {
                line: e.line,
                column: e.col,
            },
        }
    }

    /// Build a minimal Salsa database with one source file and return the
    /// file handle. Does not register a workspace or project — the tests in
    /// this module only call `parse_file` directly, which requires no workspace.
    fn make_db_with_file(text: &str) -> (Database, SourceFile) {
        let db = Database::default();
        let path = std::path::PathBuf::from("models/gen.gen.sql");
        let file = SourceFile::new(&db, path, text.to_string(), std::path::PathBuf::from("."));
        (db, file)
    }

    /// Return the diagnostics for `file` via `file_diagnostics`, which requires a
    /// workspace. Registers a minimal workspace with a single project.
    fn diagnostics_for(text: &str) -> Vec<crate::Diagnostic> {
        let db = Database::default();
        let path = std::path::PathBuf::from("models/gen.gen.sql");
        let file = SourceFile::new(
            &db,
            path.clone(),
            text.to_string(),
            std::path::PathBuf::from("."),
        );
        let project = crate::ProjectInput::new(
            &db,
            std::path::PathBuf::from("."),
            String::new(),
            String::new(),
        );
        let ws = crate::Workspace::new(&db, vec![file], vec![project], vec![], None, vec![]);
        crate::file_diagnostics(&db, ws, file)
    }

    // ── Generator file routing tests ─────────────────────────────────────────

    /// A generator file whose body is a list literal parses with a root body
    /// node of kind `ARRAY_LITERAL` (the CST kind for `[...]`), not a
    /// `SELECT_STMT`.
    #[test]
    fn parse_generator_file_routes_to_meta_language_body() {
        let source = "---\ngenerates: models\n---\n[{name: 'us_west', body: SELECT * FROM orders}]";
        let (db, file) = make_db_with_file(source);
        let parse = super::parse_file(&db, file);
        let syntax = parse.syntax();

        // The CST must contain an ARRAY_LITERAL node (the `[...]` list body).
        let has_array_literal = syntax
            .descendants()
            .any(|n| n.kind() == SyntaxKind::ARRAY_LITERAL);
        assert!(
            has_array_literal,
            "generator file body must produce an ARRAY_LITERAL node, not a SELECT_STMT"
        );

        // Must NOT contain a SELECT_STMT at the top level of the body
        // (the SELECT inside the record value field is fine, but the outer
        // root expression must not be a SELECT_STMT).
        let root_children_kinds: Vec<_> = syntax.children().map(|n| n.kind()).collect();
        assert!(
            !root_children_kinds.contains(&SyntaxKind::SELECT_STMT),
            "generator file must not have a top-level SELECT_STMT as root child"
        );

        // Zero parse errors expected at the generator body level.
        let errors: Vec<_> = parse.errors.iter().collect();
        assert!(
            errors.is_empty(),
            "expected zero parser errors for generator body, got: {:?}",
            errors
        );
    }

    /// A generator file whose body is a HOF chain produces a `PIPE_EXPR` root.
    #[test]
    fn parse_generator_file_with_hof_chain_in_body() {
        let source = "---\ngenerates: models\n---\nsmelt.config.load_yaml('c.yaml', Cohort) |> map(fn c => {name: c.name, body: SELECT * FROM orders})";
        let (db, file) = make_db_with_file(source);
        let parse = super::parse_file(&db, file);
        let syntax = parse.syntax();

        // Root must contain a PIPE_EXPR.
        let has_pipe_expr = syntax
            .descendants()
            .any(|n| n.kind() == SyntaxKind::PIPE_EXPR);
        assert!(
            has_pipe_expr,
            "generator file HOF chain must produce a PIPE_EXPR node"
        );

        // The lambda body should contain a RECORD_LITERAL.
        let has_record_literal = syntax
            .descendants()
            .any(|n| n.kind() == SyntaxKind::RECORD_LITERAL);
        assert!(
            has_record_literal,
            "HOF chain body must contain a RECORD_LITERAL node"
        );
    }

    /// A generator file with a top-level bare `SELECT` emits
    /// `GenerateFileBareSelectForbidden` anchored at the `SELECT` keyword token.
    #[test]
    fn parse_generator_file_with_top_level_select_emits_bare_select_forbidden() {
        let source = "---\ngenerates: models\n---\nSELECT * FROM orders";
        let diags = diagnostics_for(source);
        let bare_select_diag = diags
            .iter()
            .find(|d| d.code == Some(DiagnosticCode::GenerateFileBareSelectForbidden));
        assert!(
            bare_select_diag.is_some(),
            "expected GenerateFileBareSelectForbidden diagnostic, got: {:?}",
            diags
        );
        // Anchor precision: must cover only the `SELECT` keyword token (line 3,
        // column 0, length 6), not the full statement.
        let range = &bare_select_diag.unwrap().range;
        let r = lc(source, range);
        assert_eq!(
            r.start.line, 3,
            "anchor line should be 3 (body starts at line 3), got: {:?}",
            range
        );
        assert_eq!(
            r.start.column, 0,
            "anchor column should be 0, got: {:?}",
            range
        );
        assert_eq!(
            r.end.column - r.start.column,
            "SELECT".len() as u32,
            "anchor span should cover exactly 'SELECT' ({} chars), got: {:?}",
            "SELECT".len(),
            range
        );
    }

    /// A generator file with a top-level `WITH ... SELECT` emits
    /// `GenerateFileBareSelectForbidden` anchored at the `WITH` keyword token.
    #[test]
    fn parse_generator_file_with_top_level_with_emits_bare_select_forbidden() {
        let source = "---\ngenerates: models\n---\nWITH cte AS (SELECT 1) SELECT * FROM cte";
        let diags = diagnostics_for(source);
        let bare_select_diag = diags
            .iter()
            .find(|d| d.code == Some(DiagnosticCode::GenerateFileBareSelectForbidden));
        assert!(
            bare_select_diag.is_some(),
            "expected GenerateFileBareSelectForbidden diagnostic for WITH body, got: {:?}",
            diags
        );
        // Anchor precision: must cover only the `WITH` keyword token (line 3,
        // column 0, length 4), not the full statement.
        let range = &bare_select_diag.unwrap().range;
        let r = lc(source, range);
        assert_eq!(
            r.start.line, 3,
            "anchor line should be 3 (body starts at line 3), got: {:?}",
            range
        );
        assert_eq!(
            r.start.column, 0,
            "anchor column should be 0, got: {:?}",
            range
        );
        assert_eq!(
            r.end.column - r.start.column,
            "WITH".len() as u32,
            "anchor span should cover exactly 'WITH' ({} chars), got: {:?}",
            "WITH".len(),
            range
        );
    }

    /// A generator file with a top-level `VALUES` emits
    /// `GenerateFileBareSelectForbidden`.
    #[test]
    fn parse_generator_file_with_top_level_values_emits_bare_select_forbidden() {
        let source = "---\ngenerates: models\n---\nVALUES (1), (2)";
        let diags = diagnostics_for(source);
        let bare_select_diag = diags
            .iter()
            .find(|d| d.code == Some(DiagnosticCode::GenerateFileBareSelectForbidden));
        assert!(
            bare_select_diag.is_some(),
            "expected GenerateFileBareSelectForbidden diagnostic for VALUES body, got: {:?}",
            diags
        );
        // Anchor precision: must cover only the `VALUES` keyword token (line 3,
        // column 0, length 6), not the full statement.
        let range = &bare_select_diag.unwrap().range;
        let r = lc(source, range);
        assert_eq!(
            r.start.line, 3,
            "anchor line should be 3 (body starts at line 3), got: {:?}",
            range
        );
        assert_eq!(
            r.start.column, 0,
            "anchor column should be 0, got: {:?}",
            range
        );
        assert_eq!(
            r.end.column - r.start.column,
            "VALUES".len() as u32,
            "anchor span should cover exactly 'VALUES' ({} chars), got: {:?}",
            "VALUES".len(),
            range
        );
    }

    /// `generates: views` produces `GeneratesUnknownValue` anchored at the
    /// value token.
    #[test]
    fn generates_unknown_value_surfaces_as_diagnostic() {
        let source = "---\ngenerates: views\n---\n[]";
        let diags = diagnostics_for(source);
        let unk = diags
            .iter()
            .find(|d| d.code == Some(DiagnosticCode::GeneratesUnknownValue));
        assert!(
            unk.is_some(),
            "expected GeneratesUnknownValue diagnostic, got: {:?}",
            diags
        );
    }

    /// `generates: models` + `name: foo` produces `GeneratesMixedWithBareModel`
    /// anchored at the `name:` key.
    #[test]
    fn generates_mixed_with_name_field_surfaces_as_diagnostic() {
        let source = "---\ngenerates: models\nname: foo\n---\n[]";
        let diags = diagnostics_for(source);
        let mixed = diags
            .iter()
            .find(|d| d.code == Some(DiagnosticCode::GeneratesMixedWithBareModel));
        assert!(
            mixed.is_some(),
            "expected GeneratesMixedWithBareModel diagnostic (NameField), got: {:?}",
            diags
        );
    }

    /// `generates: models` + `--- name: foo ---` delimiter in body produces
    /// `GeneratesMixedWithBareModel` anchored at the delimiter line.
    #[test]
    fn generates_mixed_with_section_delimiter_surfaces_as_diagnostic() {
        let source = "---\ngenerates: models\n---\n--- name: foo ---\nSELECT 1";
        let diags = diagnostics_for(source);
        let mixed = diags
            .iter()
            .find(|d| d.code == Some(DiagnosticCode::GeneratesMixedWithBareModel));
        assert!(
            mixed.is_some(),
            "expected GeneratesMixedWithBareModel diagnostic (SectionDelimiter), got: {:?}",
            diags
        );
    }

    /// Regression: a `Single` / `Multi` / `Empty` file's parse output is
    /// unaffected by the generator routing.
    #[test]
    fn non_generator_file_parses_unchanged() {
        // Single-model file
        let single = "---\nname: my_model\n---\nSELECT 1 AS x";
        let (db, file) = make_db_with_file(single);
        let parse = super::parse_file(&db, file);
        let syntax = parse.syntax();
        let has_select = syntax
            .descendants()
            .any(|n| n.kind() == SyntaxKind::SELECT_STMT);
        assert!(
            has_select,
            "Single-model file must still parse its SELECT_STMT"
        );

        // Empty file (no frontmatter)
        let empty = "SELECT 1 AS x";
        let (db2, file2) = make_db_with_file(empty);
        let parse2 = super::parse_file(&db2, file2);
        let syntax2 = parse2.syntax();
        let has_select2 = syntax2
            .descendants()
            .any(|n| n.kind() == SyntaxKind::SELECT_STMT);
        assert!(has_select2, "Plain SQL file must parse its SELECT_STMT");
    }

    /// Multi-line frontmatter offsets: line/column in diagnostics resolves to
    /// the post-frontmatter position. A bare SELECT in a generator file with
    /// three frontmatter lines should produce a diagnostic whose range starts
    /// at line 4 (0-based: line 3), not line 0.
    #[test]
    fn generator_file_body_offset_is_consumed_correctly() {
        // frontmatter occupies lines 1-3 (1-based); body starts at line 4.
        let source = "---\ngenerates: models\n---\nSELECT * FROM orders";
        let diags = diagnostics_for(source);
        let bare_select_diag = diags
            .iter()
            .find(|d| d.code == Some(DiagnosticCode::GenerateFileBareSelectForbidden));
        let diag = bare_select_diag.expect("expected GenerateFileBareSelectForbidden");

        // Line numbers are 0-based in the LSP convention used internally.
        // The body starts at line 3 (0-based) — after the three-line frontmatter.
        let r = lc(source, &diag.range);
        assert!(
            r.start.line >= 3,
            "diagnostic line {} must be >= 3 (body starts after frontmatter)",
            r.start.line
        );
    }
}

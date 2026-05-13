//! Parse + reference extraction queries (Salsa-tracked wrappers around
//! `smelt-parser`).
//!
//! These queries form the bottom layer of the smelt-db query graph: every
//! semantic query parses the file through one of these wrappers so the parse
//! result is cached once per file revision.

use std::path::PathBuf;
use std::sync::Arc;

use smelt_parser::{self, ast::SmeltPathRef, File as AstFile};

use crate::{Model, Range, RefLocation, SourceFile, SourceLocation};

// ============================================================================
// Syntax queries
// ============================================================================

#[salsa::tracked(returns(ref))]
pub fn parse_file(db: &dyn salsa::Database, file: SourceFile) -> smelt_parser::Parse {
    let text = file.text(db);
    let clean_text = smelt_parser::strip_frontmatter(text);
    smelt_parser::parse(&clean_text)
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
    ast.select_stmt()?;

    Some(Arc::new(Model {
        name: model_name,
        path,
        source_path,
    }))
}

/// Legacy `smelt.models.name` locations.
///
/// Phase 4: `smelt.ref()` is now a parse error; this query always returns an
/// empty vec. Kept so callers compile without changes during the migration.
/// Will be removed in a follow-up cleanup.
#[salsa::tracked]
pub fn model_refs(db: &dyn salsa::Database, file: SourceFile) -> Arc<Vec<RefLocation>> {
    // smelt.ref() is a parse error — no legacy RefCall nodes can appear.
    let _ = (db, file);
    Arc::new(Vec::new())
}

/// Path-form refs (`smelt.<path>` in value position) extracted with
/// their resolution metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathRefLocation {
    pub path: Vec<String>,
    pub range: Range,
    /// True when this ref appears in a `TableExpr` (FROM/JOIN) position.
    /// Phase 2a uses this to gate kind-mismatch diagnostics: a path
    /// resolving to a `Test` is invalid in `TableExpr` positions.
    pub in_table_expr_position: bool,
}

/// Extract every unified `smelt.<path>` value-form ref from a file.
///
/// Phase 2a — Salsa-tracked sister of [`model_refs`] for the new path
/// surface. The legacy `model_refs` query still surfaces
/// `smelt.models.name` callsites; this query surfaces the unified
/// path-form value refs (no `(`) so the diagnostic pass can validate
/// them through [`resolve_ref_path`]. Call-form path refs are not
/// included here — they're consumed by the function-call pipeline.
#[salsa::tracked]
pub fn model_path_refs(db: &dyn salsa::Database, file: SourceFile) -> Arc<Vec<PathRefLocation>> {
    let parse = parse_file(db, file);
    let text = file.text(db);
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
        let range = smelt_parser::ast::text_range_to_range(text, path_ref.text_range());
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
    let _ = (db, file);
    Arc::new(Vec::new())
}

//! Per-file type-checking diagnostics (top-level `check_type_diagnostics`
//! Salsa query plus the small `check_expression_types` /
//! `check_unsupported_constructs` accumulator helpers invoked by
//! `check_file_diagnostics`).

use salsa::Accumulator;
use smelt_parser::{self, File as AstFile};
use smelt_types::{parse_type, DataType};

use crate::queries::parse::{model_path_refs, model_refs, model_sources, parse_file, parse_model};
use crate::queries::schema::{
    model_input_constraints, model_schema, resolved_model_schema, type_context, typed_model_schema,
};
use crate::type_inference;
use crate::{find_project, resolve_ref, DiagnosticAcc, SourceFile, Workspace};
use crate::{Diagnostic, DiagnosticCode, DiagnosticData, DiagnosticSeverity};

// ============================================================================
// Type compatibility + expression / construct checks
// ============================================================================

/// Pure helper: walk a single `Expr` node and collect CAST/function-name
/// diagnostics into `out`. `range_offset` is added to each diagnostic's
/// `TextRange` so that body-fragment ranges map into the enclosing file's
/// byte space. Pass `TextSize::from(0)` for hand-authored callers.
pub(crate) fn collect_expression_type_diagnostics(
    expr: &smelt_parser::ast::Expr,
    range_offset: rowan::TextSize,
    out: &mut Vec<Diagnostic>,
) {
    if let Some(cast_expr) = expr.as_cast() {
        if let Some(type_spec) = cast_expr.type_spec() {
            let type_text = type_spec.full_text();
            if parse_type(&type_text).is_err() {
                // Use the enclosing cast expression's span as the diagnostic
                // range (TypeSpec does not expose a syntax() accessor; the
                // full cast expression is a precise-enough anchor).
                let range = expr.syntax().text_range() + range_offset;
                out.push(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Unknown type '{}' in CAST expression. Type inference unavailable.",
                        type_text
                    ),
                    range,
                    code: Some(DiagnosticCode::UnknownCastType),
                    data: None,
                });
            }
        }
        if let Some(inner) = cast_expr.expression() {
            collect_expression_type_diagnostics(&inner, range_offset, out);
        }
    }

    if let Some(func) = expr.as_function_call() {
        if let Some(name) = func.name() {
            let upper_name = name.to_uppercase();
            // Exempt HOF names (`map`, `filter`, `reduce`) — meta-language
            // constructs validated by `check_hof_position_diagnostics` instead.
            let is_hof = type_inference::HofKind::from_name(&name.to_lowercase()).is_some();
            if !is_hof
                && func.namespace().is_none()
                && smelt_types::SqlFunction::from_name(&upper_name).is_none()
            {
                let range = func.syntax().text_range() + range_offset;
                out.push(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Function '{}' is not a recognized SQL function. Type inference unavailable.",
                        name
                    ),
                    range,
                    code: Some(DiagnosticCode::UnrecognizedFunction),
                    data: None,
                });
            }
        }
    }
}

/// Pure helper: walk the select list of a `SelectStmt` and collect
/// CAST/function-name diagnostics. Returns the same diagnostics that the
/// Salsa-accumulator path would emit. `range_offset` is `TextSize::from(0)`
/// for hand-authored callers; emission-body callers pass the body span's
/// byte start as a `TextSize`.
pub fn check_expression_types_for_select(
    select_stmt: &smelt_parser::ast::SelectStmt,
    range_offset: rowan::TextSize,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    if let Some(select_list) = select_stmt.select_list() {
        for item in select_list.items() {
            if let Some(expr) = item.expression() {
                collect_expression_type_diagnostics(&expr, range_offset, &mut out);
            }
        }
    }
    out
}

pub(crate) fn check_expression_types(expr: &smelt_parser::ast::Expr, db: &dyn salsa::Database) {
    let mut out = Vec::new();
    collect_expression_type_diagnostics(expr, rowan::TextSize::from(0), &mut out);
    for diag in out {
        DiagnosticAcc(diag).accumulate(db);
    }
}

/// Pure helper: walk a `ModelSchema` and collect `CannotInferType`
/// diagnostics for every column whose type is `Unknown` or absent.
/// `range_offset` is `TextSize::from(0)` for hand-authored callers;
/// emission-body callers pass the body span's byte start so that column
/// ranges are remapped to the enclosing file.
pub fn cannot_infer_type_for_schema(
    typed_schema: &crate::ModelSchema,
    range_offset: rowan::TextSize,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for col in &typed_schema.columns {
        if col.name == "*" {
            continue;
        }
        match &col.data_type {
            Some(typed_col) if matches!(typed_col.data_type, DataType::Unknown) => {
                let range = col.range + range_offset;
                out.push(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Could not infer type for column '{}'. Consider adding an explicit CAST.",
                        col.name
                    ),
                    range,
                    code: Some(DiagnosticCode::CannotInferType),
                    data: Some(DiagnosticData::CannotInferType {
                        column_name: col.name.clone(),
                    }),
                });
            }
            None => {
                let range = col.range + range_offset;
                out.push(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Could not infer type for column '{}'. Consider adding an explicit CAST.",
                        col.name
                    ),
                    range,
                    code: Some(DiagnosticCode::CannotInferType),
                    data: Some(DiagnosticData::CannotInferType {
                        column_name: col.name.clone(),
                    }),
                });
            }
            _ => {}
        }
    }
    out
}

pub(crate) fn check_unsupported_constructs(
    syntax: &smelt_parser::syntax_kind::SyntaxNode,
    db: &dyn salsa::Database,
) {
    use smelt_parser::SyntaxKind::{PIVOT_CLAUSE, UNPIVOT_CLAUSE};

    for node in syntax.descendants() {
        let (kind_name, node_range) = match node.kind() {
            PIVOT_CLAUSE => ("PIVOT", node.text_range()),
            UNPIVOT_CLAUSE => ("UNPIVOT", node.text_range()),
            _ => continue,
        };
        DiagnosticAcc(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!(
                "{} is not supported \u{2014} output columns depend on data values and cannot be determined at compile time",
                kind_name
            ),
            range: node_range,
            code: Some(DiagnosticCode::UnsupportedConstruct),
            data: None,
        })
        .accumulate(db);
    }
}

pub(crate) fn types_compatible(expected: &DataType, actual: &DataType) -> bool {
    if matches!(expected, DataType::Unknown) || matches!(actual, DataType::Unknown) {
        return true;
    }
    if expected == actual {
        return true;
    }
    if expected.is_numeric() && actual.is_numeric() {
        return true;
    }
    if expected.is_string() && actual.is_string() {
        return true;
    }
    if expected.is_temporal() && actual.is_temporal() {
        return true;
    }
    if matches!(expected, DataType::Boolean) && matches!(actual, DataType::Boolean) {
        return true;
    }
    if expected.is_string() {
        return true;
    }
    false
}

#[salsa::tracked]
pub fn check_type_diagnostics(db: &dyn salsa::Database, workspace: Workspace, file: SourceFile) {
    let path = file.path(db);
    if !path
        .to_str()
        .map(|s| s.contains("models/"))
        .unwrap_or(false)
    {
        return;
    }

    if parse_model(db, file).is_none() {
        return;
    }

    let refs = model_refs(db, file);
    let sources = model_sources(db, file);
    // Phase 4: also include path-form refs that point to MODELS, SEEDS, or
    // SOURCES (smelt.models.* / smelt.seeds.* / smelt.sources.*) in the
    // "has any upstream dependency" guard. Without this, a model that uses
    // ONLY path-form refs (no legacy smelt.ref()/smelt.source()) would be
    // skipped by the early-return and never get column checks.
    //
    // Phase 4: path refs to models/seeds/sources bypass the early-return so
    // column checks run for models that use only path-form refs (no legacy
    // smelt.ref()/smelt.source()).
    //
    // Note: smelt.functions.* path refs in SELECT position (not FROM) are
    // excluded — they expand inline and don't bring FROM-scope columns.
    // However, smelt.functions.* in FROM/JOIN position (SMELT_PATH_CALL nodes
    // that appear directly in a TableRef) DO provide column scope and must
    // bypass the early-return so that UndeclaredColumn checks run against the
    // inferred return schema of the function call.
    let has_data_path_refs = model_path_refs(db, file).iter().any(|loc| {
        matches!(
            loc.path.first().map(|s| s.as_str()),
            Some("models") | Some("seeds") | Some("sources")
        )
    });
    // Check for smelt.functions.* (SMELT_PATH_CALL) nodes in FROM/JOIN position.
    let has_from_position_path_calls = {
        use smelt_parser::syntax_kind::SyntaxKind as Sk;
        let syntax = parse_file(db, file).syntax();
        syntax.descendants().any(|n| {
            n.kind() == Sk::SMELT_PATH_CALL
                && n.ancestors()
                    .any(|a| matches!(a.kind(), Sk::TABLE_REF | Sk::FROM_CLAUSE | Sk::JOIN_CLAUSE))
        })
    };
    // Files that have ONLY smelt.functions.* calls in FROM (no direct data refs)
    // skip the CannotInferType check since type inference for function return
    // schemas is intentionally limited. They still run check_undeclared_columns.
    let has_direct_data_refs = !refs.is_empty() || !sources.is_empty() || has_data_path_refs;
    if !has_direct_data_refs && !has_from_position_path_calls {
        return;
    }

    if has_direct_data_refs {
        let typed_schema = typed_model_schema(db, workspace, file);
        for diag in cannot_infer_type_for_schema(&typed_schema, rowan::TextSize::from(0)) {
            DiagnosticAcc(diag).accumulate(db);
        }
    } // end if has_direct_data_refs

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    if let Some(ast) = AstFile::cast(syntax) {
        if let Some(select_stmt) = ast.select_stmt() {
            let ctx = type_context(db, workspace, file);
            let undeclared = type_inference::check_undeclared_columns(&select_stmt, &ctx);
            for info in undeclared {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: info.message,
                    range: info.range,
                    code: Some(DiagnosticCode::UndeclaredColumn),
                    data: Some(DiagnosticData::UndeclaredColumn {
                        qualifier: info.qualifier,
                        column_name: info.column_name,
                    }),
                })
                .accumulate(db);
            }
        }
    }

    let project = find_project(db, workspace, &file.project_root(db).clone());

    let input_constraints = model_input_constraints(db, workspace, file);
    for constraint in input_constraints.iter() {
        let upstream = match project
            .and_then(|p| resolve_ref(db, workspace, p, constraint.ref_name.clone()))
        {
            Some(p) => p,
            None => continue,
        };
        let upstream_schema = typed_model_schema(db, workspace, upstream);

        for (col_name, col_constraint) in &constraint.required_columns {
            let expected = match &col_constraint.expected_type {
                Some(t) => t,
                None => continue,
            };
            let upstream_col = upstream_schema.columns.iter().find(|c| c.name == *col_name);
            if let Some(col) = upstream_col {
                if let Some(actual) = &col.data_type {
                    if !types_compatible(&expected.data_type, &actual.data_type) {
                        for site in &col_constraint.usage_sites {
                            DiagnosticAcc(Diagnostic {
                                severity: DiagnosticSeverity::Warning,
                                message: format!(
                                    "Column '{}' from '{}' has type {} but is used where {} is expected",
                                    col_name, constraint.ref_name, actual.data_type, expected.data_type
                                ),
                                range: *site,
                                code: Some(DiagnosticCode::TypeMismatch),
                                data: Some(DiagnosticData::TypeMismatch {
                                    column_name: col_name.clone(),
                                    ref_name: constraint.ref_name.clone(),
                                    actual_type: actual.data_type.to_string(),
                                    expected_type: expected.data_type.to_string(),
                                }),
                            })
                            .accumulate(db);
                        }
                    }
                }
            }
        }
    }

    // Detect cycles: base schema has content but resolved is empty OR all
    // types are Unknown → cycle recovery fired.
    //
    // Phase 4: `model_refs` always returns empty (legacy syntax is a parse
    // error). Check path-form refs (smelt.models.*) instead.
    for path_loc in model_path_refs(db, file).iter() {
        // Only model refs can form cycles.
        if path_loc.path.first().map(|s| s.as_str()) != Some("models") {
            continue;
        }
        let model_name = match path_loc.path.last() {
            Some(n) => n.clone(),
            None => continue,
        };
        if let Some(upstream) =
            project.and_then(|p| resolve_ref(db, workspace, p, model_name.clone()))
        {
            let upstream_base = model_schema(db, upstream);
            let upstream_resolved = resolved_model_schema(db, workspace, upstream);
            let all_unknown = !upstream_resolved.columns.is_empty()
                && upstream_resolved.columns.iter().all(|c| {
                    c.data_type
                        .as_ref()
                        .map(|t| matches!(t.data_type, DataType::Unknown))
                        .unwrap_or(true)
                });
            let has_any_column =
                !upstream_base.columns.is_empty() || !upstream_base.row_extensions.is_empty();
            if has_any_column && (upstream_resolved.columns.is_empty() || all_unknown) {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Circular dependency involving model '{}' — type information unavailable",
                        model_name
                    ),
                    range: path_loc.range,
                    code: Some(DiagnosticCode::CircularDependency),
                    data: None,
                })
                .accumulate(db);
            }
        }
    }
}

// ============================================================================
// Phase 3 (Phase F): Production-path (Salsa) integration tests
// ============================================================================
//
// These tests call `db.file_diagnostics()` via the `TestDb` wrapper to verify
// that the pure ternary / reducer / lambda check functions are properly wired
// into the production diagnostics pipeline.
//
// RED BEFORE WIRING, GREEN AFTER.

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{test_harness::TestDb, DiagnosticCode};
    use std::path::PathBuf;

    /// Build a single-file workspace with the given SQL body.
    fn setup(sql: &str) -> (TestDb, PathBuf) {
        let mut db = TestDb::default();
        let path = PathBuf::from("test_model.sql");
        db.set_file_text(path.clone(), Arc::new(sql.to_string()));
        db.set_all_files(Arc::new(vec![path.clone()]));
        db.set_file_project_root(path.clone(), PathBuf::from("."));
        db.set_project_sources_yaml(PathBuf::from("."), Arc::new(String::new()));
        db.set_all_project_roots(Arc::new(vec![PathBuf::from(".")]));
        (db, path)
    }

    // ── Ternary condition not boolean ─────────────────────────────────────────

    /// `SELECT if 42 then a else b FROM t` — integer condition — must produce
    /// exactly one `TernaryConditionNotBoolean` diagnostic via `file_diagnostics`.
    #[test]
    fn file_diagnostics_emits_ternary_condition_not_boolean() {
        let (mut db, path) = setup("SELECT if 42 then a else b FROM t");
        let diags = db.file_diagnostics(path);
        let matches: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::TernaryConditionNotBoolean))
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "SELECT if 42 then a else b FROM t must produce exactly 1 \
             TernaryConditionNotBoolean; got: {:?}",
            diags
        );
    }

    // ── Ternary branch type mismatch ──────────────────────────────────────────

    /// `SELECT if cond then 1 else 'x' FROM t` — incompatible branch types —
    /// must produce exactly one `TernaryBranchTypeMismatch` diagnostic.
    #[test]
    fn file_diagnostics_emits_ternary_branch_type_mismatch() {
        let (mut db, path) = setup("SELECT if cond then 1 else 'x' FROM t");
        let diags = db.file_diagnostics(path);
        let matches: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::TernaryBranchTypeMismatch))
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "SELECT if cond then 1 else 'x' FROM t must produce exactly 1 \
             TernaryBranchTypeMismatch; got: {:?}",
            diags
        );
    }

    // ── Ternary dangling then ─────────────────────────────────────────────────

    /// `SELECT then x FROM t` — bare `then` outside any `if ... then` form —
    /// must produce exactly one `TernaryDanglingThen` diagnostic.
    #[test]
    fn file_diagnostics_emits_ternary_dangling_then() {
        let (mut db, path) = setup("SELECT then x FROM t");
        let diags = db.file_diagnostics(path);
        let matches: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::TernaryDanglingThen))
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "SELECT then x FROM t must produce exactly 1 TernaryDanglingThen; \
             got: {:?}",
            diags
        );
    }

    // ── Ternary dangling else ─────────────────────────────────────────────────

    /// `SELECT if c then x FROM t` — ternary with no `else` branch —
    /// must produce exactly one `TernaryDanglingElse` diagnostic.
    #[test]
    fn file_diagnostics_emits_ternary_dangling_else() {
        let (mut db, path) = setup("SELECT if c then x FROM t");
        let diags = db.file_diagnostics(path);
        let matches: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::TernaryDanglingElse))
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "SELECT if c then x FROM t must produce exactly 1 TernaryDanglingElse; \
             got: {:?}",
            diags
        );
    }

    // ── Parameterised reducer arity mismatch ──────────────────────────────────

    /// `SELECT reduce(xs, concat_with()) FROM t` — `concat_with` missing its
    /// required `sep` argument — must produce exactly one `ReducerArityMismatch`
    /// diagnostic via `file_diagnostics`.
    #[test]
    fn file_diagnostics_emits_reducer_call_diagnostics() {
        let (mut db, path) = setup("SELECT reduce(xs, concat_with()) FROM t");
        let diags = db.file_diagnostics(path);
        let matches: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::ReducerArityMismatch))
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "SELECT reduce(xs, concat_with()) FROM t must produce exactly 1 \
             ReducerArityMismatch; got: {:?}",
            diags
        );
    }

    // ── Lambda arity mismatch (multi-arg in HOF position) ─────────────────────

    /// `SELECT map(xs, fn (a, b) => a + b) FROM t` — 2-arg lambda where map
    /// expects arity 1 — must produce exactly one `LambdaArityMismatch`.
    #[test]
    fn file_diagnostics_emits_lambda_arity_mismatch() {
        let (mut db, path) = setup("SELECT map(xs, fn (a, b) => a + b) FROM t");
        let diags = db.file_diagnostics(path);
        let matches: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::LambdaArityMismatch))
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "SELECT map(xs, fn (a, b) => a + b) FROM t must produce exactly 1 \
             LambdaArityMismatch; got: {:?}",
            diags
        );
    }

    // ── Lambda duplicate parameter ─────────────────────────────────────────────

    /// `SELECT map(xs, fn (a, a) => a) FROM t` — duplicate parameter `a` —
    /// must produce exactly one `LambdaDuplicateParameter` diagnostic.
    #[test]
    fn file_diagnostics_emits_lambda_duplicate_parameter() {
        let (mut db, path) = setup("SELECT map(xs, fn (a, a) => a) FROM t");
        let diags = db.file_diagnostics(path);
        let matches: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::LambdaDuplicateParameter))
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "SELECT map(xs, fn (a, a) => a) FROM t must produce exactly 1 \
             LambdaDuplicateParameter; got: {:?}",
            diags
        );
    }

    // ── TernaryKeywordShadowed (smelt.define named `if`) ──────────────────────

    /// `smelt.define if(x: Boolean) -> Boolean = x` — declares a function named
    /// `if`, which is a reserved ternary keyword — must produce exactly one
    /// `TernaryKeywordShadowed` diagnostic via `file_diagnostics`.
    #[test]
    fn file_diagnostics_emits_ternary_keyword_shadowed() {
        let (mut db, path) = setup("smelt.define if(x: Boolean) -> Boolean = x");
        let diags = db.file_diagnostics(path);
        let matches: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::TernaryKeywordShadowed))
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "smelt.define if(...) must produce exactly 1 TernaryKeywordShadowed; \
             got: {:?}",
            diags
        );
    }

    // ── _for_select pure helper: UnrecognizedFunction ─────────────────────────

    /// `check_expression_types_for_select` must emit the same
    /// `UnrecognizedFunction` diagnostic as the Salsa-accumulator path for
    /// `SELECT foo_unknown(1) FROM t`.
    #[test]
    fn check_expression_types_for_select_emits_unrecognized_function() {
        use crate::queries::check_types::check_expression_types_for_select;
        use smelt_parser::{self, File as AstFile};

        let sql = "SELECT foo_unknown(1) FROM t";
        let parse = smelt_parser::parse(sql);
        let syntax = parse.syntax();
        let ast = AstFile::cast(syntax).expect("should parse as AstFile");
        let select = ast.select_stmt().expect("should have select_stmt");

        let diags = check_expression_types_for_select(&select, rowan::TextSize::from(0));
        let matches: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::UnrecognizedFunction))
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "SELECT foo_unknown(1) FROM t must produce exactly 1 UnrecognizedFunction; \
             got: {:?}",
            diags
        );

        // Also verify the _for_file path produces the same diagnostic.
        let (mut db, path) = setup(sql);
        let file_diags = db.file_diagnostics(path);
        let file_matches: Vec<_> = file_diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::UnrecognizedFunction))
            .collect();
        assert_eq!(
            file_matches.len(),
            matches.len(),
            "file_diagnostics and check_expression_types_for_select must agree on \
             UnrecognizedFunction count; file: {:?}, pure: {:?}",
            file_diags,
            diags
        );
    }

    /// `check_expression_types_for_select` must emit `UnknownCastType` for
    /// `SELECT CAST(x AS BogusType) FROM t`.
    #[test]
    fn check_expression_types_for_select_emits_unknown_cast_type() {
        use crate::queries::check_types::check_expression_types_for_select;
        use smelt_parser::{self, File as AstFile};

        let sql = "SELECT CAST(x AS BogusType) FROM t";
        let parse = smelt_parser::parse(sql);
        let syntax = parse.syntax();
        let ast = AstFile::cast(syntax).expect("should parse as AstFile");
        let select = ast.select_stmt().expect("should have select_stmt");

        let diags = check_expression_types_for_select(&select, rowan::TextSize::from(0));
        let matches: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::UnknownCastType))
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "SELECT CAST(x AS BogusType) FROM t must produce exactly 1 UnknownCastType; \
             got: {:?}",
            diags
        );
    }

    // ── _for_select pure helper: range_offset is plumbed ─────────────────────

    /// When `range_offset = 0`, `check_expression_types_for_select` emits the
    /// diagnostic at the real AST node position (not the old zero-zero default).
    /// For single-line SQL the diagnostic has a non-zero start byte offset.
    #[test]
    fn check_expression_types_for_select_range_offset_zero_noop() {
        use crate::queries::check_types::check_expression_types_for_select;
        use smelt_parser::{self, File as AstFile};

        let sql = "SELECT CAST(x AS BogusType) FROM t";
        let parse = smelt_parser::parse(sql);
        let syntax = parse.syntax();
        let ast = AstFile::cast(syntax).expect("should parse as AstFile");
        let select = ast.select_stmt().expect("should have select_stmt");

        let diags_0 = check_expression_types_for_select(&select, rowan::TextSize::from(0));
        // The CAST expression starts after "SELECT " (7 bytes); with offset 0
        // the range start should be at byte 7.
        assert!(
            !diags_0.is_empty(),
            "must produce at least one diagnostic; got: {:?}",
            diags_0
        );
        let first = &diags_0[0];
        // "SELECT " is 7 bytes; the CAST expression starts at byte offset 7.
        assert!(
            u32::from(first.range.start()) > 0,
            "CAST expression is not at byte 0 in 'SELECT CAST(...)'; start={:?}",
            first.range.start()
        );
    }

    /// When `range_offset > 0`, `check_expression_types_for_select` shifts each
    /// diagnostic range so that byte offsets map into the enclosing file rather
    /// than the fragment. Passing `range_offset = 100` bytes shifts each
    /// reported start forward by 100.
    #[test]
    fn check_expression_types_for_select_range_offset_shifts_position() {
        use crate::queries::check_types::check_expression_types_for_select;
        use smelt_parser::{self, File as AstFile};

        // Fragment SQL — will be treated as starting at byte 100 in the
        // enclosing file.
        let fragment_sql = "SELECT CAST(x AS BogusType) FROM t";
        let parse = smelt_parser::parse(fragment_sql);
        let syntax = parse.syntax();
        let ast = AstFile::cast(syntax).expect("should parse as AstFile");
        let select = ast.select_stmt().expect("should have select_stmt");

        let diags_no_offset = check_expression_types_for_select(&select, rowan::TextSize::from(0));
        let diags_with_offset =
            check_expression_types_for_select(&select, rowan::TextSize::from(100));

        assert!(
            !diags_no_offset.is_empty(),
            "must produce diagnostics without offset; got: {:?}",
            diags_no_offset
        );
        assert_eq!(
            diags_no_offset.len(),
            diags_with_offset.len(),
            "offset must not change the number of diagnostics"
        );

        let no_off = &diags_no_offset[0];
        let with_off = &diags_with_offset[0];

        // With 100 bytes of offset, start byte advances by 100.
        assert_eq!(
            u32::from(with_off.range.start()),
            u32::from(no_off.range.start()) + 100,
            "range_offset = 100 must add 100 to start byte; \
             no_offset: {:?}, with_offset: {:?}",
            no_off.range,
            with_off.range
        );
    }

    // ── cannot_infer_type_for_schema ──────────────────────────────────────────

    /// `cannot_infer_type_for_schema` must emit `CannotInferType` for a column
    /// with `data_type: None`.
    #[test]
    fn cannot_infer_type_for_schema_emits_for_unknown_column() {
        use crate::queries::check_types::cannot_infer_type_for_schema;
        use crate::{Column, ColumnSource, ModelSchema};

        let col_range = rowan::TextRange::new(7.into(), 15.into()); // "some_col"

        let col_no_type = Column {
            name: "some_col".to_string(),
            alias: None,
            source: ColumnSource::Computed,
            expression: "some_col".to_string(),
            range: col_range,
            data_type: None, // None → CannotInferType
        };

        let schema = ModelSchema {
            columns: vec![col_no_type],
            row_extensions: vec![],
            input_constraints: vec![],
        };

        let diags = cannot_infer_type_for_schema(&schema, rowan::TextSize::from(0));
        let matches: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::CannotInferType))
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "schema with one untyped column must produce 1 CannotInferType; \
             got: {:?}",
            diags
        );
    }

    /// `cannot_infer_type_for_schema` must emit `CannotInferType` for a column
    /// whose data_type is `Unknown`.
    #[test]
    fn cannot_infer_type_for_schema_emits_for_unknown_data_type() {
        use crate::queries::check_types::cannot_infer_type_for_schema;
        use crate::{Column, ColumnSource, ModelSchema};
        use smelt_types::{DataType, TypedColumn};

        let col_range = rowan::TextRange::new(7.into(), 15.into());

        let col_unknown = Column {
            name: "some_col".to_string(),
            alias: None,
            source: ColumnSource::Computed,
            expression: "some_col".to_string(),
            range: col_range,
            data_type: Some(TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            }),
        };

        let schema = ModelSchema {
            columns: vec![col_unknown],
            row_extensions: vec![],
            input_constraints: vec![],
        };

        let diags = cannot_infer_type_for_schema(&schema, rowan::TextSize::from(0));
        let matches: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::CannotInferType))
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "schema with Unknown data_type column must produce 1 CannotInferType; \
             got: {:?}",
            diags
        );
    }

    /// `cannot_infer_type_for_schema` must produce zero diagnostics for a
    /// column with a concrete known type.
    #[test]
    fn cannot_infer_type_for_schema_no_diags_for_typed_column() {
        use crate::queries::check_types::cannot_infer_type_for_schema;
        use crate::{Column, ColumnSource, ModelSchema};
        use smelt_types::{DataType, TypedColumn};

        let col_range = rowan::TextRange::new(7.into(), 15.into());

        let col_typed = Column {
            name: "some_col".to_string(),
            alias: None,
            source: ColumnSource::Computed,
            expression: "some_col".to_string(),
            range: col_range,
            data_type: Some(TypedColumn {
                data_type: DataType::Integer,
                nullable: false,
            }),
        };

        let schema = ModelSchema {
            columns: vec![col_typed],
            row_extensions: vec![],
            input_constraints: vec![],
        };

        let diags = cannot_infer_type_for_schema(&schema, rowan::TextSize::from(0));
        assert!(
            diags.is_empty(),
            "schema with typed column must produce 0 diagnostics; got: {:?}",
            diags
        );
    }
}

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
use crate::{resolve_ref, DiagnosticAcc, Position, Range, SourceFile, Workspace};
use crate::{Diagnostic, DiagnosticCode, DiagnosticData, DiagnosticSeverity};

// ============================================================================
// Type compatibility + expression / construct checks
// ============================================================================

pub(crate) fn check_expression_types(expr: &smelt_parser::ast::Expr, db: &dyn salsa::Database) {
    let default_range = Range {
        start: Position { line: 0, column: 0 },
        end: Position { line: 0, column: 0 },
    };

    if let Some(cast_expr) = expr.as_cast() {
        if let Some(type_spec) = cast_expr.type_spec() {
            let type_text = type_spec.full_text();
            if parse_type(&type_text).is_err() {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Unknown type '{}' in CAST expression. Type inference unavailable.",
                        type_text
                    ),
                    range: default_range,
                    code: Some(DiagnosticCode::UnknownCastType),
                    data: None,
                })
                .accumulate(db);
            }
        }
        if let Some(inner) = cast_expr.expression() {
            check_expression_types(&inner, db);
        }
    }

    if let Some(func) = expr.as_function_call() {
        if let Some(name) = func.name() {
            let upper_name = name.to_uppercase();
            // Phase B (meta-language): exempt HOF names (`map`, `filter`,
            // `reduce`) from the UnrecognizedFunction warning — they are
            // meta-language constructs, not SQL built-ins, and are validated
            // by `check_hof_position_diagnostics` instead.
            let is_hof = type_inference::HofKind::from_name(&name.to_lowercase()).is_some();
            if !is_hof
                && func.namespace().is_none()
                && smelt_types::SqlFunction::from_name(&upper_name).is_none()
            {
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Function '{}' is not a recognized SQL function. Type inference unavailable.",
                        name
                    ),
                    range: default_range,
                    code: Some(DiagnosticCode::UnrecognizedFunction),
                    data: None,
                })
                .accumulate(db);
            }
        }
    }
}

pub(crate) fn check_unsupported_constructs(
    syntax: &smelt_parser::syntax_kind::SyntaxNode,
    text: &str,
    db: &dyn salsa::Database,
) {
    use smelt_parser::SyntaxKind::{PIVOT_CLAUSE, UNPIVOT_CLAUSE};

    for node in syntax.descendants() {
        let (kind_name, node_range) = match node.kind() {
            PIVOT_CLAUSE => ("PIVOT", node.text_range()),
            UNPIVOT_CLAUSE => ("UNPIVOT", node.text_range()),
            _ => continue,
        };
        let range = smelt_parser::ast::text_range_to_range(text, node_range);
        DiagnosticAcc(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!(
                "{} is not supported \u{2014} output columns depend on data values and cannot be determined at compile time",
                kind_name
            ),
            range,
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

    let text = file.text(db);

    if has_direct_data_refs {
        let typed_schema = typed_model_schema(db, workspace, file);

        for col in &typed_schema.columns {
            if col.name == "*" {
                continue;
            }

            match &col.data_type {
                Some(typed_col) if matches!(typed_col.data_type, DataType::Unknown) => {
                    let range = smelt_parser::ast::text_range_to_range(text, col.range);
                    DiagnosticAcc(Diagnostic {
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
                    })
                    .accumulate(db);
                }
                None => {
                    let range = smelt_parser::ast::text_range_to_range(text, col.range);
                    DiagnosticAcc(Diagnostic {
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
                    })
                    .accumulate(db);
                }
                _ => {}
            }
        }
    } // end if has_direct_data_refs

    let parse = parse_file(db, file);
    let syntax = parse.syntax();
    if let Some(ast) = AstFile::cast(syntax) {
        if let Some(select_stmt) = ast.select_stmt() {
            let ctx = type_context(db, workspace, file);
            let undeclared = type_inference::check_undeclared_columns(&select_stmt, &ctx);
            for info in undeclared {
                let range = smelt_parser::ast::text_range_to_range(text, info.range);
                DiagnosticAcc(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: info.message,
                    range,
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

    let input_constraints = model_input_constraints(db, workspace, file);
    for constraint in input_constraints.iter() {
        let upstream = match resolve_ref(db, workspace, constraint.ref_name.clone()) {
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
                            let range = smelt_parser::ast::text_range_to_range(text, *site);
                            DiagnosticAcc(Diagnostic {
                                severity: DiagnosticSeverity::Warning,
                                message: format!(
                                    "Column '{}' from '{}' has type {} but is used where {} is expected",
                                    col_name, constraint.ref_name, actual.data_type, expected.data_type
                                ),
                                range,
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
        if let Some(upstream) = resolve_ref(db, workspace, model_name.clone()) {
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
}

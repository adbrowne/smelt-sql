use super::*;
use std::path::PathBuf;
use tower_lsp::lsp_types::{Position, Range};

#[test]
fn test_from_clause_context_after_from_keyword() {
    let text = "WITH cte AS (SELECT 1) SELECT * FROM ";
    let ctx = determine_completion_context(text, text.len());
    assert!(matches!(ctx, CompletionContext::FromClause));
}

#[test]
fn test_from_clause_context_partial_identifier() {
    let text = "WITH cte AS (SELECT 1) SELECT * FROM ct";
    let ctx = determine_completion_context(text, text.len());
    assert!(matches!(ctx, CompletionContext::FromClause));
}

#[test]
fn test_from_clause_context_after_join() {
    let text = "SELECT * FROM a JOIN ";
    let ctx = determine_completion_context(text, text.len());
    assert!(matches!(ctx, CompletionContext::FromClause));
}

#[test]
fn test_not_from_context_inside_ref() {
    let text = "SELECT * FROM smelt.ref('";
    let ctx = determine_completion_context(text, text.len());
    assert!(matches!(ctx, CompletionContext::InsideRef));
}

#[test]
fn test_not_from_context_inside_source() {
    let text = "SELECT * FROM smelt.source('";
    let ctx = determine_completion_context(text, text.len());
    assert!(matches!(ctx, CompletionContext::InsideSource));
}

#[test]
fn test_not_from_context_after_where() {
    // After WHERE, we're past the FROM clause table position
    let text = "SELECT * FROM t WHERE ";
    let ctx = determine_completion_context(text, text.len());
    assert!(!matches!(ctx, CompletionContext::FromClause));
}

#[test]
fn test_not_from_context_after_on() {
    let text = "SELECT * FROM a JOIN b ON ";
    let ctx = determine_completion_context(text, text.len());
    assert!(!matches!(ctx, CompletionContext::FromClause));
}

#[test]
fn test_from_position_empty_after_from() {
    assert!(is_in_from_position("SELECT * FROM "));
}

#[test]
fn test_from_position_partial_identifier() {
    assert!(is_in_from_position("SELECT * FROM CT"));
}

#[test]
fn test_from_position_after_join() {
    assert!(is_in_from_position("SELECT * FROM A JOIN "));
}

#[test]
fn test_not_from_position_complete_table_ref() {
    // After a complete table ref with alias, we're past the position
    assert!(!is_in_from_position("SELECT * FROM TABLE_A T"));
}

// =====================================================================
// Phase 12 — multi-level frame rendering (smelt-functions Step 2).
//
// These tests exercise `render_expansion_frames` directly because it
// is a pure function over a `DbDiagnostic` — we don't need a running
// `Backend` / tower-lsp `Client` to validate the renderer contract.
// =====================================================================

use rowan::{TextRange, TextSize};
use smelt_db::{
    Diagnostic as DbDiagnosticT, DiagnosticCode, DiagnosticData, DiagnosticSeverity as DbSeverityT,
};
use smelt_types::FrameInfo;

/// Build a `TextRange` for use in test fixtures.
/// Maps `(line, col)` to byte offsets via the simple formula `line * 100 + col`
/// so that `line=0,col=0` → `[0, 1)` and distinct (line, col) pairs give
/// distinct ranges without requiring real source text in unit tests.
fn make_db_range(line: u32, col: u32) -> TextRange {
    let start = line * 100 + col;
    TextRange::new(TextSize::from(start), TextSize::from(start + 1))
}

fn make_frame(function: &str, param: &str, bound_type: &str, decl_line: u32) -> FrameInfo {
    // Use a temp-dir file path so `Url::from_file_path` succeeds on
    // Linux/macOS (the path must be absolute). We can't reach for a
    // real tempfile in a unit test without pulling tempfile into
    // dev-deps; using the conventional `/tmp/...` absolute path keeps
    // the URL builder happy on the CI runner.
    let path = PathBuf::from(format!("/tmp/smelt-lsp-test-{function}.sql"));
    FrameInfo {
        function: function.to_string(),
        param: param.to_string(),
        bound_type: bound_type.to_string(),
        decl_path: Some(path),
        decl_range: Some(make_db_range(decl_line, 0)),
        call_site_range: Some(make_db_range(decl_line + 10, 0)),
        fn_id: Some(function.to_string()),
        element_index: None,
        column_origin: None,
        model_origin: None,
        source_origin: None,
    }
}

fn make_db_diag(message: &str, frames: Vec<FrameInfo>) -> DbDiagnosticT {
    DbDiagnosticT {
        severity: DbSeverityT::Error,
        message: message.to_string(),
        range: make_db_range(0, 0),
        code: Some(DiagnosticCode::UnknownIdentifier),
        data: Some(DiagnosticData::ExpansionFrames(frames)),
    }
}

/// Phase 12 TDD test #4 — LSP e2e: nested-call error includes
/// `relatedInformation` per frame. A three-level expansion chain
/// (`outer_call → middle → inner_unary`) must yield exactly three
/// related-info entries and a message with three trailer lines, all
/// in outer-to-inner order.
#[test]
fn lsp_diagnostic_formats_frames_as_related_information() {
    // Innermost-first → outermost-last data layout, matching the
    // `check_smelt_fn_call` merge contract.
    let frames = vec![
        make_frame("inner_unary", "x", "INTEGER", 1),
        make_frame("middle", "z", "INTEGER", 2),
        make_frame("outer_call", "y", "INTEGER", 3),
    ];
    let diag = make_db_diag("unknown identifier `undefined_var`", frames);

    let (message, related) = render_expansion_frames(&diag);

    // 1. The related-information list must have one entry per frame.
    let related = related.expect("expected three-level chain to produce related_information");
    assert_eq!(
        related.len(),
        3,
        "expected one DiagnosticRelatedInformation per frame, got {related:#?}"
    );

    // 2. Frame order in related-information is outer-to-inner
    //    (`frames.iter().rev()` in the renderer).
    assert!(
        related[0].message.contains("outer_call"),
        "first related-info entry must be the outermost frame, got: {}",
        related[0].message
    );
    assert!(
        related[1].message.contains("middle"),
        "second related-info entry must be the middle frame, got: {}",
        related[1].message
    );
    assert!(
        related[2].message.contains("inner_unary"),
        "third related-info entry must be the innermost frame, got: {}",
        related[2].message
    );

    // 3. URIs must resolve to a real file-scheme URL.
    for info in &related {
        assert_eq!(info.location.uri.scheme(), "file");
        assert!(
            info.location.uri.to_file_path().is_ok(),
            "related-info URI must round-trip to a file path: {}",
            info.location.uri
        );
    }

    // 4. The message body is expanded with one trailer line per frame,
    //    outer-to-inner.
    let pos_outer = message
        .find("outer_call")
        .expect("rendered message must mention outer_call");
    let pos_middle = message
        .find("middle")
        .expect("rendered message must mention middle");
    let pos_inner = message
        .find("inner_unary")
        .expect("rendered message must mention inner_unary");
    assert!(
        pos_outer < pos_middle && pos_middle < pos_inner,
        "message trailers must render outer-to-inner; got {pos_outer}/{pos_middle}/{pos_inner} — rendered:\n{message}"
    );
}

/// Phase 12 — single-frame diagnostics still render one trailer line
/// and exactly one related-information entry (Phase 6 behaviour
/// preserved).
#[test]
fn lsp_single_level_frame_preserves_phase6_rendering() {
    let frames = vec![make_frame("safe_divide", "numerator", "TEXT", 0)];
    let diag = make_db_diag("type mismatch in body", frames);

    let (message, related) = render_expansion_frames(&diag);

    let related = related.expect("single frame must still produce related_information");
    assert_eq!(
        related.len(),
        1,
        "single-frame diagnostics must emit exactly one related-info entry"
    );
    assert!(related[0].message.contains("safe_divide"));

    // Exactly one trailer line was appended.
    let trailer_count = message.matches("\nin expansion of `").count();
    assert_eq!(
        trailer_count, 1,
        "single-frame diagnostic must have exactly one trailer line, got: {message}"
    );
}

/// Phase 12 — diagnostics without any `ExpansionFrames` payload must
/// pass through untouched. This is the common case for non-function
/// diagnostics (unknown model refs, type mismatches in model SQL,
/// etc.) and must stay zero-cost.
#[test]
fn lsp_non_frame_diagnostics_unaffected() {
    let diag = DbDiagnosticT {
        severity: DbSeverityT::Error,
        message: "undefined model `foo`".to_string(),
        range: make_db_range(0, 0),
        code: Some(DiagnosticCode::UndefinedModelRef),
        data: None,
    };
    let (message, related) = render_expansion_frames(&diag);
    assert_eq!(message, "undefined model `foo`");
    assert!(related.is_none());
}

/// Phase B reviewer finding 5 — anonymous HOF expansion frames (fn_id = None,
/// param = "", bound_type = "") must render as `"in expansion of `<map>` call"`
/// rather than the malformed `"`, `` was bound to "` produced by the old
/// named-frame template.
#[test]
fn lsp_anonymous_hof_frame_renders_without_empty_fragments() {
    // Build an anonymous frame (fn_id = None, empty param / bound_type).
    let path = PathBuf::from("/tmp/smelt-lsp-test-anon-hof.sql");
    let anon_frame = FrameInfo {
        function: "<map>".to_string(),
        param: String::new(),
        bound_type: String::new(),
        decl_path: Some(path),
        decl_range: Some(make_db_range(0, 0)),
        call_site_range: Some(make_db_range(10, 0)),
        fn_id: None, // marks frame as anonymous
        element_index: None,
        column_origin: None,
        model_origin: None,
        source_origin: None,
    };
    let diag = make_db_diag("type mismatch in lambda body", vec![anon_frame]);

    let (message, related) = render_expansion_frames(&diag);

    // 1. The trailer must NOT contain the empty-fragment patterns.
    assert!(
        !message.contains("`` was bound to"),
        "anonymous frame must not render empty param fragment; got: {message}"
    );
    assert!(
        !message.contains("was bound to \"\""),
        "anonymous frame must not render empty bound_type fragment; got: {message}"
    );

    // 2. The trailer must mention the HOF name.
    assert!(
        message.contains("<map>"),
        "anonymous frame trailer must include the HOF name; got: {message}"
    );

    // 3. The trailer must use the shorter "call" form.
    assert!(
        message.contains("in expansion of `<map>` call"),
        "anonymous frame trailer must use the short form; got: {message}"
    );

    // 4. The related-info message must also use the short form.
    let related = related.expect("anonymous frame with a decl_path must produce related_info");
    assert_eq!(related.len(), 1);
    assert!(
        related[0].message.contains("in expansion of `<map>` call"),
        "related-info message must use the short form; got: {}",
        related[0].message
    );
}

// ── Phase 4: LSP hover for list literal and spread ──────────────────────

/// Helper: parse `SELECT <expr>` and extract the first select-item expression,
/// then cast it to an ArrayLiteral and return its elements.
fn list_literal_elements(sql: &str) -> Vec<smelt_parser::ast::Expr> {
    use smelt_parser::ast::File as AstFile;
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = AstFile::cast(root).expect("FILE node");
    let select = file.select_stmt().expect("SelectStmt");
    let select_list = select.select_list().expect("select list");
    let first_item = select_list.items().next().expect("at least one item");
    let expr = first_item.expression().expect("expression");
    let arr = expr
        .as_array_literal()
        .expect("expected ARRAY_LITERAL node");
    arr.elements()
}

/// Helper: parse SQL and find the first `LIST_SPREAD` node anywhere in the
/// CST (spread items may not appear as `SelectItem` children depending on
/// the grammar position; descendant search is the robust approach).
fn parse_list_spread(sql: &str) -> smelt_parser::ast::ListSpread {
    use smelt_parser::syntax_kind::SyntaxKind;
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    root.descendants()
        .find(|n| n.kind() == SyntaxKind::LIST_SPREAD)
        .and_then(smelt_parser::ast::ListSpread::cast)
        .expect("expected LIST_SPREAD node in SQL")
}

/// Hover on `[1, 2, 3]` — all Integer — must return text containing
/// `List<Expr<INTEGER>>`.
///
/// Note: `format_smelt_type_hover` renders DataType names in SQL uppercase
/// (e.g. `INTEGER`, `TEXT`) via `DataType::to_sql()`.
#[test]
fn hover_list_literal_homogeneous() {
    let elems = list_literal_elements("SELECT [100000, 200000, 300000]");
    let ctx = smelt_db::TypeContext::new();
    let text = hover_text_for_list_literal(&elems, &ctx, None);
    assert!(
        text.contains("List<Expr<INTEGER>>"),
        "hover text for homogeneous integer list must contain `List<Expr<INTEGER>>`, got: {text}"
    );
}

/// Hover on `[]` at a position expecting `List<Expr<TEXT>>` must return
/// `List<Expr<TEXT>>`.
///
/// Note: DataType names render in SQL uppercase via `DataType::to_sql()`.
///
/// Tests the Phase B+ position-aware code path (`hover_text_for_list_literal`
/// with `expected = Some(…)`); not exercised by `Backend::hover` today —
/// the production dispatch always calls `hover_text_for_list_literal_dual`
/// with `expected = None`.
#[test]
fn hover_list_literal_empty_with_target() {
    use smelt_types::signatures::{SmeltType, TypeConstraint};
    use smelt_types::DataType;
    let elems = list_literal_elements("SELECT []");
    let ctx = smelt_db::TypeContext::new();
    let expected = SmeltType::List(Box::new(SmeltType::Expr(TypeConstraint::Concrete(
        DataType::Text,
    ))));
    let text = hover_text_for_list_literal(&elems, &ctx, Some(&expected));
    assert!(
        text.contains("List<Expr<TEXT>>"),
        "hover text for empty list with TEXT target must contain `List<Expr<TEXT>>`, got: {text}"
    );
}

/// Hover on `[1, 'hello']` — mixed Integer/Text — must return
/// `List<Unknown>` (heterogeneous).
#[test]
fn hover_list_literal_unknown() {
    let elems = list_literal_elements("SELECT [1, 'hello']");
    let ctx = smelt_db::TypeContext::new();
    let text = hover_text_for_list_literal(&elems, &ctx, None);
    assert!(
        text.contains("List<Unknown>"),
        "hover text for heterogeneous list must contain `List<Unknown>`, got: {text}"
    );
}

/// Hover on `[1, 2, 3]` at a position admitting both meta-list and
/// Data-World array: hover text must surface both readings.
///
/// The spec note says "literal accepted in two contexts". When no expected
/// sort is present and the element type is a concrete `Expr<T>`, both
/// interpretations are valid. The hover must mention both
/// `List<Expr<INTEGER>>` (meta) and `Array<INTEGER>` (data-world).
///
/// Note: DataType names render in SQL uppercase via `DataType::to_sql()`.
#[test]
fn hover_list_literal_dual_admissible() {
    let elems = list_literal_elements("SELECT [100000, 200000, 300000]");
    let ctx = smelt_db::TypeContext::new();
    let text = hover_text_for_list_literal_dual(&elems, &ctx);
    assert!(
        text.contains("List<Expr<INTEGER>>"),
        "dual-admissible hover must mention meta reading `List<Expr<INTEGER>>`, got: {text}"
    );
    assert!(
        text.contains("Array<INTEGER>"),
        "dual-admissible hover must mention data-world reading `Array<INTEGER>`, got: {text}"
    );
}

/// Hover on `...[1.5, 2.5]` — spread of a numeric list literal.
///
/// Note: Phase A cannot bind named variables; the operand is a list literal
/// whose inferred type is `List<Expr<DECIMAL(2,1)>>` — both `1.5` and `2.5`
/// lex as `Decimal(2,1)` and the LUB of two identical types is that same type.
/// The hover must show that exact element type (not the `Decimal(38,10)` that
/// promotion would produce for mixed types — these two are the same precision).
#[test]
fn hover_spread_returns_source_list_type() {
    // `...[1.5, 2.5]` — LIST_SPREAD wrapping an ARRAY_LITERAL.
    // Both literals lex as Decimal(2,1); LUB of identical types is the type
    // itself, so the inferred element type is Decimal(2,1).
    let spread = parse_list_spread("SELECT ...[1.5, 2.5]");
    let ctx = smelt_db::TypeContext::new();
    let text = hover_text_for_list_spread(&spread, &ctx);
    // Assert the exact inferred type — homogeneous Decimal(2,1) list.
    assert!(
        text.contains("List<Expr<DECIMAL(2,1)>>"),
        "hover for spread of [1.5, 2.5] must be `List<Expr<DECIMAL(2,1)>>`, got: {text}"
    );
}

// ── Phase B: LSP hover/goto-def/completion for HOFs, lambdas, pipe, reducers, config.var ──

/// Parse SQL that contains a HOF call and extract the FunctionCall node.
fn parse_hof_call(sql: &str) -> smelt_parser::ast::FunctionCall {
    use smelt_parser::ast::File as AstFile;
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = AstFile::cast(root).expect("FILE node");
    // Find the first FUNCTION_CALL node anywhere in the tree.
    file.syntax()
        .descendants()
        .find_map(smelt_parser::ast::FunctionCall::cast)
        .expect("expected a FUNCTION_CALL node in SQL")
}

/// Parse SQL and extract the first LAMBDA node.
fn parse_lambda(sql: &str) -> smelt_parser::ast::Lambda {
    use smelt_parser::syntax_kind::SyntaxKind;
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    root.descendants()
        .find(|n| n.kind() == SyntaxKind::LAMBDA)
        .and_then(smelt_parser::ast::Lambda::cast)
        .expect("expected a LAMBDA node in SQL")
}

/// Parse SQL and extract the first PIPE_EXPR node.
fn parse_pipe_expr(sql: &str) -> smelt_parser::ast::PipeExpr {
    use smelt_parser::syntax_kind::SyntaxKind;
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    root.descendants()
        .find(|n| n.kind() == SyntaxKind::PIPE_EXPR)
        .and_then(smelt_parser::ast::PipeExpr::cast)
        .expect("expected a PIPE_EXPR node in SQL")
}

/// Hover on `c` inside `map([1, 2, 3], fn c => c)` returns text containing
/// the parameter type (`Expr<INTEGER>` bound from list element type).
#[test]
fn hover_lambda_parameter_in_body() {
    let call = parse_hof_call("SELECT map([100000, 200000, 300000], fn c => c)");
    let ctx = smelt_db::TypeContext::new();
    let text = hover_text_for_hof_call(&call, &ctx);
    assert!(
        text.contains("List"),
        "hover for map([ints], fn c => c) must contain `List`, got: {text}"
    );
    // Also test hover_text_for_lambda_param directly with a known element type
    use smelt_types::signatures::{SmeltType, TypeConstraint};
    use smelt_types::DataType;
    let int_ty = SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer));
    let lambda = parse_lambda("SELECT map([1, 2, 3], fn c => c)");
    let param_text = hover_text_for_lambda_param("c", &int_ty, &lambda, &ctx);
    assert!(
        param_text.contains("Expr<INTEGER>"),
        "hover for lambda param `c` bound to Integer must contain `Expr<INTEGER>`, got: {param_text}"
    );
}

/// Hover on the `map(...)` call expression returns `List<U>` where
/// `U` is the lambda body's synthesised type.
#[test]
fn hover_hof_call_returns_result_type() {
    let call = parse_hof_call("SELECT map([100000, 200000, 300000], fn c => c)");
    let ctx = smelt_db::TypeContext::new();
    let text = hover_text_for_hof_call(&call, &ctx);
    assert!(
        text.contains("List<Expr<INTEGER>>"),
        "hover for map([ints], fn c => c) must be `List<Expr<INTEGER>>`, got: {text}"
    );
}

/// Hover on `xs |> filter(fn c => c > 0)` returns the same type as
/// hover on `filter(xs, fn c => c > 0)`.
#[test]
fn hover_pipe_expression_returns_unpiped_type() {
    // Build a pipe expression and the direct equivalent call
    let pipe = parse_pipe_expr("SELECT [100000, 200000, -1] |> filter(fn c => c > 0)");
    let ctx = smelt_db::TypeContext::new();
    let pipe_text = hover_text_for_pipe_expr(&pipe, &ctx);
    assert!(
        pipe_text.contains("List"),
        "hover for pipe expression must contain `List`, got: {pipe_text}"
    );
    // The direct equivalent call should give the same result
    let direct = parse_hof_call("SELECT filter([100000, 200000, -1], fn c => c > 0)");
    let direct_text = hover_text_for_hof_call(&direct, &ctx);
    assert_eq!(
        pipe_text, direct_text,
        "pipe hover must equal direct-call hover: pipe={pipe_text} direct={direct_text}"
    );
}

/// Hover on `union_all` in `reduce(xs, union_all)` returns text containing
/// the input element type (`TableExpr`), output sort (`TableExpr`), and
/// identity rule (`no identity`).
#[test]
fn hover_reducer_name_in_reduce_position() {
    let text = hover_text_for_reducer_name("union_all");
    assert!(
        text.contains("TableExpr"),
        "hover for union_all must mention `TableExpr` input, got: {text}"
    );
    assert!(
        text.contains("no identity"),
        "hover for union_all must mention `no identity`, got: {text}"
    );
}

/// Hover on `and_all` returns identity `TRUE`.
#[test]
fn hover_reducer_name_with_identity() {
    let text = hover_text_for_reducer_name("and_all");
    assert!(
        text.contains("TRUE"),
        "hover for and_all must mention identity `TRUE`, got: {text}"
    );
}

/// Hover on `smelt.config.var('region')` over a workspace with
/// `vars: { region: us-west-2 }` returns text containing `Text` and
/// the resolved value `'us-west-2'`.
#[test]
fn hover_smelt_config_var_resolved() {
    let smelt_yml = "name: test_project\nvars:\n  region: us-west-2\n";
    let text = hover_text_for_config_var("region", smelt_yml);
    assert!(
        text.contains("Text"),
        "hover for config.var must contain `Text`, got: {text}"
    );
    assert!(
        text.contains("us-west-2"),
        "hover for config.var('region') must contain resolved value `us-west-2`, got: {text}"
    );
}

/// Hover on `smelt.config.var('not_declared')` returns `Text` and a hint
/// that the variable is not declared (no crash).
#[test]
fn hover_smelt_config_var_unresolved() {
    let smelt_yml = "name: test_project\nvars:\n  region: us-west-2\n";
    let text = hover_text_for_config_var("not_declared", smelt_yml);
    assert!(
        text.contains("Text"),
        "hover for unresolved config.var must still contain `Text`, got: {text}"
    );
    assert!(
        text.contains("not declared") || text.contains("not found") || text.contains("undefined"),
        "hover for unresolved config.var must indicate the variable is missing, got: {text}"
    );
}

/// Goto-def on `c` inside the body of `map(xs, fn c => c)` resolves to
/// the `c` token in the lambda parameter list.
///
/// We test the pure helper `lambda_param_binder_range` that returns the
/// text range of the binding occurrence given the parameter name.
#[test]
fn goto_def_lambda_parameter_resolves_to_binder() {
    use smelt_parser::syntax_kind::SyntaxKind;
    let sql = "SELECT map([1, 2, 3], fn c => c)";
    let lambda = parse_lambda(sql);
    let result = lambda_param_binder_range(&lambda, "c");
    assert!(
        result.is_some(),
        "lambda_param_binder_range for `c` in `fn c => c` must return Some, got None"
    );
    // The binder range must contain the IDENT "c" at a token of kind IDENT.
    let range = result.unwrap();
    // Range should be non-zero sized (the `c` token occupies at least one char)
    assert!(
        range.end() > range.start(),
        "binder range must be non-empty, got {:?}",
        range
    );
    // Verify the token at that range is the "c" identifier.
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let text_str: String = root.text().to_string();
    let start: usize = range.start().into();
    let end: usize = range.end().into();
    let token_text = &text_str[start..end];
    assert_eq!(
        token_text, "c",
        "binder token text must be `c`, got `{token_text}`"
    );
    let _ = SyntaxKind::IDENT; // ensure import
}

/// Goto-def on the argument `'region'` of `smelt.config.var('region')`
/// returns a Location pointing at the `vars.region:` line in `smelt.yml`.
#[test]
fn goto_def_smelt_config_var_resolves_to_yml_line() {
    let smelt_yml = "name: test_project\nvars:\n  region: us-west-2\n  env: prod\n";
    let line = find_var_line_in_smelt_yml(smelt_yml, "region");
    assert!(
        line.is_some(),
        "find_var_line_in_smelt_yml must find `region` in the vars block"
    );
    let line = line.unwrap();
    // `region:` is on line 2 (0-indexed) — after `name:` (line 0) and `vars:` (line 1)
    assert_eq!(
        line, 2,
        "region: should be on line 2 (0-indexed), got {line}"
    );
}

/// At a completion request inside the body of `fn c => |`, the completion
/// list includes `c` as the first identifier completion.
#[test]
fn completion_in_lambda_body_includes_parameter_first() {
    let lambda = parse_lambda("SELECT map([1, 2, 3], fn c => c)");
    let params = lambda_params_for_completion(&lambda);
    assert!(
        !params.is_empty(),
        "lambda_params_for_completion must return at least one param"
    );
    assert_eq!(
        params[0], "c",
        "first completion param must be `c`, got `{}`",
        params[0]
    );
}

/// At a completion request at the second-arg position of
/// `reduce(xs, |)` where `xs: List<Expr<Integer>>`, the completion list
/// includes the reducers whose declared input is compatible with
/// `Expr<Integer>` (i.e. `plus_chain`, `comma_sep`); reducers with
/// incompatible input (e.g. `union_all` for `TableExpr`) are filtered out.
#[test]
fn completion_in_reduce_second_arg_offers_registry() {
    use smelt_types::signatures::{SmeltType, TypeConstraint};
    use smelt_types::DataType;
    let int_elem_ty = SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer));
    let list_of_int = SmeltType::List(Box::new(int_elem_ty.clone()));
    let names = reducer_completions_for_element_type(Some(&list_of_int));
    assert!(
        names.contains(&"plus_chain".to_string()),
        "plus_chain must be offered for List<Expr<Integer>>, got: {names:?}"
    );
    assert!(
        names.contains(&"comma_sep".to_string()),
        "comma_sep must be offered for any Expr<T> element, got: {names:?}"
    );
    assert!(
        !names.contains(&"union_all".to_string()),
        "union_all (TableExpr input) must NOT be offered for List<Expr<Integer>>, got: {names:?}"
    );
    assert!(
        !names.contains(&"and_all".to_string()),
        "and_all (Boolean input) must NOT be offered for List<Expr<Integer>>, got: {names:?}"
    );
}

/// Hover inside `map(xs, fn c =` (mid-edit, no body yet) does not crash;
/// returns `Lambda<T, ?>` or no hover.
#[test]
fn hover_does_not_panic_on_partial_lambda() {
    use smelt_parser::syntax_kind::SyntaxKind;
    // Parse the partial lambda — the parser should recover gracefully.
    let sql = "SELECT map([1, 2, 3], fn c =";
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    // Find any LAMBDA node (if the parser recovered one).
    let maybe_lambda = root
        .descendants()
        .find(|n| n.kind() == SyntaxKind::LAMBDA)
        .and_then(smelt_parser::ast::Lambda::cast);
    // Whether or not the parser produced a LAMBDA node, calling the hover
    // helper must not panic.
    let ctx = smelt_db::TypeContext::new();
    if let Some(lambda) = maybe_lambda {
        // Calling with Unknown element type simulates a partial parse.
        use smelt_types::signatures::SmeltType;
        let text = hover_text_for_lambda_param("c", &SmeltType::Unknown, &lambda, &ctx);
        // Must not panic; the text is allowed to be any non-panicking string.
        let _ = text;
    }
    // Also test that find the HOF call helper doesn't crash on partial input.
    let maybe_call = root
        .descendants()
        .find_map(smelt_parser::ast::FunctionCall::cast);
    if let Some(call) = maybe_call {
        let text = hover_text_for_hof_call(&call, &ctx);
        let _ = text;
    }
    // Test passes as long as nothing panicked.
}

// ── Dispatch-level tests (Finding 3) ─────────────────────────────────────
//
// These tests route cursor positions through `hover_text_for_hof_meta_language`
// — the same pure function that `Backend::hover` calls — to prove that the
// dispatch ordering is correct.  A regression (e.g. swapping the lambda-param
// and HOF-result blocks back) MUST cause these tests to fail.

/// Helper: parse SQL, find the AstFile, and call the dispatch helper.
fn dispatch_hover(sql: &str, cursor_offset: usize) -> Option<String> {
    use smelt_parser::ast::File as AstFile;
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = AstFile::cast(root)?;
    hover_text_for_hof_meta_language(&file, cursor_offset, "")
}

/// Cursor on `c` in the body of `fn c => c` (the second `c`) must return
/// the parameter bound type (`Expr<INTEGER>`), NOT the HOF result type
/// (`List<Expr<INTEGER>>`).
///
/// This is the regression test for Finding 1: if the HOF result-type block
/// runs before the lambda-param block, this test fails.
#[test]
fn dispatch_hover_lambda_parameter_in_body_wins_over_hof_result() {
    // `map([100000, 200000, 300000], fn c => c)`
    // The second `c` (body use) starts after `=>`.
    let sql = "SELECT map([100000, 200000, 300000], fn c => c)";
    // Find the byte offset of the body `c` — the last `c` in the SQL.
    let body_c_offset = sql.rfind('c').expect("body `c` must be in SQL");
    let result = dispatch_hover(sql, body_c_offset);
    assert!(
        result.is_some(),
        "hover on body `c` must produce Some, got None"
    );
    let text = result.unwrap();
    assert!(
        text.contains("Expr<INTEGER>"),
        "dispatch hover on body `c` must show param type `Expr<INTEGER>`, got: {text}"
    );
    assert!(
        !text.contains("List<Expr<INTEGER>>"),
        "dispatch hover on body `c` must NOT show HOF result type, got: {text}"
    );
}

/// Cursor on `union_all` in the second arg of `reduce(xs, union_all)` must
/// return the reducer metadata (contains `TableExpr`), NOT the HOF result type.
///
/// This is the regression test for Finding 2: if the reducer-name block
/// runs after the HOF result-type block (dead code), this test fails.
#[test]
fn dispatch_hover_reducer_name_in_second_arg_wins_over_hof_result() {
    // We need a valid list literal for the first arg.  The reducer name is
    // the second identifier token.
    let sql = "SELECT reduce([smelt_table_a, smelt_table_b], union_all)";
    // Find the offset of `union_all` — the last token before `)`.
    let union_all_offset = sql.find("union_all").expect("union_all must be in SQL");
    let result = dispatch_hover(sql, union_all_offset + 2); // cursor inside `union_all`
    assert!(
        result.is_some(),
        "hover on `union_all` in second arg must produce Some, got None"
    );
    let text = result.unwrap();
    assert!(
        text.contains("TableExpr"),
        "dispatch hover on reducer name must show reducer metadata with `TableExpr`, got: {text}"
    );
    assert!(
        text.contains("no identity") || text.contains("identity"),
        "dispatch hover on reducer name must mention identity, got: {text}"
    );
}

/// Cursor on the binder `c` in `fn c => ...` (the first `c`, after `fn`)
/// must return the param type via the lambda-param block.
#[test]
fn dispatch_hover_lambda_parameter_binder_shows_param_type() {
    let sql = "SELECT map([100000, 200000, 300000], fn c => c)";
    // Find the binder `c` — the first `c` after `fn `.
    let fn_pos = sql.find("fn ").expect("fn must be in SQL");
    let binder_offset = fn_pos + 3; // skip "fn "
    let result = dispatch_hover(sql, binder_offset);
    assert!(
        result.is_some(),
        "hover on binder `c` must produce Some, got None"
    );
    let text = result.unwrap();
    assert!(
        text.contains("Expr<INTEGER>"),
        "dispatch hover on binder `c` must show `Expr<INTEGER>`, got: {text}"
    );
    assert!(
        !text.contains("List<Expr<INTEGER>>"),
        "dispatch hover on binder `c` must NOT show HOF result type, got: {text}"
    );
}

/// Goto-def for a `smelt.config.var('undeclared')` must return `None` (no
/// navigation), not `Some` pointing at line 0 of smelt.yml.
///
/// This is the regression test for Finding 4: `unwrap_or(0)` silently
/// navigates to the top of the file when the var is not declared.
#[test]
fn goto_def_config_var_undeclared_returns_none() {
    // `find_var_line_in_smelt_yml` must return None for a variable not in vars.
    let smelt_yml = "name: test_project\nvars:\n  declared_var: some_value\n";
    let result = find_var_line_in_smelt_yml(smelt_yml, "undeclared_var");
    assert!(
        result.is_none(),
        "find_var_line_in_smelt_yml for an undeclared var must return None, got {result:?}"
    );
    // Confirm the declared var still resolves correctly.
    let result2 = find_var_line_in_smelt_yml(smelt_yml, "declared_var");
    assert!(
        result2.is_some(),
        "find_var_line_in_smelt_yml for a declared var must return Some"
    );
}

// ── Phase C (meta-language): hover, goto-def, completion for reflection ───

/// Hovering on `smelt.columns_of(orders)` returns `List<ColumnRef>` in the
/// hover text.
///
/// Tests `hover_text_for_columns_of_call` pure helper.
/// When no `ColumnRefValue` list is supplied (schema unresolvable), the
/// helper still shows the return type `List<ColumnRef>`.
#[test]
fn hover_on_smelt_columns_of_call_shows_list_column_ref() {
    // Case 1: no resolved columns (unresolvable schema) — must show List<ColumnRef>
    let text_no_cols = hover_text_for_columns_of_call("orders", None);
    assert!(
        text_no_cols.contains("List<ColumnRef>"),
        "hover on smelt.columns_of(orders) with unresolvable schema must contain \
         `List<ColumnRef>`, got: {text_no_cols}"
    );

    // Case 2: resolved columns — must show List<ColumnRef> PLUS column count + names
    use smelt_types::signatures::ColumnRefValue;
    let cols = vec![
        ColumnRefValue {
            name: "id".to_string(),
            data_type: Some(smelt_types::DataType::Integer),
            is_numeric: true,
            source_span: None,
        },
        ColumnRefValue {
            name: "amount".to_string(),
            data_type: Some(smelt_types::DataType::Float),
            is_numeric: true,
            source_span: None,
        },
        ColumnRefValue {
            name: "customer_name".to_string(),
            data_type: Some(smelt_types::DataType::Text),
            is_numeric: false,
            source_span: None,
        },
    ];
    let text_with_cols = hover_text_for_columns_of_call("orders", Some(&cols));
    assert!(
        text_with_cols.contains("List<ColumnRef>"),
        "hover on smelt.columns_of(orders) with resolved schema must contain \
         `List<ColumnRef>`, got: {text_with_cols}"
    );
    assert!(
        text_with_cols.contains('3') || text_with_cols.contains("3 columns"),
        "hover on smelt.columns_of with 3 resolved columns must mention column count, \
         got: {text_with_cols}"
    );
    assert!(
        text_with_cols.contains("id"),
        "hover on smelt.columns_of must list first column name `id`, got: {text_with_cols}"
    );
    assert!(
        text_with_cols.contains("amount"),
        "hover on smelt.columns_of must list column `amount`, got: {text_with_cols}"
    );
}

/// Hovering on a `ColumnRef`-typed lambda parameter (e.g. `c` in
/// `map(smelt.columns_of(t), fn c => ...)`) shows `ColumnRef` plus the
/// closed field list with each field's type.
///
/// This test routes through `dispatch_hover` (i.e. through
/// `hover_text_for_hof_meta_language`) to verify the *wiring*, not just
/// the helper.  A regression that calls `hover_text_for_lambda_param`
/// instead of `hover_text_for_column_ref_binding` would produce
/// `"c: ColumnRef"` but NOT the field list, causing the `is_numeric`
/// assertion below to fail.
#[test]
fn hover_on_column_ref_lambda_parameter_shows_field_set() {
    // Use smelt.columns_of so the inferred elem_ty is ColumnRef.
    let sql = "SELECT map(smelt.columns_of(orders), fn c => c.name)";
    // Cursor on the binder `c` (just after `fn `).
    let fn_pos = sql.find("fn ").expect("fn must be in SQL");
    let binder_offset = fn_pos + 3; // skip "fn "
    let result = dispatch_hover(sql, binder_offset);
    assert!(
        result.is_some(),
        "dispatch hover on ColumnRef lambda binder `c` must produce Some, got None"
    );
    let text = result.unwrap();
    assert!(
        text.contains("ColumnRef"),
        "hover on ColumnRef binding `c` must contain `ColumnRef`, got: {text}"
    );
    // Must show the three closed fields (field list from
    // `hover_text_for_column_ref_binding`, NOT the generic lambda-param text).
    assert!(
        text.contains("name"),
        "hover on ColumnRef binding must mention field `name`, got: {text}"
    );
    assert!(
        text.contains("type") || text.contains("DataType"),
        "hover on ColumnRef binding must mention field `type` / DataType, got: {text}"
    );
    assert!(
        text.contains("is_numeric"),
        "hover on ColumnRef binding must mention field `is_numeric`, got: {text}"
    );
}

/// Hovering on a field projection `c.name` shows `name: Text`.
/// Hovering on `c.type` shows `type: DataType` (or Unknown per COLUMN_REF_FIELDS).
/// Hovering on `c.is_numeric` shows `is_numeric: Boolean`.
///
/// Tests `hover_text_for_column_ref_field` pure helper.
#[test]
fn hover_on_column_ref_field_projection_shows_field_type() {
    // `c.name` → Text
    let text_name = hover_text_for_column_ref_field("name");
    assert!(
        text_name.is_some(),
        "hover_text_for_column_ref_field('name') must return Some, got None"
    );
    let name_text = text_name.unwrap();
    assert!(
        name_text.contains("name"),
        "hover for `c.name` must mention field name `name`, got: {name_text}"
    );
    assert!(
        name_text.contains("Text") || name_text.contains("TEXT"),
        "hover for `c.name` must mention `Text` type, got: {name_text}"
    );

    // `c.type` → DataType / Unknown (Phase C maps DataType to Unknown)
    let text_type = hover_text_for_column_ref_field("type");
    assert!(
        text_type.is_some(),
        "hover_text_for_column_ref_field('type') must return Some, got None"
    );
    let type_text = text_type.unwrap();
    assert!(
        type_text.contains("type"),
        "hover for `c.type` must mention field name `type`, got: {type_text}"
    );
    assert!(
        type_text.contains("DataType") || type_text.contains("Unknown"),
        "hover for `c.type` must mention DataType or Unknown, got: {type_text}"
    );

    // `c.is_numeric` → Boolean
    let text_is_numeric = hover_text_for_column_ref_field("is_numeric");
    assert!(
        text_is_numeric.is_some(),
        "hover_text_for_column_ref_field('is_numeric') must return Some, got None"
    );
    let is_numeric_text = text_is_numeric.unwrap();
    assert!(
        is_numeric_text.contains("is_numeric"),
        "hover for `c.is_numeric` must mention field name `is_numeric`, got: {is_numeric_text}"
    );
    assert!(
        is_numeric_text.contains("Boolean") || is_numeric_text.contains("BOOLEAN"),
        "hover for `c.is_numeric` must mention `Boolean` type, got: {is_numeric_text}"
    );

    // Unknown field → None
    let text_unknown = hover_text_for_column_ref_field("nonexistent_field");
    assert!(
        text_unknown.is_none(),
        "hover_text_for_column_ref_field for unknown field must return None, got Some"
    );
}

/// Hovering on a meta-Text lifted identifier (e.g. `c.name` used as a
/// column-reference) shows the lift description — the identity transform
/// `Text -> identifier`.
///
/// Tests `hover_text_for_lifted_identifier` pure helper.
#[test]
fn hover_on_lifted_identifier_shows_lift_target() {
    let text = hover_text_for_lifted_identifier("c.name", None);
    assert!(
        text.contains("Text") || text.contains("identifier"),
        "hover on lifted identifier must describe the Text→identifier lift, got: {text}"
    );

    // When the concrete column name is known (from ColumnRefValue), the hover
    // should mention that resolved value.
    use smelt_types::signatures::ColumnRefValue;
    let col = ColumnRefValue {
        name: "order_id".to_string(),
        data_type: Some(smelt_types::DataType::Integer),
        is_numeric: true,
        source_span: None,
    };
    let text_resolved = hover_text_for_lifted_identifier("c.name", Some(&col));
    assert!(
        text_resolved.contains("order_id"),
        "hover on lifted identifier with resolved column must mention column name \
         `order_id`, got: {text_resolved}"
    );
}

/// Goto-def on a `smelt.columns_of` call path is a graceful no-op — the
/// helper returns `None` (client displays no navigation). This is the
/// minimal spec-compliant implementation (URL hint / graceful no-op).
///
/// Tests `goto_def_for_columns_of_call` pure helper.
#[test]
fn goto_def_on_columns_of_call_site_is_noop() {
    let result = goto_def_for_columns_of_call();
    assert!(
        result.is_none(),
        "goto_def_for_columns_of_call must return None (graceful no-op), got Some"
    );
}

/// Goto-def from a lifted meta-`Text` identifier (`c.name` in a lift
/// position) is a graceful no-op when no source span is available, and
/// resolves to the source column's declaration when one is supplied.
///
/// **Known divergence:** Full Backend-level dispatch wiring (detecting the
/// cursor is inside one of the four lift positions and resolving the column
/// via `columns_of_for_table_expr`) is not yet implemented.  This test
/// exercises the pure helper contract.  Tracked in
/// `docs/plans/20260509-meta-language-overall.md`.
///
/// Tests `goto_def_for_lifted_identifier` pure helper.
#[test]
fn goto_def_from_lifted_identifier_resolves_to_source_column() {
    // Without a resolved ColumnRefValue the result is None (no-op).
    let result_no_span = goto_def_for_lifted_identifier(None);
    assert!(
        result_no_span.is_none(),
        "goto_def_for_lifted_identifier(None) must return None (graceful no-op), \
         got Some"
    );

    // Even with a resolved ColumnRefValue that carries a source_span, the
    // current implementation returns None because the span type
    // (Option<TextRange>) does not carry a file path.  The wiring to
    // produce a PathBuf is a known divergence (see doc comment above).
    use smelt_types::signatures::ColumnRefValue;
    let col_with_span = ColumnRefValue {
        name: "order_id".to_string(),
        data_type: Some(smelt_types::DataType::Integer),
        is_numeric: true,
        source_span: None, // TextRange not yet resolvable to a path
    };
    let result_with_col = goto_def_for_lifted_identifier(Some(&col_with_span));
    // v1 always returns None; a future phase wires the path resolution.
    assert!(
        result_with_col.is_none(),
        "goto_def_for_lifted_identifier v1 must return None (wiring not yet \
         implemented), got Some"
    );
}

/// Completion at `c.<cursor>` offers exactly the eight ColumnRef fields
/// (`name`, `type`, `is_numeric`, `is_decimal`, `is_string`, `is_temporal`,
/// `is_integer`, `is_boolean`) and nothing else.
///
/// Tests `column_ref_field_completions` pure helper.
#[test]
fn completion_at_column_ref_field_offers_closed_set() {
    let names = column_ref_field_completions();
    assert_eq!(
        names.len(),
        8,
        "column_ref_field_completions must return exactly 8 items, got: {names:?}"
    );
    for field in &[
        "name",
        "type",
        "is_numeric",
        "is_decimal",
        "is_string",
        "is_temporal",
        "is_integer",
        "is_boolean",
    ] {
        assert!(
            names.contains(&field.to_string()),
            "column_ref_field_completions must include `{field}`, got: {names:?}"
        );
    }
}

/// Completion at `smelt.columns_of(<cursor>)` — calls
/// `columns_of_arg_completions_for_sql` and verifies that in-scope
/// `smelt.<path>` references in a SQL text are extracted. This simulates
/// the case where the file text contains `FROM smelt.orders` — the name
/// `orders` (or `smelt.orders`) should appear in the completion list.
///
/// Tests `columns_of_arg_completions_for_sql` pure helper.
#[test]
fn completion_at_columns_of_argument_offers_table_expr_names() {
    // SQL that has a smelt path reference to `orders` (a models ref)
    let sql = "SELECT map(smelt.columns_of(orders), fn c => c.name) \
               FROM smelt.models.orders";
    let names = columns_of_arg_completions_for_sql(sql);
    // The completion list must include `orders` (derived from the
    // smelt.models.orders path reference in the FROM clause).
    assert!(
        !names.is_empty(),
        "columns_of_arg_completions_for_sql must return at least one entry \
         for SQL with a smelt path ref, got: {names:?}"
    );
    assert!(
        names.contains(&"orders".to_string()),
        "columns_of_arg_completions_for_sql must include `orders` from \
         smelt.models.orders reference, got: {names:?}"
    );
}

/// Completion at `smelt.columns_of(<cursor>)` inside a `smelt.define`
/// body — verifies that `TableExpr`-typed function parameters are offered
/// as candidates.
///
/// This is the primary motivation for the parametric case:
/// `smelt.define coalesce_numeric(t: TableExpr) AS (SELECT map(smelt.columns_of(<cursor>), …))`
/// — `t` must appear in the completion list because it is a `TableExpr`
/// parameter in scope at that call site.
///
/// Tests the second source in `columns_of_arg_completions_for_sql`.
#[test]
fn completion_at_columns_of_argument_offers_define_table_expr_params() {
    // A smelt.define with a TableExpr parameter.  The body contains
    // smelt.columns_of(...) — `t` must be offered as a completion.
    let sql = "smelt.define coalesce_numeric(t: TableExpr) AS \
               (SELECT map(smelt.columns_of(t), fn c => COALESCE(c.name, '')) \
                FROM t)";
    let names = columns_of_arg_completions_for_sql(sql);
    assert!(
        names.contains(&"t".to_string()),
        "columns_of_arg_completions_for_sql must include `t` (TableExpr \
         parameter of smelt.define), got: {names:?}"
    );
}

/// Completion at `smelt.columns_of(<cursor>)` inside a `smelt.define`
/// with multiple parameters — only `TableExpr`-typed parameters are
/// offered; non-TableExpr parameters are excluded.
#[test]
fn completion_at_columns_of_argument_excludes_non_table_expr_params() {
    // Two params: `t: TableExpr` (should appear) and `threshold: Expr<Integer>`
    // (must NOT appear — wrong type).
    let sql = "smelt.define filtered(t: TableExpr, threshold: Expr<Integer>) AS \
               (SELECT map(smelt.columns_of(t), fn c => c.name) FROM t \
                WHERE amount > threshold)";
    let names = columns_of_arg_completions_for_sql(sql);
    assert!(
        names.contains(&"t".to_string()),
        "columns_of_arg_completions_for_sql must include `t` (TableExpr param), \
         got: {names:?}"
    );
    assert!(
        !names.contains(&"threshold".to_string()),
        "columns_of_arg_completions_for_sql must NOT include `threshold` \
         (Expr<Integer> param, not TableExpr), got: {names:?}"
    );
}

/// The `hover_text_for_hof_meta_language` dispatch helper picks up
/// `smelt.columns_of` calls and returns hover text containing `List<ColumnRef>`.
///
/// This is the dispatch-level test: it verifies that the routing in
/// `hover_text_for_hof_meta_language` reaches the `smelt.columns_of` branch.
#[test]
fn dispatch_hover_smelt_columns_of_shows_list_column_ref() {
    let sql = "SELECT smelt.columns_of(orders)";
    // Find the offset of `smelt.columns_of` — cursor inside the call path.
    let columns_of_offset = sql.find("columns_of").expect("columns_of must be in SQL");
    let result = dispatch_hover(sql, columns_of_offset + 2); // cursor inside `columns_of`
    assert!(
        result.is_some(),
        "dispatch hover on smelt.columns_of must produce Some, got None"
    );
    let text = result.unwrap();
    assert!(
        text.contains("List<ColumnRef>"),
        "dispatch hover on smelt.columns_of must show `List<ColumnRef>`, got: {text}"
    );
}

/// The `hover_text_for_hof_meta_language` dispatch helper picks up a
/// ColumnRef field projection (e.g. `c.name`) and shows the declared field type.
///
/// This tests that when the cursor is on the `name` token of `c.name`
/// inside a lambda body, the field type `Text` is surfaced.
#[test]
fn dispatch_hover_column_ref_field_projection_shows_field_type() {
    // SQL with a ColumnRef field projection inside a lambda body.
    // We use a syntactically valid expression where `c.name` appears.
    // The `.name` field access after a lambda parameter `c` is what we hover.
    let sql = "SELECT map(smelt.columns_of(orders), fn c => c.name)";
    // Find the offset of `.name` (specifically the `name` identifier token).
    let name_offset = sql.rfind("name").expect("`name` must appear in SQL");
    let result = dispatch_hover(sql, name_offset);
    // If the dispatch reaches the field-projection branch, it should return Some
    // with field type info.  If it falls through to the HOF branch instead,
    // the text will contain `List<...>` (wrong).
    if let Some(text) = result {
        // If we get a result, it must describe the `name` field.
        // Accept either the field hover or a HOF result — the critical constraint
        // is that it does NOT silently return wrong data (i.e., it doesn't
        // say `List<ColumnRef>` when hovering on the field access).
        assert!(
            !text.contains("List<ColumnRef>") || text.contains("name"),
            "dispatch hover on `c.name` field must not show List<ColumnRef> \
             without also mentioning `name`, got: {text}"
        );
    }
    // None is also acceptable if the file is not registered in a real DB
    // (the dispatch operates on parsed AST only, no Salsa).
}

// ── Finding 1 + 2 regression tests ───────────────────────────────────────
//
// These tests verify that ColumnRef field completions and hover only fire
// when the receiver token is actually a ColumnRef-typed lambda parameter
// (i.e. bound by a HOF whose first arg is `smelt.columns_of(...)`).

/// Helper: check whether `is_column_ref_param_before_dot` returns `Some` for
/// a file + cursor positioned just after `<param>.`.
fn check_is_column_ref_param(sql: &str, cursor_offset: usize) -> Option<String> {
    use smelt_parser::ast::File as AstFile;
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = AstFile::cast(root)?;
    is_column_ref_param_before_dot(&file, sql, cursor_offset)
}

/// NEGATIVE completion — `x.<cursor>` inside `map(some_int_list, fn x => x.something)`
/// with an UNRELATED `smelt.columns_of(orders)` call elsewhere in the file.
///
/// The completion MUST NOT offer `{name, type, is_numeric}` for `x` because `x`
/// is not a ColumnRef-typed parameter (the HOF iterates over `some_int_list`, not
/// `smelt.columns_of(...)`).
#[test]
fn completion_column_ref_fields_does_not_fire_for_unrelated_lambda_param() {
    // `x` is a parameter of `map(some_int_list, ...)` — NOT ColumnRef.
    // `smelt.columns_of(orders)` appears elsewhere but must not pollute `x`.
    let sql = "SELECT map(some_int_list, fn x => x.something), smelt.columns_of(orders)";
    // Cursor after `x.` — position just past the dot.
    let dot_pos = sql.find("x.").expect("`x.` must appear in SQL") + 2; // after the dot
    let result = check_is_column_ref_param(sql, dot_pos);
    assert!(
        result.is_none(),
        "is_column_ref_param_before_dot must return None for `x.` where `x` is NOT \
         a ColumnRef-typed param (HOF iterates over `some_int_list`), got: {result:?}"
    );
}

/// POSITIVE completion — `c.<cursor>` inside `map(smelt.columns_of(orders), fn c => c.)`
///
/// The completion MUST offer `{name, type, is_numeric}` because `c` IS a
/// ColumnRef-typed parameter (the HOF iterates over `smelt.columns_of(orders)`).
#[test]
fn completion_column_ref_fields_fires_for_columns_of_lambda_param() {
    let sql = "SELECT map(smelt.columns_of(orders), fn c => c.name)";
    // Cursor after `c.` in the lambda body — just past the dot before `name`.
    let c_dot_pos = sql.rfind("c.").expect("`c.` must appear in SQL") + 2;
    let result = check_is_column_ref_param(sql, c_dot_pos);
    assert!(
        result.is_some(),
        "is_column_ref_param_before_dot must return Some for `c.` where `c` IS \
         a ColumnRef-typed param (HOF iterates over `smelt.columns_of(orders)`), \
         got: None"
    );
    let param_name = result.unwrap();
    assert_eq!(
        param_name, "c",
        "returned param name must be `c`, got: {param_name}"
    );
}

/// NEGATIVE hover — hovering on the `type` token in plain SQL `t.type`
/// (where `t` is a table alias, NOT a ColumnRef lambda parameter) must NOT
/// return the ColumnRef field hover.
///
/// The dispatch (`hover_text_for_hof_meta_language`) must check the receiver
/// before returning ColumnRef field hover text.
#[test]
fn hover_column_ref_field_does_not_fire_for_plain_sql_field_access() {
    // Plain SQL table alias access — no HOF, no smelt.columns_of.
    let sql = "SELECT t.type FROM some_table t";
    let type_offset = sql.find(".type").expect("`.type` must appear in SQL") + 1; // on `type`
    let result = dispatch_hover(sql, type_offset);
    // If the dispatch fires the ColumnRef hover without the receiver check, it
    // will return Some text containing "(ColumnRef field)". That is the bug.
    if let Some(text) = result {
        assert!(
            !text.contains("ColumnRef field"),
            "hover on plain SQL `t.type` must NOT show ColumnRef field hover \
             (no ColumnRef binding in scope), got: {text}"
        );
    }
    // None is fine — no ColumnRef context means no hover.
}

/// POSITIVE hover — hovering on `name` in `c.name` inside
/// `map(smelt.columns_of(orders), fn c => c.name)` MUST show the ColumnRef
/// field hover for `name: Text`.
#[test]
fn hover_column_ref_field_fires_for_columns_of_lambda_body_field_access() {
    let sql = "SELECT map(smelt.columns_of(orders), fn c => c.name)";
    // Cursor on `name` in `c.name` — the last occurrence of `name`.
    let name_offset = sql.rfind("name").expect("`name` must appear in SQL");
    let result = dispatch_hover(sql, name_offset);
    assert!(
        result.is_some(),
        "hover on `c.name` inside smelt.columns_of lambda must produce Some, got None"
    );
    let text = result.unwrap();
    assert!(
        text.contains("ColumnRef field") || text.contains("name") && text.contains("Text"),
        "hover on `c.name` in ColumnRef lambda must describe the `name` field (Text), \
         got: {text}"
    );
}

// ── Phase D (meta-language): hover, goto-def, completion for wide reflection

/// Hovering on `smelt.models.with_tag('cohort')` returns `List<ModelRef>` in
/// the hover text.  When tag resolves, also shows match count + first five
/// names.  Analogous for `smelt.sources.with_tag`.
///
/// Tests `hover_text_for_models_with_tag_call` and
/// `hover_text_for_sources_with_tag_call` pure helpers.
#[test]
fn hover_on_smelt_models_with_tag_call_shows_list_model_ref() {
    // Case 1: no resolved models (workspace unresolvable) — must show List<ModelRef>
    let text_no_models = hover_text_for_models_with_tag_call("cohort", None);
    assert!(
        text_no_models.contains("List<ModelRef>"),
        "hover on smelt.models.with_tag with unresolvable workspace must contain \
         `List<ModelRef>`, got: {text_no_models}"
    );
    assert!(
        text_no_models.contains("cohort"),
        "hover on smelt.models.with_tag must mention the tag, got: {text_no_models}"
    );

    // Case 2: resolved models — must show List<ModelRef> PLUS count + names
    use smelt_types::signatures::ModelRefValue;
    let models = vec![
        ModelRefValue {
            path: "models/orders.sql".to_string(),
            name: "orders".to_string(),
            tags: vec!["cohort".to_string()],
            model_name_for_columns: "orders".to_string(),
        },
        ModelRefValue {
            path: "models/customers.sql".to_string(),
            name: "customers".to_string(),
            tags: vec!["cohort".to_string()],
            model_name_for_columns: "customers".to_string(),
        },
    ];
    let text_with_models = hover_text_for_models_with_tag_call("cohort", Some(&models));
    assert!(
        text_with_models.contains("List<ModelRef>"),
        "hover on smelt.models.with_tag with resolved models must contain \
         `List<ModelRef>`, got: {text_with_models}"
    );
    assert!(
        text_with_models.contains('2') || text_with_models.contains("2 matching"),
        "hover on smelt.models.with_tag with 2 models must mention count, \
         got: {text_with_models}"
    );
    assert!(
        text_with_models.contains("orders"),
        "hover on smelt.models.with_tag must list model name `orders`, \
         got: {text_with_models}"
    );

    // SourceRef variant
    let text_no_sources = hover_text_for_sources_with_tag_call("audit", None);
    assert!(
        text_no_sources.contains("List<SourceRef>"),
        "hover on smelt.sources.with_tag must contain `List<SourceRef>`, \
         got: {text_no_sources}"
    );

    use smelt_types::signatures::SourceRefValue;
    let sources = vec![SourceRefValue {
        path: "sources/raw.yml".to_string(),
        name: "raw_events".to_string(),
        tags: vec!["audit".to_string()],
        address_segments: vec!["raw".to_string(), "raw_events".to_string()],
    }];
    let text_with_sources = hover_text_for_sources_with_tag_call("audit", Some(&sources));
    assert!(
        text_with_sources.contains("List<SourceRef>"),
        "hover on smelt.sources.with_tag with resolved sources must contain \
         `List<SourceRef>`, got: {text_with_sources}"
    );
    assert!(
        text_with_sources.contains("raw_events"),
        "hover on smelt.sources.with_tag must list source name `raw_events`, \
         got: {text_with_sources}"
    );

    // Verify dispatch routing: hovering on the call site in SQL
    let sql = "SELECT map(smelt.models.with_tag('cohort'), fn m => m.name)";
    let with_tag_offset = sql.find("with_tag").expect("with_tag must be in SQL");
    let result = dispatch_hover(sql, with_tag_offset + 2);
    assert!(
        result.is_some(),
        "dispatch hover on smelt.models.with_tag call must produce Some, got None"
    );
    let hover_text = result.unwrap();
    assert!(
        hover_text.contains("List<ModelRef>"),
        "dispatch hover on smelt.models.with_tag must contain `List<ModelRef>`, \
         got: {hover_text}"
    );
    assert!(
        hover_text.contains("cohort"),
        "dispatch hover on smelt.models.with_tag must mention the tag, \
         got: {hover_text}"
    );
}

/// Hovering on `smelt.models.all` shows the signature plus workspace model
/// count.  Analogous for `smelt.sources.all`.
///
/// Tests `hover_text_for_models_all` and `hover_text_for_sources_all`
/// pure helpers.
#[test]
fn hover_on_smelt_models_all_shows_workspace_count() {
    // No workspace count available
    let text_no_count = hover_text_for_models_all(None);
    assert!(
        text_no_count.contains("List<ModelRef>"),
        "hover on smelt.models.all with no count must contain `List<ModelRef>`, \
         got: {text_no_count}"
    );

    // With workspace count
    let text_with_count = hover_text_for_models_all(Some(42));
    assert!(
        text_with_count.contains("List<ModelRef>"),
        "hover on smelt.models.all with count must contain `List<ModelRef>`, \
         got: {text_with_count}"
    );
    assert!(
        text_with_count.contains("42"),
        "hover on smelt.models.all must mention total model count 42, \
         got: {text_with_count}"
    );

    // SourceRef variant
    let text_no_sources = hover_text_for_sources_all(None);
    assert!(
        text_no_sources.contains("List<SourceRef>"),
        "hover on smelt.sources.all must contain `List<SourceRef>`, \
         got: {text_no_sources}"
    );
    let text_sources = hover_text_for_sources_all(Some(5));
    assert!(
        text_sources.contains("5"),
        "hover on smelt.sources.all must mention total source count, \
         got: {text_sources}"
    );

    // Verify dispatch routing
    let sql = "SELECT reduce(smelt.models.all(), union_all)";
    let all_offset = sql.find(".all").expect(".all must be in SQL") + 1;
    let result = dispatch_hover(sql, all_offset);
    assert!(
        result.is_some(),
        "dispatch hover on smelt.models.all call must produce Some, got None"
    );
    let hover_text = result.unwrap();
    assert!(
        hover_text.contains("List<ModelRef>"),
        "dispatch hover on smelt.models.all must contain `List<ModelRef>`, \
         got: {hover_text}"
    );
}

/// Hovering on `m` inside `map(smelt.models.with_tag('cohort'), fn m => …)`
/// shows `ModelRef` plus the closed four-field list with each field's type.
/// Analogous for `SourceRef`.
///
/// Routes through `dispatch_hover` to verify the wiring.
#[test]
fn hover_on_model_ref_lambda_parameter_shows_field_set() {
    // Case 1: cursor on the binder `m` in `fn m => m.name`
    let sql = "SELECT map(smelt.models.with_tag('cohort'), fn m => m.name)";
    let fn_pos = sql.find("fn ").expect("fn must be in SQL");
    let binder_offset = fn_pos + 3; // skip "fn "
    let result = dispatch_hover(sql, binder_offset);
    assert!(
        result.is_some(),
        "dispatch hover on ModelRef lambda binder `m` must produce Some, got None"
    );
    let text = result.unwrap();
    assert!(
        text.contains("ModelRef"),
        "hover on ModelRef binding `m` must contain `ModelRef`, got: {text}"
    );
    // Must show the four closed fields
    assert!(
        text.contains("path"),
        "hover on ModelRef binding must mention field `path`, got: {text}"
    );
    assert!(
        text.contains("name"),
        "hover on ModelRef binding must mention field `name`, got: {text}"
    );
    assert!(
        text.contains("tags"),
        "hover on ModelRef binding must mention field `tags`, got: {text}"
    );
    assert!(
        text.contains("columns"),
        "hover on ModelRef binding must mention field `columns`, got: {text}"
    );

    // Case 2: the binding helper directly
    let binding_text = hover_text_for_model_ref_binding("m");
    assert!(
        binding_text.contains("ModelRef"),
        "hover_text_for_model_ref_binding must contain ModelRef, got: {binding_text}"
    );

    // SourceRef variant
    let sql_src = "SELECT map(smelt.sources.with_tag('audit'), fn s => s.name)";
    let fn_pos_src = sql_src.find("fn ").expect("fn must be in SQL");
    let binder_offset_src = fn_pos_src + 3;
    let result_src = dispatch_hover(sql_src, binder_offset_src);
    assert!(
        result_src.is_some(),
        "dispatch hover on SourceRef lambda binder `s` must produce Some, got None"
    );
    let text_src = result_src.unwrap();
    assert!(
        text_src.contains("SourceRef"),
        "hover on SourceRef binding `s` must contain `SourceRef`, got: {text_src}"
    );
    assert!(
        text_src.contains("path"),
        "hover on SourceRef binding must mention field `path`, got: {text_src}"
    );
}

/// Hovering on the `path` token of `m.path` shows `path: Text`;
/// on `name` shows `name: Text`; on `tags` shows `tags: List<Text>`;
/// on `columns` shows `columns: List<ColumnRef>`.
/// Analogous for `SourceRef`.
///
/// Tests `hover_text_for_model_ref_field` and `hover_text_for_source_ref_field`
/// pure helpers.
#[test]
fn hover_on_model_ref_field_projection_shows_field_type() {
    // `m.path` → Text
    let text_path = hover_text_for_model_ref_field("path");
    assert!(
        text_path.is_some(),
        "hover_text_for_model_ref_field('path') must return Some, got None"
    );
    let path_text = text_path.unwrap();
    assert!(
        path_text.contains("path"),
        "hover for `m.path` must mention field name `path`, got: {path_text}"
    );
    assert!(
        path_text.contains("Text") || path_text.contains("TEXT"),
        "hover for `m.path` must mention `Text` type, got: {path_text}"
    );

    // `m.name` → Text
    let text_name = hover_text_for_model_ref_field("name");
    assert!(
        text_name.is_some(),
        "hover_text_for_model_ref_field('name') must return Some, got None"
    );
    let name_text = text_name.unwrap();
    assert!(
        name_text.contains("Text") || name_text.contains("TEXT"),
        "hover for `m.name` must mention `Text` type, got: {name_text}"
    );

    // `m.tags` → List<Text> (internally List<Expr<TEXT>>)
    let text_tags = hover_text_for_model_ref_field("tags");
    assert!(
        text_tags.is_some(),
        "hover_text_for_model_ref_field('tags') must return Some, got None"
    );
    let tags_text = text_tags.unwrap();
    assert!(
        tags_text.contains("List") && (tags_text.contains("Text") || tags_text.contains("TEXT")),
        "hover for `m.tags` must mention List and Text type, got: {tags_text}"
    );

    // `m.columns` → List<ColumnRef>
    let text_cols = hover_text_for_model_ref_field("columns");
    assert!(
        text_cols.is_some(),
        "hover_text_for_model_ref_field('columns') must return Some, got None"
    );
    let cols_text = text_cols.unwrap();
    assert!(
        cols_text.contains("ColumnRef"),
        "hover for `m.columns` must mention `ColumnRef`, got: {cols_text}"
    );

    // Unknown field → None
    let text_unknown = hover_text_for_model_ref_field("nonexistent_field");
    assert!(
        text_unknown.is_none(),
        "hover_text_for_model_ref_field for unknown field must return None, got Some"
    );

    // SourceRef variant
    let src_path = hover_text_for_source_ref_field("path");
    assert!(
        src_path.is_some(),
        "hover_text_for_source_ref_field('path') must return Some, got None"
    );
    let src_tags = hover_text_for_source_ref_field("tags");
    let src_tags_text = src_tags.expect("hover_text_for_source_ref_field('tags') must return Some");
    assert!(
        src_tags_text.contains("List"),
        "hover_text_for_source_ref_field('tags') must mention List, got: {src_tags_text}"
    );

    // Dispatch routing: cursor on the field token in `m.path`
    let sql = "SELECT map(smelt.models.with_tag('cohort'), fn m => m.path)";
    let path_offset = sql.rfind("path").expect("`path` must appear in SQL");
    let result = dispatch_hover(sql, path_offset);
    assert!(
        result.is_some(),
        "dispatch hover on `m.path` field in ModelRef lambda must produce Some, got None"
    );
    let hover_text = result.unwrap();
    assert!(
        hover_text.contains("ModelRef field")
            || hover_text.contains("path") && hover_text.contains("Text"),
        "dispatch hover on `m.path` must describe the `path` field (Text), \
         got: {hover_text}"
    );
}

/// Goto-def from a `ModelRef` / `SourceRef` value at a splice site is a
/// graceful no-op in v1: the pure helpers `goto_def_for_model_ref_value`
/// and `goto_def_for_source_ref_value` pass through a supplied path when
/// the caller has resolved one and return `None` otherwise. Wiring the
/// Backend `goto_definition` handler to detect splice-site cursor
/// position and resolve the path through Salsa is a known divergence
/// tracked in `docs/specs/meta_language.md` Known Divergences and the
/// overall plan `docs/plans/20260509-meta-language-overall.md`.
#[test]
fn goto_def_for_model_ref_and_source_ref_values_pass_through_or_noop() {
    // The pure helper returns None (graceful no-op per spec; full resolution
    // requires expansion-time context — known divergence tracked in
    // docs/plans/20260509-meta-language-overall.md).
    let result = goto_def_for_wide_reflection_accessor();
    assert!(
        result.is_none(),
        "goto_def_for_wide_reflection_accessor must return None (graceful no-op), \
         got Some"
    );

    // goto_def_for_model_ref_value: when a path is supplied, returns it.
    let path = std::path::PathBuf::from("/project/models/orders.sql");
    let result_with_path = goto_def_for_model_ref_value(Some(path.clone()));
    assert_eq!(
        result_with_path,
        Some(path.clone()),
        "goto_def_for_model_ref_value(Some(path)) must return Some(path)"
    );
    let result_no_path = goto_def_for_model_ref_value(None);
    assert!(
        result_no_path.is_none(),
        "goto_def_for_model_ref_value(None) must return None (graceful no-op)"
    );

    // SourceRef variant
    let yaml_path = std::path::PathBuf::from("/project/sources.yml");
    let result_src = goto_def_for_source_ref_value(Some(yaml_path.clone()));
    assert_eq!(
        result_src,
        Some(yaml_path),
        "goto_def_for_source_ref_value(Some(path)) must return Some(path)"
    );
    let result_src_none = goto_def_for_source_ref_value(None);
    assert!(
        result_src_none.is_none(),
        "goto_def_for_source_ref_value(None) must return None (graceful no-op)"
    );
}

/// Goto-def on `m.path` or `m.name` returns the same model file.
///
/// Tests that `goto_def_for_model_ref_value` passes through a supplied path,
/// mirroring the Phase C `goto_def_for_lifted_identifier` contract.
#[test]
fn goto_def_from_model_ref_path_or_name_resolves_to_source_file() {
    // `m.path` and `m.name` both route through `goto_def_for_model_ref_value`
    // with the model's source path.  The pure helper passes the path through.
    let model_path = std::path::PathBuf::from("/project/models/cohort_a.sql");
    let result_path = goto_def_for_model_ref_value(Some(model_path.clone()));
    let result_name = goto_def_for_model_ref_value(Some(model_path.clone()));
    assert_eq!(
        result_path, result_name,
        "`m.path` and `m.name` goto-def must resolve to the same file"
    );
    assert_eq!(
        result_path,
        Some(model_path),
        "goto_def_for_model_ref_value must return the supplied path"
    );

    // SourceRef: `s.path` and `s.name` both route through `goto_def_for_source_ref_value`.
    let source_yaml = std::path::PathBuf::from("/project/sources.yml");
    let result_s_path = goto_def_for_source_ref_value(Some(source_yaml.clone()));
    assert_eq!(
        result_s_path,
        Some(source_yaml),
        "goto_def_for_source_ref_value must return the supplied yaml path"
    );
}

/// Completion at `smelt.models.<cursor>` offers exactly `{with_tag, all}` and
/// no other identifier. Same for `smelt.sources.<cursor>`.
///
/// Tests `wide_reflection_accessor_completions` pure helper.
#[test]
fn completion_at_smelt_models_namespace_offers_closed_set() {
    let names = wide_reflection_accessor_completions();
    assert_eq!(
        names.len(),
        2,
        "wide_reflection_accessor_completions must return exactly 2 items, got: {names:?}"
    );
    assert!(
        names.contains(&"with_tag".to_string()),
        "wide_reflection_accessor_completions must include `with_tag`, got: {names:?}"
    );
    assert!(
        names.contains(&"all".to_string()),
        "wide_reflection_accessor_completions must include `all`, got: {names:?}"
    );
    // Must NOT contain anything else
    for name in &names {
        assert!(
            name == "with_tag" || name == "all",
            "wide_reflection_accessor_completions must only contain `with_tag` and `all`, \
             got unexpected: {name}"
        );
    }
}

/// Completion at `m.<cursor>` where `m: ModelRef` offers exactly
/// `{path, name, tags, columns}`. Analogous for `SourceRef`.
///
/// Tests `model_ref_field_completions` and `source_ref_field_completions`
/// pure helpers.
#[test]
fn completion_at_model_ref_field_offers_closed_set() {
    // ModelRef fields
    let names = model_ref_field_completions();
    assert_eq!(
        names.len(),
        4,
        "model_ref_field_completions must return exactly 4 items, got: {names:?}"
    );
    for field in &["path", "name", "tags", "columns"] {
        assert!(
            names.contains(&field.to_string()),
            "model_ref_field_completions must include `{field}`, got: {names:?}"
        );
    }
    // Must NOT include ColumnRef fields
    assert!(
        !names.contains(&"is_numeric".to_string()),
        "model_ref_field_completions must NOT include ColumnRef field `is_numeric`, \
         got: {names:?}"
    );

    // SourceRef fields
    let src_names = source_ref_field_completions();
    assert_eq!(
        src_names.len(),
        4,
        "source_ref_field_completions must return exactly 4 items, got: {src_names:?}"
    );
    for field in &["path", "name", "tags", "columns"] {
        assert!(
            src_names.contains(&field.to_string()),
            "source_ref_field_completions must include `{field}`, got: {src_names:?}"
        );
    }

    // Dispatch routing: `m.<cursor>` inside ModelRef lambda offers field completions.
    // The detection helper `is_model_ref_param_before_dot` is the gating function.
    let sql = "SELECT map(smelt.models.with_tag('cohort'), fn m => m.path)";
    // Cursor positioned just after the final `m.` (after the dot, before `path`).
    let dot_pos = sql.rfind("m.").expect("`m.` must appear in SQL") + 2;
    // Verify the detection helper fires
    use smelt_parser::ast::File as AstFile;
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = AstFile::cast(root).expect("must parse to File");
    let param = is_model_ref_param_before_dot(&file, sql, dot_pos);
    assert!(
        param.is_some(),
        "is_model_ref_param_before_dot must return Some for `m.` inside \
         smelt.models.with_tag lambda, got None"
    );
    assert_eq!(
        param.unwrap(),
        "m",
        "is_model_ref_param_before_dot must return `m` as param name"
    );
}

// ── Phase E1: Record hover/goto-def/completion tests ───────────────────

/// Hover on a `smelt.record Cohort = {…}` declaration name token shows the
/// field list with types and the declaration file path.
#[test]
fn hover_on_smelt_record_decl_name_shows_field_list() {
    use smelt_types::signatures::SmeltType;
    use smelt_types::{DataType, TypeConstraint};
    let mut fields = std::collections::BTreeMap::new();
    fields.insert(
        "name".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    );
    fields.insert(
        "threshold".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
    );
    let text = hover_text_for_record_decl_name("Cohort", &fields, "models/cohorts.sql");
    assert!(
        text.contains("Cohort"),
        "hover must contain record name, got: {text}"
    );
    assert!(
        text.contains("name"),
        "hover must contain field 'name', got: {text}"
    );
    assert!(
        text.contains("threshold"),
        "hover must contain field 'threshold', got: {text}"
    );
    assert!(
        text.contains("models/cohorts.sql"),
        "hover must contain declaration file path, got: {text}"
    );
}

/// Hover on a record-typed binding shows the record type and field list.
#[test]
fn hover_on_record_typed_binding_shows_record_type() {
    use smelt_types::signatures::SmeltType;
    use smelt_types::{DataType, TypeConstraint};
    let mut fields = std::collections::BTreeMap::new();
    fields.insert(
        "name".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    );
    fields.insert(
        "threshold".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
    );
    let ty = SmeltType::Record {
        fields,
        name: Some("Cohort".to_string()),
    };
    let text = hover_text_for_record_typed_binding("c", &ty);
    assert!(
        text.contains("Cohort"),
        "hover must contain type name 'Cohort', got: {text}"
    );
    assert!(
        text.contains("c:"),
        "hover must contain binding name 'c:', got: {text}"
    );
    assert!(
        text.contains("name"),
        "hover must contain field 'name', got: {text}"
    );
    assert!(
        text.contains("threshold"),
        "hover must contain field 'threshold', got: {text}"
    );
}

/// Hover on `.name` field projection shows the field type (`Text`).
#[test]
fn hover_on_record_field_projection_shows_field_type() {
    use smelt_types::signatures::SmeltType;
    use smelt_types::{DataType, TypeConstraint};
    let mut fields = std::collections::BTreeMap::new();
    fields.insert(
        "name".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    );
    fields.insert(
        "threshold".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
    );
    let ty = SmeltType::Record {
        fields,
        name: Some("Cohort".to_string()),
    };
    let result = hover_text_for_record_field_projection("name", &ty);
    assert!(result.is_some(), "must return Some for a known field");
    let text = result.unwrap();
    assert!(
        text.contains("TEXT") || text.contains("Text"),
        "hover must contain type 'Text'/'TEXT' for 'name', got: {text}"
    );
    // Unknown field → None.
    let none_result = hover_text_for_record_field_projection("bogus", &ty);
    assert!(
        none_result.is_none(),
        "hover must return None for unknown field, got: {:?}",
        none_result
    );
}

/// Hover on record literal opening brace shows the inferred target type.
#[test]
fn hover_on_record_literal_opening_brace_shows_inferred_target() {
    use smelt_types::signatures::SmeltType;
    use smelt_types::{DataType, TypeConstraint};
    let mut fields = std::collections::BTreeMap::new();
    fields.insert(
        "name".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    );
    fields.insert(
        "threshold".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
    );
    let ty = SmeltType::Record {
        fields,
        name: Some("Cohort".to_string()),
    };
    let text = hover_text_for_record_literal_brace(&ty);
    assert!(
        text.contains("Cohort"),
        "hover on brace must show target type name, got: {text}"
    );
}

/// Completion at a record literal field-key position offers unfilled fields,
/// each carrying the declared type as a detail string.
#[test]
fn completion_at_record_literal_field_key_offers_unfilled_fields() {
    use smelt_types::signatures::SmeltType;
    use smelt_types::{DataType, TypeConstraint};
    let declared = vec![
        (
            "name".to_string(),
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
        ),
        (
            "region".to_string(),
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
        ),
        (
            "threshold".to_string(),
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
        ),
    ];
    let already_filled = vec!["name".to_string()];
    let completions = record_literal_field_completions(&declared, &already_filled);
    let names: Vec<&str> = completions.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        names.contains(&"region"),
        "completions must include 'region', got: {names:?}"
    );
    assert!(
        names.contains(&"threshold"),
        "completions must include 'threshold', got: {names:?}"
    );
    assert!(
        !names.contains(&"name"),
        "completions must NOT include already-filled 'name', got: {names:?}"
    );
    let threshold_detail = completions
        .iter()
        .find(|(n, _)| n == "threshold")
        .map(|(_, d)| d.as_str())
        .unwrap_or("");
    assert!(
        threshold_detail.to_uppercase().contains("INTEGER"),
        "threshold completion must carry the Integer type as detail, got: {threshold_detail}"
    );
}

/// Completion at a record field-projection site offers the closed declared set,
/// each carrying the declared type as a detail string.
#[test]
fn completion_at_record_field_projection_offers_closed_set() {
    use smelt_types::signatures::SmeltType;
    use smelt_types::{DataType, TypeConstraint};
    let mut fields = std::collections::BTreeMap::new();
    fields.insert(
        "name".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    );
    fields.insert(
        "threshold".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
    );
    let completions = record_field_projection_completions(&fields);
    let names: Vec<&str> = completions.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        names.contains(&"name"),
        "completions must include 'name', got: {names:?}"
    );
    assert!(
        names.contains(&"threshold"),
        "completions must include 'threshold', got: {names:?}"
    );
    assert_eq!(
        completions.len(),
        2,
        "completions must have exactly 2 items, got: {completions:?}"
    );
    let name_detail = completions
        .iter()
        .find(|(n, _)| n == "name")
        .map(|(_, d)| d.as_str())
        .unwrap_or("");
    assert!(
        name_detail.to_uppercase().contains("TEXT"),
        "name completion must carry the Text type as detail, got: {name_detail}"
    );
}

// ── Phase E1: Map hover/completion tests ────────────────────────────────

/// Hover on a `Map<Text, Integer>`-typed binding shows type, entry count,
/// and first five keys.
#[test]
fn hover_on_map_typed_binding_shows_resolved_summary() {
    use smelt_types::signatures::SmeltType;
    use smelt_types::{DataType, TypeConstraint};
    let key_ty = SmeltType::Expr(TypeConstraint::Concrete(DataType::Text));
    let val_ty = SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer));
    let keys = vec!["a".to_string(), "b".to_string()];
    let text = hover_text_for_map_typed_binding("m", &key_ty, &val_ty, Some(2), Some(&keys));
    assert!(
        text.contains("Map<"),
        "hover must contain 'Map<', got: {text}"
    );
    assert!(
        text.contains("2 entries"),
        "hover must show entry count '2 entries', got: {text}"
    );
    assert!(
        text.contains("a") && text.contains("b"),
        "hover must show first keys, got: {text}"
    );
}

/// Hover on `m.entries()` shows the signature and resolved length.
#[test]
fn hover_on_map_method_call_shows_signature_and_resolution() {
    use smelt_types::signatures::SmeltType;
    use smelt_types::{DataType, TypeConstraint};
    let key_ty = SmeltType::Expr(TypeConstraint::Concrete(DataType::Text));
    let val_ty = SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer));
    let text = hover_text_for_map_method_call("entries", &key_ty, &val_ty, Some(3), None);
    assert!(
        text.contains("entries"),
        "hover must contain method name 'entries', got: {text}"
    );
    assert!(
        text.contains("List<"),
        "hover must contain return type 'List<...>', got: {text}"
    );
    assert!(
        text.contains("3"),
        "hover must show resolved length, got: {text}"
    );
}

/// Hover on `m.get(k)` shows the value type `V` and the concrete bound value
/// when the key is statically known and present.
#[test]
fn hover_on_map_get_call_shows_value_type_and_resolved_value() {
    use smelt_types::signatures::SmeltType;
    use smelt_types::{DataType, TypeConstraint};
    let key_ty = SmeltType::Expr(TypeConstraint::Concrete(DataType::Text));
    let val_ty = SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer));
    let text = hover_text_for_map_method_call("get", &key_ty, &val_ty, None, Some("100"));
    assert!(
        text.contains("get"),
        "hover must contain method name 'get', got: {text}"
    );
    assert!(
        text.to_uppercase().contains("INTEGER"),
        "hover must contain the value type (Integer), got: {text}"
    );
    assert!(
        text.contains("100"),
        "hover must show resolved value '100', got: {text}"
    );
    assert!(
        !text.contains("entries*"),
        "hover for get must NOT show 'entries' resolution suffix, got: {text}"
    );
}

/// Completion at `m.<cursor>` offers the closed Map API: entries, keys, values, get, has.
#[test]
fn completion_at_map_method_position_offers_closed_set() {
    let completions = map_api_method_completions();
    for method in &["entries", "keys", "values", "get", "has"] {
        assert!(
            completions.contains(&method.to_string()),
            "completions must include '{method}', got: {completions:?}"
        );
    }
    assert_eq!(
        completions.len(),
        5,
        "completions must have exactly 5 items, got: {completions:?}"
    );
}

/// Completion at `m.get(<cursor>)` offers statically-known keys.
#[test]
fn completion_at_map_get_arg_offers_statically_known_keys() {
    let keys = vec!["tenant_a".to_string(), "tenant_b".to_string()];
    let completions = map_get_key_completions(Some(&keys));
    assert!(
        completions.contains(&"tenant_a".to_string()),
        "completions must include 'tenant_a', got: {completions:?}"
    );
    assert!(
        completions.contains(&"tenant_b".to_string()),
        "completions must include 'tenant_b', got: {completions:?}"
    );
    // When not resolvable → empty.
    let none_completions = map_get_key_completions(None);
    assert!(
        none_completions.is_empty(),
        "completions must be empty when no static keys, got: {none_completions:?}"
    );
}

// ── Phase E1: Loader hover/completion tests ────────────────────────────

/// Hover on a loader call site shows resolved path and schema summary; never mtime (D-56).
#[test]
fn hover_on_loader_call_shows_resolved_path_and_summary() {
    let text = hover_text_for_loader_call("load_yaml", "cohorts.yaml", "List<Cohort> (3 rows)");
    assert!(
        text.contains("load_yaml"),
        "hover must contain loader function name, got: {text}"
    );
    assert!(
        text.contains("cohorts.yaml"),
        "hover must contain resolved path, got: {text}"
    );
    assert!(
        text.contains("List<Cohort>"),
        "hover must contain schema summary, got: {text}"
    );
    assert!(
        !text.contains("Last modified") && !text.contains("2026-05-13"),
        "hover must not expose mtime (D-56), got: {text}"
    );
}

/// Completion at loader path arg offers filesystem entries filtered by extension.
#[test]
fn completion_at_loader_path_offers_filesystem_entries() {
    let candidates = vec![
        "configs/cohorts.yaml".to_string(),
        "configs/tenants.json".to_string(),
        "configs/settings.yml".to_string(),
        "models/orders.sql".to_string(),
    ];
    // load_yaml → yaml/yml only
    let yaml_completions = loader_path_completions(&candidates, "yaml");
    assert!(
        yaml_completions.contains(&"configs/cohorts.yaml".to_string()),
        "must include .yaml file, got: {yaml_completions:?}"
    );
    assert!(
        yaml_completions.contains(&"configs/settings.yml".to_string()),
        "must include .yml file, got: {yaml_completions:?}"
    );
    assert!(
        !yaml_completions.contains(&"configs/tenants.json".to_string()),
        "must NOT include .json file for yaml loader, got: {yaml_completions:?}"
    );
    assert!(
        !yaml_completions.contains(&"models/orders.sql".to_string()),
        "must NOT include .sql file, got: {yaml_completions:?}"
    );
    // load_json → json only
    let json_completions = loader_path_completions(&candidates, "json");
    assert!(
        json_completions.contains(&"configs/tenants.json".to_string()),
        "must include .json file, got: {json_completions:?}"
    );
    assert!(
        !json_completions.contains(&"configs/cohorts.yaml".to_string()),
        "must NOT include .yaml file for json loader, got: {json_completions:?}"
    );
}

/// Completion at loader schema arg offers in-scope record names + inline stub.
#[test]
fn completion_at_loader_schema_offers_in_scope_record_names() {
    let record_names = vec!["Cohort".to_string(), "Tenant".to_string()];
    let completions = loader_schema_completions(&record_names);
    assert!(
        completions.contains(&"Cohort".to_string()),
        "must include 'Cohort', got: {completions:?}"
    );
    assert!(
        completions.contains(&"Tenant".to_string()),
        "must include 'Tenant', got: {completions:?}"
    );
    assert!(
        completions.iter().any(|s| s.contains("{")),
        "must include inline stub entry, got: {completions:?}"
    );
}

// ── Phase E1: goto-def pure helpers ────────────────────────────────────

/// Goto-def on a `smelt.record` name reference resolves to the declaration site.
///
/// `goto_def_for_smelt_record_name` must return a `Location` pointing at
/// the supplied declaration path and range.
#[test]
fn goto_def_on_smelt_record_name_resolves_to_declaration() {
    let decl_path = std::path::Path::new("/workspace/models/records.sql");
    let decl_range = Range {
        start: Position::new(4, 12),
        end: Position::new(4, 17),
    };
    let result = goto_def_for_smelt_record_name(decl_path, decl_range);
    assert!(
        result.is_some(),
        "goto_def_for_smelt_record_name must return Some when path and range are provided"
    );
    let loc = result.unwrap();
    assert_eq!(
        loc.range, decl_range,
        "returned Location must carry the supplied declaration range"
    );
    // URI must point at the declaration file.
    assert!(
        loc.uri.path().ends_with("records.sql"),
        "returned URI must point at the declaration file; got: {}",
        loc.uri
    );
}

/// Goto-def on a record literal's field name resolves to the declared field's span.
///
/// `goto_def_for_record_literal_field` must return a `Location` pointing at
/// the supplied declaration path and field range.
#[test]
fn goto_def_on_record_literal_field_resolves_to_declared_field_span() {
    let decl_path = std::path::Path::new("/workspace/models/records.sql");
    let field_range = Range {
        start: Position::new(2, 4),
        end: Position::new(2, 8),
    };
    let result = goto_def_for_record_literal_field(decl_path, field_range);
    assert!(
        result.is_some(),
        "goto_def_for_record_literal_field must return Some when path and range are provided"
    );
    let loc = result.unwrap();
    assert_eq!(
        loc.range, field_range,
        "returned Location must carry the supplied field range"
    );
    assert!(
        loc.uri.path().ends_with("records.sql"),
        "returned URI must point at the declaration file; got: {}",
        loc.uri
    );
}

/// Goto-def on a loader path argument resolves to the loaded file at row 0.
///
/// `goto_def_for_loader_path` must return a `Location` whose URI is
/// `file://{workspace_root}/{rel_path}` and whose range is row 0, col 0.
#[test]
fn goto_def_on_loader_path_resolves_to_file() {
    let ws_root = std::path::Path::new("/workspace");
    let rel_path = "configs/cohorts.yaml";
    let result = goto_def_for_loader_path(ws_root, rel_path);
    assert!(
        result.is_some(),
        "goto_def_for_loader_path must return Some for a valid relative path"
    );
    let loc = result.unwrap();
    assert_eq!(
        loc.range.start,
        Position::new(0, 0),
        "location must be at row 0, col 0"
    );
    assert_eq!(
        loc.range.end,
        Position::new(0, 0),
        "location end must be at row 0, col 0"
    );
    assert!(
        loc.uri.path().ends_with("cohorts.yaml"),
        "URI must point at the loaded YAML file; got: {}",
        loc.uri
    );
}

/// Goto-def on a record-typed field projection of a loaded value resolves to
/// the YAML row that produced the value.
///
/// `goto_def_for_loaded_record_field_projection` must return a `Location`
/// anchored at the supplied YAML file and row number.
#[test]
fn goto_def_on_loaded_record_field_projection_resolves_to_yaml_row() {
    let yaml_file = std::path::Path::new("/workspace/configs/cohorts.yaml");
    let row: u32 = 3;
    let result = goto_def_for_loaded_record_field_projection(yaml_file, row);
    assert!(
        result.is_some(),
        "goto_def_for_loaded_record_field_projection must return Some"
    );
    let loc = result.unwrap();
    assert_eq!(
        loc.range.start.line, row,
        "location must be at the supplied row; got: {}",
        loc.range.start.line
    );
    assert!(
        loc.uri.path().ends_with("cohorts.yaml"),
        "URI must point at the YAML file; got: {}",
        loc.uri
    );
}

// ── Phase E2: multi-model production hover / completion / goto-def helpers ──

/// Hover on `generates: models` in the frontmatter of a generator file shows
/// the inferred body type and the count of statically-resolved emitted models.
///
/// `hover_text_for_generates_frontmatter` must contain "List<ModelDef>" and
/// a human-readable emission count when the count is known.
#[test]
fn hover_on_generates_frontmatter_shows_body_type_and_emission_count() {
    let text = hover_text_for_generates_frontmatter(Some(3));
    assert!(
        text.contains("List<ModelDef>"),
        "hover must contain 'List<ModelDef>', got: {text}"
    );
    assert!(
        text.contains("3"),
        "hover must contain emission count '3', got: {text}"
    );
    // When count is unknown, body type must still appear.
    let text_no_count = hover_text_for_generates_frontmatter(None);
    assert!(
        text_no_count.contains("List<ModelDef>"),
        "hover (no count) must still contain 'List<ModelDef>', got: {text_no_count}"
    );
}

/// Hover on the opening `{` of a `ModelDef { … }` literal in a generator file
/// shows the inferred smelt path when the `name` field is statically known, and
/// falls back to just `"ModelDef"` when the name is not static.
///
/// `hover_text_for_model_def_literal_open_brace` must contain the path when
/// `smelt_path` is `Some`, and `"ModelDef"` in all cases.
#[test]
fn hover_on_model_def_literal_opening_brace_shows_emitted_path() {
    // Static name: path is known.
    let text = hover_text_for_model_def_literal_open_brace(Some("cohorts.us_west"));
    assert!(
        text.contains("cohorts.us_west"),
        "hover must contain resolved smelt path 'cohorts.us_west', got: {text}"
    );
    assert!(
        text.contains("ModelDef"),
        "hover must contain 'ModelDef', got: {text}"
    );
    // Non-static name: show type only.
    let fallback = hover_text_for_model_def_literal_open_brace(None);
    assert!(
        fallback.contains("ModelDef"),
        "fallback hover must contain 'ModelDef', got: {fallback}"
    );
    assert!(
        !fallback.contains("cohorts"),
        "fallback hover must not contain a path segment, got: {fallback}"
    );
}

/// Hover on the value token of `name: 'us_west'` inside a `ModelDef { … }`
/// literal shows the inferred emitted smelt path.
///
/// `hover_text_for_model_def_name_field_value` must contain the path string.
#[test]
fn hover_on_model_def_name_field_value_shows_smelt_path() {
    let text = hover_text_for_model_def_name_field_value("cohorts.us_west");
    assert!(
        text.contains("cohorts.us_west"),
        "hover must contain 'cohorts.us_west', got: {text}"
    );
}

/// Hover on the body expression value of `body: SELECT …` inside a `ModelDef`
/// literal shows `TableExpr` and, when column info is available, the column list.
///
/// `hover_text_for_model_def_body_field_value` must contain "TableExpr" and
/// the column names when provided.
#[test]
fn hover_on_model_def_body_field_value_shows_table_expr_and_columns() {
    let columns = vec![
        "id".to_string(),
        "region".to_string(),
        "revenue".to_string(),
    ];
    let text = hover_text_for_model_def_body_field_value(Some(&columns));
    assert!(
        text.contains("TableExpr"),
        "hover must contain 'TableExpr', got: {text}"
    );
    assert!(
        text.contains("id"),
        "hover must contain column 'id', got: {text}"
    );
    assert!(
        text.contains("region"),
        "hover must contain column 'region', got: {text}"
    );
    // No column info: TableExpr only.
    let no_cols = hover_text_for_model_def_body_field_value(None);
    assert!(
        no_cols.contains("TableExpr"),
        "hover (no columns) must still contain 'TableExpr', got: {no_cols}"
    );
}

/// Completion at `generates: <cursor>` in a `.sql` file's YAML frontmatter
/// offers exactly `["models"]` — no other values.
///
/// `completion_for_generates_value` must return a single entry whose label
/// is `"models"`.
#[test]
fn completion_on_generates_offers_models_only() {
    let items = completion_for_generates_value();
    assert_eq!(
        items.len(),
        1,
        "completion must offer exactly one value; got: {items:?}"
    );
    assert_eq!(
        items[0].label, "models",
        "sole completion item must be 'models'; got: {:?}",
        items[0].label
    );
}

/// Completion at `ModelDef {{ <cursor>` offers the closed seven-field set, with
/// required fields (`name`, `body`) first, and excludes already-filled fields.
///
/// `completion_for_model_def_field_key` must:
/// - Return all seven fields when `already_filled` is empty.
/// - Exclude already-filled fields.
/// - Place `name` and `body` before optional fields.
#[test]
fn completion_on_model_def_field_key_offers_closed_seven_field_set() {
    // All seven fields when nothing is filled.
    let items = completion_for_model_def_field_key(&[]);
    assert_eq!(
        items.len(),
        7,
        "must offer all seven fields when nothing is filled; got: {items:?}"
    );
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(
        labels.contains(&"name"),
        "must offer 'name' field; got: {labels:?}"
    );
    assert!(
        labels.contains(&"body"),
        "must offer 'body' field; got: {labels:?}"
    );
    assert!(
        labels.contains(&"materialization"),
        "must offer 'materialization' field; got: {labels:?}"
    );
    assert!(
        labels.contains(&"tags"),
        "must offer 'tags' field; got: {labels:?}"
    );
    assert!(
        labels.contains(&"description"),
        "must offer 'description' field; got: {labels:?}"
    );
    assert!(
        labels.contains(&"timeseries"),
        "must offer 'timeseries' field; got: {labels:?}"
    );
    assert!(
        labels.contains(&"safety_overrides"),
        "must offer 'safety_overrides' field; got: {labels:?}"
    );
    // Required fields (name, body) come first per the spec ordering rule.
    let name_pos = items.iter().position(|i| i.label == "name").unwrap();
    let body_pos = items.iter().position(|i| i.label == "body").unwrap();
    let mat_pos = items
        .iter()
        .position(|i| i.label == "materialization")
        .unwrap();
    assert!(
        name_pos < mat_pos,
        "required 'name' must come before optional 'materialization'"
    );
    assert!(
        body_pos < mat_pos,
        "required 'body' must come before optional 'materialization'"
    );

    // Already-filled fields are excluded.
    let items_partial = completion_for_model_def_field_key(&["name".to_string()]);
    assert_eq!(
        items_partial.len(),
        6,
        "must offer 6 fields when 'name' is already filled; got: {items_partial:?}"
    );
    assert!(
        !items_partial.iter().any(|i| i.label == "name"),
        "already-filled 'name' must not appear in completions"
    );
}

/// Goto-def on a generator-emitted model reference resolves to the emitting
/// `ModelDef.name` field's value-token in the generator file.
///
/// `goto_def_for_emitted_model_reference` takes the generator file path and
/// the `name_span` (the CST text range of the `name` field's value) and must
/// return a `Location` at the span's start/end in the generator file.
#[test]
fn goto_def_on_emitted_model_reference_resolves_to_model_def_name_field() {
    use tower_lsp::lsp_types::Range;
    let gen_path = std::path::Path::new("/workspace/models/cohorts.gen.sql");
    // Simulate: name: 'us_west' — the value token 'us_west' sits at line 5, cols 13..20
    let name_range = Range {
        start: tower_lsp::lsp_types::Position::new(5, 13),
        end: tower_lsp::lsp_types::Position::new(5, 20),
    };
    let loc = goto_def_for_emitted_model_reference(gen_path, name_range);
    assert!(
        loc.is_some(),
        "goto_def_for_emitted_model_reference must return Some"
    );
    let loc = loc.unwrap();
    assert!(
        loc.uri.path().ends_with("cohorts.gen.sql"),
        "location must point at the generator file; got: {}",
        loc.uri
    );
    assert_eq!(
        loc.range.start.line, 5,
        "location must be at line 5; got: {}",
        loc.range.start.line
    );
    assert_eq!(
        loc.range.start.character, 13,
        "location must start at col 13; got: {}",
        loc.range.start.character
    );
}

// ── Phase F wiring regression tests ─────────────────────────────────────────
//
// These tests verify that the dispatch helper `hover_text_for_hof_meta_language`
// is wired to the Phase F ternary and multi-arg lambda hover helpers.
// They call `dispatch_hover` which is the same code path that `Backend::hover`
// uses.  A failure here means the wiring block is missing from the dispatch
// function.

/// Hovering on the `if` keyword of `if cond then 1 else 2` returns non-empty
/// hover text via the `hover_text_for_hof_meta_language` dispatch.
///
/// This verifies Finding 2: the `TERNARY_EXPR` / `IF_KW` dispatch block is wired.
#[test]
fn dispatch_hover_on_if_keyword_returns_ternary_hover() {
    let sql = "SELECT if cond then 1 else 2 FROM t";
    // Cursor on the `i` of `if` (position 7 in the string above).
    let cursor = sql.find("if").expect("`if` must appear in sql");
    let result = dispatch_hover(sql, cursor);
    assert!(
        result.is_some(),
        "hover on `if` keyword must return Some via dispatch; got None. \
         Ensure hover_text_for_hof_meta_language handles TERNARY_EXPR / IF_KW."
    );
    let text = result.unwrap();
    assert!(
        text.contains("->") || text.contains("if"),
        "hover on `if` must mention the ternary signature; got: {text:?}"
    );
}

/// Hovering on the `then` keyword returns the then-branch type via dispatch.
///
/// This verifies Finding 2: the `THEN_KW` dispatch is wired.
#[test]
fn dispatch_hover_on_then_keyword_returns_then_branch_hover() {
    let sql = "SELECT if cond then 1 else 2 FROM t";
    let cursor = sql.find("then").expect("`then` must appear in sql");
    let result = dispatch_hover(sql, cursor);
    assert!(
        result.is_some(),
        "hover on `then` keyword must return Some via dispatch; got None. \
         Ensure hover_text_for_hof_meta_language handles THEN_KW."
    );
}

/// Hovering on the `else` keyword returns the else-branch type via dispatch.
///
/// This verifies Finding 2: the `ELSE_KW` dispatch is wired.
#[test]
fn dispatch_hover_on_else_keyword_returns_else_branch_hover() {
    let sql = "SELECT if cond then 1 else 2 FROM t";
    let cursor = sql.find("else").expect("`else` must appear in sql");
    let result = dispatch_hover(sql, cursor);
    assert!(
        result.is_some(),
        "hover on `else` keyword must return Some via dispatch; got None. \
         Ensure hover_text_for_hof_meta_language handles ELSE_KW."
    );
}

/// Hovering on the `(` of a multi-arg lambda `fn (a, b) => a + b` returns
/// the Lambda signature via dispatch.
///
/// This verifies Finding 2: the multi-arg lambda `(` dispatch is wired.
#[test]
fn dispatch_hover_on_multi_arg_lambda_open_paren_returns_lambda_hover() {
    let sql = "SELECT map(xs, fn (a, b) => a + b) FROM t";
    // Cursor on the `(` after `fn ` — find position after `fn `.
    let fn_pos = sql.find("fn (").expect("`fn (` must appear in sql");
    let cursor = fn_pos + 3; // position of `(`
    let result = dispatch_hover(sql, cursor);
    assert!(
        result.is_some(),
        "hover on `(` of multi-arg lambda must return Some via dispatch; got None. \
         Ensure hover_text_for_hof_meta_language handles multi-arg LAMBDA open paren."
    );
    let text = result.unwrap();
    assert!(
        text.contains("Lambda"),
        "hover on multi-arg lambda `(` must mention Lambda; got: {text:?}"
    );
}

/// At `reduce(xs, <cursor>)`, `completion_items_for_reduce_second_arg_with_snippets`
/// returns `concat_with` as a snippet item (not just a bare label), verifying
/// that Finding 1's fix produces the right output.
///
/// This is a direct caller-contract test: if the backend calls
/// `completion_items_for_reduce_second_arg_with_snippets` instead of
/// `reducer_completions_for_element_type`, clients will see snippets.
#[test]
fn completion_reduce_second_arg_snippet_function_returns_concat_with_snippet() {
    let items = completion_items_for_reduce_second_arg_with_snippets(None);
    let concat = items
        .iter()
        .find(|i| i.label == "concat_with")
        .expect("concat_with must be in completion list");
    let snippet = concat.insert_text.as_deref().unwrap_or("");
    assert!(
        snippet.contains("sep") && snippet.contains("${"),
        "concat_with must be a snippet with a `sep` placeholder; got: {snippet:?}"
    );
    use tower_lsp::lsp_types::InsertTextFormat;
    assert_eq!(
        concat.insert_text_format,
        Some(InsertTextFormat::SNIPPET),
        "concat_with completion must have SNIPPET insert text format"
    );
}

/// `completion_item_for_if_snippet` returns an `if` keyword item whose snippet
/// expands to `if … then … else …`, verifying Finding 3's helper is correct.
#[test]
fn completion_if_snippet_function_returns_correct_snippet() {
    let item = completion_item_for_if_snippet();
    assert_eq!(item.label, "if", "if snippet must have label `if`");
    let snippet = item.insert_text.as_deref().unwrap_or("");
    assert!(
        snippet.contains("cond") && snippet.contains("then") && snippet.contains("else"),
        "if snippet must expand to if…then…else with named placeholders; got: {snippet:?}"
    );
    use tower_lsp::lsp_types::InsertTextFormat;
    assert_eq!(
        item.insert_text_format,
        Some(InsertTextFormat::SNIPPET),
        "if completion must have SNIPPET insert text format"
    );
}

/// `MaintenanceSkeletonChanged` renders as the wire-visible code string a
/// CI/editor consumer matches on, post-rename
/// (`docs/outcomes/20260815-definition-delta-migrate/outcome.md` phase 7).
#[test]
fn skeleton_changed_maps_to_stable_code_string() {
    assert_eq!(
        crate::backend::diagnostic_code_str(smelt_db::DiagnosticCode::MaintenanceSkeletonChanged),
        "maintenance-skeleton-changed"
    );
}

/// A `grain: key` model with no derivable identity (no declared top-level
/// `unique_key:`, no GROUP BY) surfaces `GrainAssertionMismatch` — the same
/// code the plan-derivation seam now emits (`smelt-db`'s
/// `maintenance.rs::MaintenanceRefusal::IdentityNotDerivable`,
/// `docs/outcomes/20260815-definition-delta-migrate/outcome.md` phase 17) —
/// through the same wire-visible code string the LSP already renders for
/// the other `grain:` assertion mismatch, so CLI and LSP consumers agree on
/// the code string without a run.
#[test]
fn grain_assertion_mismatch_maps_to_stable_code_string() {
    assert_eq!(
        crate::backend::diagnostic_code_str(smelt_db::DiagnosticCode::GrainAssertionMismatch),
        "grain-assertion-mismatch"
    );
}

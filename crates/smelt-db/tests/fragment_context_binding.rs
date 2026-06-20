//! Phase 21 — Fragment context binding validation.
//!
//! Tests:
//!   1. `fragment_argument_column_valid_in_context` — `amount` in `amount > 100`
//!      is found in the inferred context → no `FragmentColumnMissing`.
//!   2. `fragment_argument_column_missing_from_context_errors` — `nonexistent`
//!      is not in the inferred context → `FragmentColumnMissing`.
//!   3. `explicit_annotation_wider_than_inference_errors` — `pred: Expr<Boolean,
//!      source>` but `pred` is only in HAVING (projected cols), so the full
//!      `source` annotation is wider → `AnnotationTooWide`.
//!   4. `explicit_annotation_narrower_than_inference_allowed` — annotation
//!      column set ⊆ inferred → no error.
//!   5. `agg_kind_context_binding_checks` — scalar arg for `SelectItems<Agg,
//!      source>` → `FragmentKindMismatch`; aggregate arg → no error.
//!   6. `scalar_splice_rejects_agg_fragment` — `SelectItems<Scalar>` rejects
//!      Agg and Window kind args (C17: a higher-kind fragment in a scalar-only
//!      splice point fires `FragmentKindMismatch`).
//!   7. `agg_splice_accepts_agg_and_window` — `SelectItems<Agg>` accepts both
//!      Agg and Window kind args (no false positive).
//!   8. `scalar_splice_accepts_scalar` — `SelectItems<Scalar>` accepts scalar
//!      args (no false positive).

use std::collections::HashMap;

use rowan::TextRange;
use smelt_db::{check_fragment_context_bindings, DiagnosticCode, TypeContext, TypedColumn};
use smelt_parser::ast::{Expr, SelectStmt};
use smelt_parser::File as AstFile;
use smelt_types::signatures::{extract_function_signatures, FunctionSig};
use smelt_types::DataType;

// ─── helpers ──────────────────────────────────────────────────────────────────

fn parse_define_select(text: &str) -> (FunctionSig, SelectStmt, String) {
    let clean = smelt_parser::strip_frontmatter(text).to_string();
    let parse = smelt_parser::parse(&clean);
    let ast = AstFile::cast(parse.syntax()).expect("valid AstFile");
    let sig = extract_function_signatures(&ast, &clean)
        .into_iter()
        .next()
        .expect("at least one define");
    let define = ast.defines().next().expect("one SmeltDefine");
    let select = define
        .body()
        .expect("body")
        .select_stmt()
        .expect("SELECT body");
    (sig, select, clean)
}

/// Parse a standalone expression from a dummy define body.
/// Returns `(Expr, TextRange, full_dummy_text)`.
fn parse_arg_expr(expr_text: &str) -> (Expr, TextRange, String) {
    let full = format!("smelt.define _dummy() AS ({expr_text})\n");
    let clean = smelt_parser::strip_frontmatter(&full).to_string();
    let parse = smelt_parser::parse(&clean);
    let ast = AstFile::cast(parse.syntax()).expect("valid AstFile");
    let define = ast.defines().next().expect("one define");
    let body_expr = define
        .body()
        .expect("body")
        .expression()
        .expect("expression body");
    let range = body_expr.text_range();
    (body_expr, range, clean)
}

// ─── Test 1 ───────────────────────────────────────────────────────────────────

#[test]
fn fragment_argument_column_valid_in_context() {
    let (sig, select, _) = parse_define_select(
        "smelt.define filter_fn(\
           source: TableExpr, \
           filters: Expr<Boolean> \
         ) -> TableExpr AS (\
           SELECT * FROM source WHERE filters\
         )\n",
    );

    let mut body_ctx = TypeContext::new();
    body_ctx.add_tableexpr_param(
        "source",
        &[
            ("id".to_string(), TypedColumn::nullable(DataType::Integer)),
            (
                "amount".to_string(),
                TypedColumn::nullable(DataType::Double),
            ),
            ("date".to_string(), TypedColumn::nullable(DataType::Text)),
        ],
    );
    body_ctx.add_function_param("filters", TypedColumn::nullable(DataType::Boolean));

    let (arg_expr, arg_range, _arg_text) = parse_arg_expr("amount > 100");
    let bindings: HashMap<String, (Expr, TextRange)> =
        HashMap::from([("filters".to_string(), (arg_expr, arg_range))]);

    let diags = check_fragment_context_bindings(&sig, &select, &body_ctx, &body_ctx, &bindings);
    let missing: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::FragmentColumnMissing))
        .collect();
    assert!(
        missing.is_empty(),
        "valid column `amount` should not emit FragmentColumnMissing; got {diags:#?}"
    );
}

// ─── Test 2 ───────────────────────────────────────────────────────────────────

#[test]
fn fragment_argument_column_missing_from_context_errors() {
    let (sig, select, _) = parse_define_select(
        "smelt.define filter_fn(\
           source: TableExpr, \
           filters: Expr<Boolean> \
         ) -> TableExpr AS (\
           SELECT * FROM source WHERE filters\
         )\n",
    );

    let mut body_ctx = TypeContext::new();
    body_ctx.add_tableexpr_param(
        "source",
        &[
            ("id".to_string(), TypedColumn::nullable(DataType::Integer)),
            (
                "amount".to_string(),
                TypedColumn::nullable(DataType::Double),
            ),
            ("date".to_string(), TypedColumn::nullable(DataType::Text)),
        ],
    );
    body_ctx.add_function_param("filters", TypedColumn::nullable(DataType::Boolean));

    let (arg_expr, arg_range, _arg_text) = parse_arg_expr("nonexistent > 100");
    let bindings: HashMap<String, (Expr, TextRange)> =
        HashMap::from([("filters".to_string(), (arg_expr, arg_range))]);

    let diags = check_fragment_context_bindings(&sig, &select, &body_ctx, &body_ctx, &bindings);
    let missing: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::FragmentColumnMissing))
        .collect();
    assert!(
        !missing.is_empty(),
        "unknown column `nonexistent` should emit FragmentColumnMissing; got {diags:#?}"
    );
    assert!(
        missing[0].message.contains("nonexistent"),
        "diagnostic message should mention the column; got {:?}",
        missing[0].message
    );
}

// ─── Test 3 ───────────────────────────────────────────────────────────────────

#[test]
fn explicit_annotation_wider_than_inference_errors() {
    // pred: Expr<Boolean, source> but pred is only in HAVING.
    // HAVING scope = projected SELECT cols = {id}.
    // Annotation claims full source = {id, name, amount} → wider → AnnotationTooWide.
    let (sig, select, _) = parse_define_select(
        "smelt.define anno_fn(\
           source: TableExpr, \
           pred: Expr<Boolean, source> \
         ) -> TableExpr AS (\
           SELECT id FROM source GROUP BY id HAVING pred\
         )\n",
    );

    let mut body_ctx = TypeContext::new();
    body_ctx.add_tableexpr_param(
        "source",
        &[
            ("id".to_string(), TypedColumn::nullable(DataType::Integer)),
            ("name".to_string(), TypedColumn::nullable(DataType::Text)),
            (
                "amount".to_string(),
                TypedColumn::nullable(DataType::Double),
            ),
        ],
    );
    body_ctx.add_function_param("pred", TypedColumn::nullable(DataType::Boolean));

    let (arg_expr, arg_range, _arg_text) = parse_arg_expr("id > 0");
    let bindings: HashMap<String, (Expr, TextRange)> =
        HashMap::from([("pred".to_string(), (arg_expr, arg_range))]);

    let diags = check_fragment_context_bindings(&sig, &select, &body_ctx, &body_ctx, &bindings);
    let too_wide: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::AnnotationTooWide))
        .collect();
    assert!(
        !too_wide.is_empty(),
        "annotation with extra columns should emit AnnotationTooWide; got {diags:#?}"
    );
    let msg = &too_wide[0].message;
    assert!(
        msg.contains("name") || msg.contains("amount"),
        "message should name the extra columns; got {msg:?}"
    );
}

// ─── Test 4 ───────────────────────────────────────────────────────────────────

#[test]
fn explicit_annotation_narrower_than_inference_allowed() {
    // narrow_fn(source: TableExpr, narrow: TableExpr, pred: Expr<Boolean, narrow>)
    // body: SELECT * FROM source WHERE pred
    // Inferred context for pred (WHERE scope) = source = {id, name, amount}.
    // Annotation `narrow` = {id} — subset of inferred → no AnnotationTooWide.
    let (sig, select, _) = parse_define_select(
        "smelt.define narrow_fn(\
           source: TableExpr, \
           narrow: TableExpr, \
           pred: Expr<Boolean, narrow> \
         ) -> TableExpr AS (\
           SELECT * FROM source WHERE pred\
         )\n",
    );

    let mut body_ctx = TypeContext::new();
    body_ctx.add_tableexpr_param(
        "source",
        &[
            ("id".to_string(), TypedColumn::nullable(DataType::Integer)),
            ("name".to_string(), TypedColumn::nullable(DataType::Text)),
            (
                "amount".to_string(),
                TypedColumn::nullable(DataType::Double),
            ),
        ],
    );
    body_ctx.add_tableexpr_param(
        "narrow",
        &[("id".to_string(), TypedColumn::nullable(DataType::Integer))],
    );
    body_ctx.add_function_param("pred", TypedColumn::nullable(DataType::Boolean));

    let (arg_expr, arg_range, _arg_text) = parse_arg_expr("id > 0");
    let bindings: HashMap<String, (Expr, TextRange)> =
        HashMap::from([("pred".to_string(), (arg_expr, arg_range))]);

    let diags = check_fragment_context_bindings(&sig, &select, &body_ctx, &body_ctx, &bindings);
    let too_wide: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::AnnotationTooWide))
        .collect();
    assert!(
        too_wide.is_empty(),
        "narrower annotation should NOT emit AnnotationTooWide; got {diags:#?}"
    );
}

// ─── Test 5 ───────────────────────────────────────────────────────────────────

#[test]
fn agg_kind_context_binding_checks() {
    let (sig, select, _) = parse_define_select(
        "smelt.define agg_fn(\
           source: TableExpr, \
           metrics: SelectItems<Agg, source> \
         ) -> TableExpr AS (\
           SELECT id, metrics FROM source GROUP BY id\
         )\n",
    );

    let mut body_ctx = TypeContext::new();
    body_ctx.add_tableexpr_param(
        "source",
        &[
            ("id".to_string(), TypedColumn::nullable(DataType::Integer)),
            (
                "revenue".to_string(),
                TypedColumn::nullable(DataType::Double),
            ),
        ],
    );
    body_ctx.add_function_param(
        "metrics",
        TypedColumn::nullable(DataType::unknown_dynamic()),
    );

    // Negative: scalar arg → FragmentKindMismatch.
    {
        let (arg_expr, arg_range, _arg_text) = parse_arg_expr("revenue + 1");
        let bindings: HashMap<String, (Expr, TextRange)> =
            HashMap::from([("metrics".to_string(), (arg_expr, arg_range))]);
        let diags = check_fragment_context_bindings(&sig, &select, &body_ctx, &body_ctx, &bindings);
        let kind_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::FragmentKindMismatch))
            .collect();
        assert!(
            !kind_diags.is_empty(),
            "scalar arg for SelectItems<Agg> should emit FragmentKindMismatch; got {diags:#?}"
        );
    }

    // Positive: aggregate arg → no FragmentKindMismatch.
    {
        let (arg_expr, arg_range, _arg_text) = parse_arg_expr("SUM(revenue)");
        let bindings: HashMap<String, (Expr, TextRange)> =
            HashMap::from([("metrics".to_string(), (arg_expr, arg_range))]);
        let diags = check_fragment_context_bindings(&sig, &select, &body_ctx, &body_ctx, &bindings);
        let kind_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::FragmentKindMismatch))
            .collect();
        assert!(
            kind_diags.is_empty(),
            "aggregate arg for SelectItems<Agg> should NOT emit FragmentKindMismatch; got {diags:#?}"
        );
    }
}

// ─── Test 6 ───────────────────────────────────────────────────────────────────

/// C17: a Scalar-only splice point must reject Agg and Window kind fragments.
#[test]
fn scalar_splice_rejects_agg_fragment() {
    let (sig, select, _) = parse_define_select(
        "smelt.define scalar_fn(\
           source: TableExpr, \
           cols: SelectItems<Scalar, source> \
         ) -> TableExpr AS (\
           SELECT cols FROM source\
         )\n",
    );

    let mut body_ctx = TypeContext::new();
    body_ctx.add_tableexpr_param(
        "source",
        &[
            ("id".to_string(), TypedColumn::nullable(DataType::Integer)),
            (
                "revenue".to_string(),
                TypedColumn::nullable(DataType::Double),
            ),
        ],
    );
    body_ctx.add_function_param("cols", TypedColumn::nullable(DataType::unknown_dynamic()));

    // Agg arg into Scalar-only splice point → FragmentKindMismatch.
    {
        let (arg_expr, arg_range, _) = parse_arg_expr("SUM(revenue)");
        let bindings: HashMap<String, (Expr, TextRange)> =
            HashMap::from([("cols".to_string(), (arg_expr, arg_range))]);
        let diags = check_fragment_context_bindings(&sig, &select, &body_ctx, &body_ctx, &bindings);
        let kind_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::FragmentKindMismatch))
            .collect();
        assert!(
            !kind_diags.is_empty(),
            "agg arg for SelectItems<Scalar> should emit FragmentKindMismatch; got {diags:#?}"
        );
    }

    // Window arg into Scalar-only splice point → FragmentKindMismatch.
    {
        let (arg_expr, arg_range, _) = parse_arg_expr("ROW_NUMBER() OVER ()");
        let bindings: HashMap<String, (Expr, TextRange)> =
            HashMap::from([("cols".to_string(), (arg_expr, arg_range))]);
        let diags = check_fragment_context_bindings(&sig, &select, &body_ctx, &body_ctx, &bindings);
        let kind_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::FragmentKindMismatch))
            .collect();
        assert!(
            !kind_diags.is_empty(),
            "window arg for SelectItems<Scalar> should emit FragmentKindMismatch; got {diags:#?}"
        );
    }
}

// ─── Test 7 ───────────────────────────────────────────────────────────────────

/// C17: Agg splice point accepts both Agg and Window kind args (no false positive).
#[test]
fn agg_splice_accepts_agg_and_window() {
    let (sig, select, _) = parse_define_select(
        "smelt.define agg_fn2(\
           source: TableExpr, \
           metrics: SelectItems<Agg, source> \
         ) -> TableExpr AS (\
           SELECT id, metrics FROM source GROUP BY id\
         )\n",
    );

    let mut body_ctx = TypeContext::new();
    body_ctx.add_tableexpr_param(
        "source",
        &[
            ("id".to_string(), TypedColumn::nullable(DataType::Integer)),
            (
                "revenue".to_string(),
                TypedColumn::nullable(DataType::Double),
            ),
        ],
    );
    body_ctx.add_function_param(
        "metrics",
        TypedColumn::nullable(DataType::unknown_dynamic()),
    );

    // Agg arg → no FragmentKindMismatch.
    {
        let (arg_expr, arg_range, _) = parse_arg_expr("SUM(revenue)");
        let bindings: HashMap<String, (Expr, TextRange)> =
            HashMap::from([("metrics".to_string(), (arg_expr, arg_range))]);
        let diags = check_fragment_context_bindings(&sig, &select, &body_ctx, &body_ctx, &bindings);
        let kind_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::FragmentKindMismatch))
            .collect();
        assert!(
            kind_diags.is_empty(),
            "agg arg for SelectItems<Agg> must NOT emit FragmentKindMismatch; got {diags:#?}"
        );
    }

    // Window arg → no FragmentKindMismatch (Agg splice accepts Window).
    {
        let (arg_expr, arg_range, _) = parse_arg_expr("ROW_NUMBER() OVER ()");
        let bindings: HashMap<String, (Expr, TextRange)> =
            HashMap::from([("metrics".to_string(), (arg_expr, arg_range))]);
        let diags = check_fragment_context_bindings(&sig, &select, &body_ctx, &body_ctx, &bindings);
        let kind_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(DiagnosticCode::FragmentKindMismatch))
            .collect();
        assert!(
            kind_diags.is_empty(),
            "window arg for SelectItems<Agg> must NOT emit FragmentKindMismatch; got {diags:#?}"
        );
    }
}

// ─── Test 8 ───────────────────────────────────────────────────────────────────

/// C17: Scalar splice point accepts scalar args (no false positive).
#[test]
fn scalar_splice_accepts_scalar() {
    let (sig, select, _) = parse_define_select(
        "smelt.define scalar_fn2(\
           source: TableExpr, \
           cols: SelectItems<Scalar, source> \
         ) -> TableExpr AS (\
           SELECT cols FROM source\
         )\n",
    );

    let mut body_ctx = TypeContext::new();
    body_ctx.add_tableexpr_param(
        "source",
        &[
            ("id".to_string(), TypedColumn::nullable(DataType::Integer)),
            (
                "revenue".to_string(),
                TypedColumn::nullable(DataType::Double),
            ),
        ],
    );
    body_ctx.add_function_param("cols", TypedColumn::nullable(DataType::unknown_dynamic()));

    // Scalar arg → no FragmentKindMismatch.
    let (arg_expr, arg_range, _) = parse_arg_expr("revenue + 1");
    let bindings: HashMap<String, (Expr, TextRange)> =
        HashMap::from([("cols".to_string(), (arg_expr, arg_range))]);
    let diags = check_fragment_context_bindings(&sig, &select, &body_ctx, &body_ctx, &bindings);
    let kind_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::FragmentKindMismatch))
        .collect();
    assert!(
        kind_diags.is_empty(),
        "scalar arg for SelectItems<Scalar> must NOT emit FragmentKindMismatch; got {diags:#?}"
    );
}

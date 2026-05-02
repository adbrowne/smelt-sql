//! Phase 45 (smelt-functions) — JOIN-alias schema visibility in
//! `TableExpr` parameter bodies.
//!
//! A function whose body LEFT JOINs `smelt.models.Y AS y` must see
//! `y`'s columns through the body's bare-column resolver. Closes
//! review findings #10 and #21.
//!
//! Tests:
//!   1. `joined_alias_columns_visible_in_body` — bare `y.col_name`
//!      reference inside the body resolves to the joined model's
//!      schema.
//!   2. `joined_alias_shadow_warning` — when a function declares an
//!      `Expr<Text>` parameter named after a column on the JOIN-aliased
//!      schema, exactly one ParameterShadowsColumn warning fires.
//!   3. `joined_alias_missing_column_errors` — the body references an
//!      identifier that is neither on the FROM-target nor any
//!      JOIN-aliased schema; surfaces as `UnknownIdentifier`.
//!   4. `joined_alias_in_select_star_expansion` — `SELECT y.*` from
//!      the body propagates the joined schema into the call's inferred
//!      return schema, so the caller's bare reference to one of the
//!      joined columns resolves cleanly.

use std::path::PathBuf;

use smelt_db::{
    check_type_diagnostics, file_diagnostics, Database, DiagnosticAcc, DiagnosticCode,
    DiagnosticSeverity, SourceFile, Workspace,
};

fn build_db(
    project_root: PathBuf,
    sources_yaml: &str,
    files: &[(PathBuf, &str)],
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), sources_yaml.to_string());

    let mut handles = Vec::with_capacity(files.len());
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), (*content).to_string(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

fn orders_model_sql(columns: &[(&str, &str)]) -> String {
    let projections = columns
        .iter()
        .map(|(name, ty)| format!("CAST(NULL AS {ty}) AS {name}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("SELECT {projections}\n")
}

#[test]
fn joined_alias_columns_visible_in_body() {
    // TDD test 1: a function with a TableExpr `orders` parameter
    // LEFT JOINs `smelt.models.dim_customer AS dim_customer`. The
    // body reference `dim_customer.customer_name` must resolve via
    // the JOIN-aliased schema.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("enrich.sql");
    let orders_path = root.join("models").join("orders.sql");
    let dim_path = root.join("models").join("dim_customer.sql");
    let model_path = root.join("models").join("uses_enrich.sql");
    // Phase 4: use path form (smelt.ref() is removed).
    let fn_src = "smelt.define enrich(orders: TableExpr) -> TableExpr AS (\
        SELECT orders.order_id, dim_customer.customer_name \
        FROM orders \
        LEFT JOIN smelt.models.dim_customer AS dim_customer \
          ON orders.customer_id = dim_customer.customer_id\
    )\n";
    let model_src = "SELECT * FROM smelt.functions.enrich(smelt.models.orders)\n";

    let orders_sql = orders_model_sql(&[
        ("order_id", "BIGINT"),
        ("customer_id", "BIGINT"),
        ("total", "DECIMAL(18,2)"),
    ]);
    let dim_sql = orders_model_sql(&[("customer_id", "BIGINT"), ("customer_name", "VARCHAR")]);

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (fn_path, fn_src),
            (orders_path, orders_sql.as_str()),
            (dim_path, dim_sql.as_str()),
            (model_path.clone(), model_src),
        ],
    );
    let model_file = files[3];

    let diags = file_diagnostics(&db, ws, model_file);
    let bad: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .collect();
    assert!(
        bad.is_empty(),
        "expected no errors when body references JOIN-aliased columns; got {bad:#?}"
    );
}

#[test]
fn joined_alias_shadow_warning() {
    // TDD test 2: function declares an `Expr<Text>` parameter
    // named `customer_name`; the joined `dim_customer` schema also
    // has a `customer_name` column. Phase 15's
    // `ParameterShadowsColumn` warning must fire exactly once.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("shadow_join.sql");
    let orders_path = root.join("models").join("orders.sql");
    let dim_path = root.join("models").join("dim_customer.sql");
    let model_path = root.join("models").join("uses_shadow_join.sql");
    // Phase 4: use path form (smelt.ref() is removed).
    let fn_src =
        "smelt.define shadow_join(customer_name: Expr<Text>, orders: TableExpr) -> TableExpr AS (\
        SELECT orders.order_id, customer_name AS computed_name \
        FROM orders \
        LEFT JOIN smelt.models.dim_customer AS dim_customer \
          ON orders.customer_id = dim_customer.customer_id\
    )\n";
    let model_src = "SELECT * FROM smelt.functions.shadow_join('hi', smelt.models.orders)\n";

    let orders_sql = orders_model_sql(&[("order_id", "BIGINT"), ("customer_id", "BIGINT")]);
    let dim_sql = orders_model_sql(&[("customer_id", "BIGINT"), ("customer_name", "VARCHAR")]);

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (fn_path, fn_src),
            (orders_path, orders_sql.as_str()),
            (dim_path, dim_sql.as_str()),
            (model_path.clone(), model_src),
        ],
    );
    let model_file = files[3];

    let diags = file_diagnostics(&db, ws, model_file);
    let shadows: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ParameterShadowsColumn))
        .collect();
    assert_eq!(
        shadows.len(),
        1,
        "expected exactly one ParameterShadowsColumn warning, got {diags:#?}"
    );
    assert!(
        shadows[0].message.contains("customer_name"),
        "shadow message must mention `customer_name`; got: {}",
        shadows[0].message
    );
}

#[test]
fn joined_alias_missing_column_errors() {
    // TDD test 3: body references a column that is not on the
    // JOIN-aliased schema. Surfaces as `UnknownIdentifier` (the
    // body checker can't find `does_not_exist` on `dim_customer`).
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("missing.sql");
    let orders_path = root.join("models").join("orders.sql");
    let dim_path = root.join("models").join("dim_customer.sql");
    let model_path = root.join("models").join("uses_missing.sql");
    // Phase 4: use path form (smelt.ref() is removed).
    let fn_src = "smelt.define missing(orders: TableExpr) -> TableExpr AS (\
        SELECT orders.order_id, dim_customer.does_not_exist \
        FROM orders \
        LEFT JOIN smelt.models.dim_customer AS dim_customer \
          ON orders.customer_id = dim_customer.customer_id\
    )\n";
    let model_src = "SELECT * FROM smelt.functions.missing(smelt.models.orders)\n";

    let orders_sql = orders_model_sql(&[("order_id", "BIGINT"), ("customer_id", "BIGINT")]);
    let dim_sql = orders_model_sql(&[("customer_id", "BIGINT"), ("customer_name", "VARCHAR")]);

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (fn_path, fn_src),
            (orders_path, orders_sql.as_str()),
            (dim_path, dim_sql.as_str()),
            (model_path.clone(), model_src),
        ],
    );
    let model_file = files[3];

    let diags = file_diagnostics(&db, ws, model_file);
    let unknowns: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code == Some(DiagnosticCode::UnknownIdentifier)
                && d.message.contains("does_not_exist")
        })
        .collect();
    assert!(
        !unknowns.is_empty(),
        "expected an UnknownIdentifier(does_not_exist) diagnostic, got {diags:#?}"
    );
}

#[test]
fn joined_alias_in_select_star_expansion() {
    // TDD test 4: function body is `SELECT orders.*, dim_customer.*`
    // — the inferred return schema must include the joined
    // schema's columns so the caller's bare reference resolves.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("g.sql");
    let orders_path = root.join("models").join("orders.sql");
    let dim_path = root.join("models").join("dim_customer.sql");
    let model_path = root.join("models").join("uses_g.sql");
    // Phase 4: use path form (smelt.ref() is removed).
    let fn_src = "smelt.define g(orders: TableExpr) -> TableExpr AS (\
        SELECT orders.*, dim_customer.* \
        FROM orders \
        LEFT JOIN smelt.models.dim_customer AS dim_customer \
          ON orders.customer_id = dim_customer.customer_id\
    )\n";
    let model_src = "SELECT customer_name FROM smelt.functions.g(smelt.models.orders)\n";

    let orders_sql = orders_model_sql(&[("order_id", "BIGINT"), ("customer_id", "BIGINT")]);
    let dim_sql = orders_model_sql(&[("customer_id", "BIGINT"), ("customer_name", "VARCHAR")]);

    let (db, ws, files) = build_db(
        root,
        "version: 1\nsources: []\n",
        &[
            (fn_path, fn_src),
            (orders_path, orders_sql.as_str()),
            (dim_path, dim_sql.as_str()),
            (model_path.clone(), model_src),
        ],
    );
    let model_file = files[3];

    let file_diags = file_diagnostics(&db, ws, model_file);
    let type_diags: Vec<_> =
        check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, model_file)
            .into_iter()
            .map(|d| d.0.clone())
            .collect();
    let all_diags: Vec<_> = file_diags.into_iter().chain(type_diags).collect();
    let bad: Vec<_> = all_diags
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .filter(|d| {
            // Restrict to identifier-resolution diagnostics so unrelated
            // type checks don't flake this test.
            matches!(
                d.code,
                Some(DiagnosticCode::UnknownIdentifier) | Some(DiagnosticCode::UndeclaredColumn)
            )
        })
        .collect();
    assert!(
        bad.is_empty(),
        "expected `customer_name` to resolve through `g`'s inferred return schema; got {bad:#?}"
    );
}

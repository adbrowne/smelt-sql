//! Phase 11 (smelt-functions) — per-declaration frontmatter +
//! `backends:` inference + backend-namespace sugar.
//!
//! TDD coverage per the plan `20260422-smelt-functions-steps-1-2.md`:
//!   1. `frontmatter_attaches_to_next_decl` — parser-level (see
//!      smelt-parser tests; this crate exercises the DB-level assertion
//!      that `file_signature_inputs` picks up per-decl frontmatter).
//!   2. `backends_inferred_from_calls` — canonical body → `all`; body
//!      with `duckdb.read_parquet` → `[duckdb]`.
//!   3. `declared_backends_narrows`.
//!   4. `declared_backends_widening_is_error`.
//!   5. `duckdb_namespace_sugar_equivalent_to_frontmatter`.
//!   6. `old_file_level_frontmatter_on_lone_model_still_works`.

use std::path::PathBuf;

use smelt_db::{
    file_diagnostics, file_signature_inputs, function_backends, Database, DiagnosticCode,
    SourceFile, Workspace,
};
use smelt_types::BackendSet;

fn build_db(
    project_root: PathBuf,
    files: &[(PathBuf, &str)],
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());
    let mut handles = Vec::with_capacity(files.len());
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), (*content).to_string(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

#[test]
fn backends_inferred_from_calls_all_sql() {
    // TDD #2a: canonical body using only generic SQL (CAST, division,
    // CASE) infers to `All` — no backend-specific calls.
    let root = PathBuf::from("/fake/project");
    let src = "smelt.define safe_divide(numerator: Expr<Numeric>, denominator: Expr<Numeric>) \
        -> Expr<Double> AS (CAST(numerator AS DOUBLE) / denominator)\n";
    let path = root.join("functions").join("safe_divide.sql");
    let (db, ws, files) = build_db(root, &[(path, src)]);
    let set = function_backends(&db, ws, files[0], "safe_divide".to_string()).expect("sig exists");
    assert_eq!(set, BackendSet::All, "expected `all`, got {:?}", set);
}

#[test]
fn backends_inferred_from_calls_duckdb() {
    // TDD #2b: body contains a `duckdb.read_parquet(...)` call. The
    // inferred backend set is narrowed to `[duckdb]`.
    let root = PathBuf::from("/fake/project");
    let src = "smelt.define load(path: Expr<Text>) -> Expr<Text> \
        AS (duckdb.read_parquet(path))\n";
    let path = root.join("functions").join("load.sql");
    let (db, ws, files) = build_db(root, &[(path, src)]);
    let set = function_backends(&db, ws, files[0], "load".to_string()).expect("sig exists");
    assert_eq!(set, BackendSet::from_names(["duckdb"]));
}

#[test]
fn declared_backends_narrows() {
    // TDD #3: declared `backends: [duckdb]` on a body that infers to
    // `All` is an accepted narrowing — the query returns `[duckdb]`
    // and no diagnostic fires.
    let root = PathBuf::from("/fake/project");
    let src = "---\nbackends: [duckdb]\n---\nsmelt.define safe_divide(numerator: Expr<Numeric>, \
        denominator: Expr<Numeric>) -> Expr<Double> \
        AS (CAST(numerator AS DOUBLE) / denominator)\n";
    let path = root.join("functions").join("safe_divide.sql");
    let (db, ws, files) = build_db(root, &[(path, src)]);

    // Sanity: signature carries the declared set.
    let sigs = file_signature_inputs(&db, files[0]);
    assert_eq!(
        sigs[0].declared_backends,
        Some(BackendSet::from_names(["duckdb"])),
        "declared_backends should be parsed from frontmatter"
    );

    let set = function_backends(&db, ws, files[0], "safe_divide".to_string()).unwrap();
    assert_eq!(set, BackendSet::from_names(["duckdb"]));

    let diags: Vec<_> = file_diagnostics(&db, ws, files[0])
        .into_iter()
        .filter(|d| d.code == Some(DiagnosticCode::BackendsWideningNotAllowed))
        .collect();
    assert!(
        diags.is_empty(),
        "narrowing should not produce a widening diagnostic, got {:?}",
        diags
    );
}

#[test]
fn declared_backends_widening_is_error() {
    // TDD #4: body uses `duckdb.read_parquet` (inferred [duckdb]);
    // frontmatter declares `[duckdb, spark]` — widening. The
    // `BackendsWideningNotAllowed` diagnostic must fire, anchored at
    // the declaration's name.
    let root = PathBuf::from("/fake/project");
    let src = "---\nbackends: [duckdb, spark]\n---\nsmelt.define load(path: Expr<Text>) \
        -> Expr<Text> AS (duckdb.read_parquet(path))\n";
    let path = root.join("functions").join("load.sql");
    let (db, ws, files) = build_db(root, &[(path, src)]);

    let diags: Vec<_> = file_diagnostics(&db, ws, files[0])
        .into_iter()
        .filter(|d| d.code == Some(DiagnosticCode::BackendsWideningNotAllowed))
        .collect();
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one widening diagnostic, got {:#?}",
        diags
    );
    assert!(
        diags[0].message.contains("widen"),
        "diagnostic message should mention widening, got {:?}",
        diags[0].message
    );
}

#[test]
fn duckdb_namespace_sugar_equivalent_to_frontmatter() {
    // TDD #5: `smelt.extern duckdb.read_parquet(...)` must be
    // equivalent to the explicit
    // `---\nbackends: [duckdb]\n---\nsmelt.extern read_parquet(...)` form.
    // Both paths produce a signature with `declared_backends = [duckdb]`.
    let root = PathBuf::from("/fake/project");

    // Sugar form.
    let sugar_src = "smelt.extern duckdb.read_parquet(path: Expr<Text>) -> Expr<Text>\n";
    let sugar_path = root.join("functions").join("sugar.sql");
    let (db_a, _ws_a, files_a) = build_db(root.clone(), &[(sugar_path, sugar_src)]);
    let sig_a = file_signature_inputs(&db_a, files_a[0])
        .iter()
        .find(|s| s.name == "read_parquet")
        .cloned()
        .expect("sugar extern should exist");

    // Explicit frontmatter form — still a singleton extern, renamed to
    // avoid collision with the sugar form.
    let explicit_src =
        "---\nbackends: [duckdb]\n---\nsmelt.extern read_parquet(path: Expr<Text>) -> Expr<Text>\n";
    let explicit_path = root.join("functions").join("explicit.sql");
    let (db_b, _ws_b, files_b) = build_db(root, &[(explicit_path, explicit_src)]);
    let sig_b = file_signature_inputs(&db_b, files_b[0])
        .iter()
        .find(|s| s.name == "read_parquet")
        .cloned()
        .expect("explicit extern should exist");

    assert_eq!(
        sig_a.declared_backends,
        Some(BackendSet::from_names(["duckdb"])),
        "sugar form should declare [duckdb]",
    );
    assert_eq!(
        sig_b.declared_backends,
        Some(BackendSet::from_names(["duckdb"])),
        "explicit form should declare [duckdb]",
    );
    assert_eq!(
        sig_a.declared_backends, sig_b.declared_backends,
        "sugar and explicit forms must agree on declared_backends",
    );
}

#[test]
fn old_file_level_frontmatter_on_lone_model_still_works() {
    // TDD #6: load an existing single-block model fixture and assert
    // no new diagnostics fire. We test a minimal single-block model
    // verbatim here (the full file-level fixtures are exercised by
    // `example_diagnostics::timeseries_no_diagnostics`). The key
    // property: with zero `smelt.define`s in the file, frontmatter
    // attachment yields no signatures and no widening diagnostics.
    let root = PathBuf::from("/fake/project");
    // Phase 4: use path form (smelt.source() is removed).
    let src = "---\nmaterialization: table\nincremental:\n  partition_column: event_date\n---\n\
        SELECT event_id FROM smelt.sources.raw.events\n";
    let path = root.join("models").join("daily_events.sql");
    let (db, ws, files) = build_db(root, &[(path, src)]);

    let diags: Vec<_> = file_diagnostics(&db, ws, files[0])
        .into_iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::BackendsWideningNotAllowed) | Some(DiagnosticCode::ParseError)
            )
        })
        .collect();
    // Allow "UndefinedSource" since `raw.events` isn't in the fixture
    // workspace — but NO backends-widening or parse errors must fire.
    assert!(
        diags.is_empty(),
        "lone model with single-block frontmatter should not produce \
         backends diagnostics, got {:#?}",
        diags
    );
}

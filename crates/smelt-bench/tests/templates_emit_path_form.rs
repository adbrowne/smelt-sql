//! Phase 2c — bench template tests: verify templates emit path syntax.
//!
//! Test 8: `sql_templates_emit_path_syntax`
//!   - `generate_sql` must NOT produce `smelt.ref(` in its output.
//!   - `generate_sql` MUST produce `smelt.models.` in its output.
//!
//! Similarly for Python templates:
//!   - `generate_python` must NOT produce `smelt.ref(` in any emitted string.

use rand::SeedableRng;
use rand_chacha::ChaChaRng;

use smelt_bench::model_gen::graph_builder::{ModelSpec, ModelType};
use smelt_bench::model_gen::python_templates::generate_python;
use smelt_bench::model_gen::sql_templates::generate_sql;

// ---------------------------------------------------------------------------
// Test 8a: SQL templates emit path syntax
// ---------------------------------------------------------------------------

#[test]
fn sql_templates_emit_path_syntax() {
    let mut rng = ChaChaRng::seed_from_u64(42);

    for i in 0..12 {
        let spec = ModelSpec {
            name: format!("test_model_{}", i),
            layer: 2,
            dependencies: vec!["dep_a".into(), "dep_b".into(), "dep_c".into()],
            model_type: ModelType::Sql,
        };

        let sql = generate_sql(&mut rng, &spec);

        assert!(
            !sql.contains("smelt.ref("),
            "Template {i} must NOT contain 'smelt.ref(' (legacy form); got:\n{sql}"
        );
        assert!(
            sql.contains("smelt.models."),
            "Template {i} must contain 'smelt.models.' (path form); got:\n{sql}"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 8b: SQL templates produce valid SQL after migration
// ---------------------------------------------------------------------------

#[test]
fn sql_templates_still_parse_after_path_migration() {
    let mut rng = ChaChaRng::seed_from_u64(99);

    for i in 0..12 {
        let spec = ModelSpec {
            name: format!("parse_model_{}", i),
            layer: 2,
            dependencies: vec!["dep_a".into(), "dep_b".into(), "dep_c".into()],
            model_type: ModelType::Sql,
        };

        let sql = generate_sql(&mut rng, &spec);
        let clean = smelt_parser::strip_frontmatter(&sql);
        let parse = smelt_parser::parse(&clean);

        assert!(
            parse.errors.is_empty(),
            "Template {i} must parse cleanly after path migration; errors: {:?}\nSQL:\n{}",
            parse.errors,
            sql
        );
    }
}

// ---------------------------------------------------------------------------
// Test 8c: Python templates emit path syntax (no legacy smelt.ref)
// ---------------------------------------------------------------------------

#[test]
fn python_templates_emit_path_syntax() {
    let mut rng = ChaChaRng::seed_from_u64(42);

    for i in 0..4 {
        let spec = ModelSpec {
            name: format!("py_model_{}", i),
            layer: 2,
            dependencies: vec!["dep_a".into(), "dep_b".into()],
            model_type: ModelType::Python,
        };

        let py = generate_python(&mut rng, &spec);

        assert!(
            !py.contains("smelt.ref("),
            "Python template {i} must NOT contain 'smelt.ref(' (legacy form); got:\n{py}"
        );
        assert!(
            py.contains("smelt.models."),
            "Python template {i} must contain 'smelt.models.' (path form); got:\n{py}"
        );
    }
}

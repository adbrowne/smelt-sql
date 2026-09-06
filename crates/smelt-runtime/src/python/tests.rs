use super::*;
use smelt_core::config::Materialization;

/// Helper: find and copy the Python SDK into a temp dir.
fn setup_sdk(project_dir: &std::path::Path) {
    let sdk_dir = project_dir.join("python").join("smelt");
    std::fs::create_dir_all(&sdk_dir).unwrap();
    let repo_sdk = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("python")
        .join("smelt");
    for entry in std::fs::read_dir(&repo_sdk).unwrap() {
        let entry = entry.unwrap();
        if entry.path().is_file() {
            std::fs::copy(entry.path(), sdk_dir.join(entry.file_name())).unwrap();
        }
    }
}

fn minimal_config() -> Config {
    Config {
        name: "test".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets: std::collections::HashMap::new(),
        default_materialization: Materialization::View,
        models: std::collections::HashMap::new(),
        python: None,
        target: None,
        state: Default::default(),
        maintenance: None,
        probes: Default::default(),
    }
}

// ── Pure unit tests (no Python interpreter needed) ────────────────────────

#[test]
fn normalize_python_sql_section_delimiter() {
    // D-22: `--- name: X ---` is rewritten to plain `---\nname: X` frontmatter.
    let input = "--- name: my_model ---\nmaterialization: table\n---\nSELECT 1";
    let result = normalize_python_sql(input);
    assert!(result.starts_with("---\nname: my_model"));
}

#[test]
fn normalize_python_sql_plain_frontmatter_unchanged() {
    let input = "---\nmaterialization: table\n---\nSELECT 1";
    let result = normalize_python_sql(input);
    assert_eq!(result.as_ref(), input);
}

#[test]
fn normalize_python_sql_no_frontmatter_unchanged() {
    let input = "SELECT 1 AS id";
    let result = normalize_python_sql(input);
    assert_eq!(result.as_ref(), input);
}

#[test]
fn models_equal_order_independent() {
    let model_a = ModelFile {
        name: "alpha".to_string(),
        path: std::path::PathBuf::from("a.py"),
        content: "SELECT 1".to_string(),
        refs: vec![],
        parse_errors: vec![],
        metadata: None,
        kind: ModelKind::Sql,
        model_id: ModelId::from_path(std::path::PathBuf::from("a.py")),
        address_segments: Vec::new(),
    };
    let model_b = ModelFile {
        name: "beta".to_string(),
        path: std::path::PathBuf::from("b.py"),
        content: "SELECT 2".to_string(),
        refs: vec![],
        parse_errors: vec![],
        metadata: None,
        kind: ModelKind::Sql,
        model_id: ModelId::from_path(std::path::PathBuf::from("b.py")),
        address_segments: Vec::new(),
    };
    let set1 = vec![model_a.clone(), model_b.clone()];
    let set2 = vec![model_b, model_a];
    assert!(models_equal(&set1, &set2));
}

#[test]
fn models_equal_different_content() {
    let model_a = ModelFile {
        name: "same".to_string(),
        path: std::path::PathBuf::from("a.py"),
        content: "SELECT 1".to_string(),
        refs: vec![],
        parse_errors: vec![],
        metadata: None,
        kind: ModelKind::Sql,
        model_id: ModelId::from_path(std::path::PathBuf::from("a.py")),
        address_segments: Vec::new(),
    };
    let model_b = ModelFile {
        name: "same".to_string(),
        path: std::path::PathBuf::from("a.py"),
        content: "SELECT 2".to_string(),
        refs: vec![],
        parse_errors: vec![],
        metadata: None,
        kind: ModelKind::Sql,
        model_id: ModelId::from_path(std::path::PathBuf::from("a.py")),
        address_segments: Vec::new(),
    };
    assert!(!models_equal(&[model_a], &[model_b]));
}

// D-26: path-derived address segments (pure unit test, no Python).
#[test]
fn python_address_is_path_derived() {
    let root = std::path::PathBuf::from("/project");
    let paths = vec!["py".to_string()];

    // archive.py in py/ → ["archive"] + function name "users" → ["archive", "users"]
    let file = root.join("py").join("archive.py");
    let mut segs = ModelDiscovery::compute_address_segments(&file, &root, &paths);
    segs.push("users".to_string());
    assert_eq!(segs, vec!["archive", "users"]);

    // py/util.py + function "helper" → ["util", "helper"]
    let file2 = root.join("py").join("util.py");
    let mut segs2 = ModelDiscovery::compute_address_segments(&file2, &root, &paths);
    segs2.push("helper".to_string());
    assert_eq!(segs2, vec!["util", "helper"]);

    // subdirectory: py/staging/stg.py + function "events" → ["staging", "stg", "events"]
    let file3 = root.join("py").join("staging").join("stg.py");
    let mut segs3 = ModelDiscovery::compute_address_segments(&file3, &root, &paths);
    segs3.push("events".to_string());
    assert_eq!(segs3, vec!["staging", "stg", "events"]);
}

// D-26: collision between Python and SQL model with same path-derived address.
#[test]
fn python_sql_address_collision_is_duplicate_address() {
    use smelt_core::resolver::resolve_address_map;

    let sql_model = ModelFile {
        name: "users".to_string(),
        path: std::path::PathBuf::from("/project/py/archive.sql"),
        content: "SELECT 1".to_string(),
        refs: vec![],
        parse_errors: vec![],
        metadata: None,
        kind: ModelKind::Sql,
        model_id: ModelId::from_path(std::path::PathBuf::from("/project/py/archive.sql")),
        address_segments: vec!["archive".to_string(), "users".to_string()],
    };
    let python_model = ModelFile {
        name: "users".to_string(),
        path: std::path::PathBuf::from("/project/py/archive.py"),
        content: "SELECT 2".to_string(),
        refs: vec![],
        parse_errors: vec![],
        metadata: None,
        kind: ModelKind::Python {
            source_line: 1,
            queries: vec![],
        },
        model_id: ModelId::from_path(std::path::PathBuf::from("/project/py/archive.py")),
        address_segments: vec!["archive".to_string(), "users".to_string()],
    };

    let all_models = vec![sql_model, python_model];
    let (_map, collisions) = resolve_address_map(&all_models, &[], &[]);
    assert_eq!(
        collisions.len(),
        1,
        "same path-derived address must produce a collision"
    );
    assert_eq!(collisions[0].address, vec!["archive", "users"]);
}

// ── Integration tests (require Python interpreter + SDK) ─────────────────

/// `python_discovery_runs_in_runtime` — TDD anchor for P1: verify the moved
/// `discover_python_models` produces the same `ModelFile` set in runtime as
/// the CLI did. Uses the subprocess path (no PyO3 required).
#[test]
fn python_discovery_runs_in_runtime() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project_dir = tmp.path();
    setup_sdk(project_dir);

    // A minimal SQL model (smelt-core's discover_models errors on empty projects)
    let models_dir = project_dir.join("models");
    std::fs::create_dir_all(&models_dir).unwrap();
    std::fs::write(models_dir.join("anchor.sql"), "SELECT 1 AS anchor_id").unwrap();

    std::fs::write(
        models_dir.join("gen.py"),
        r#"
from smelt import model

@model
def dynamic_model(project):
    return "SELECT 1 as id, 'hello' as greeting"
"#,
    )
    .unwrap();

    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), vec!["models".to_string()]);
    let sql_models = discovery.discover_models().unwrap();
    let python_files = discovery.discover_python_files().unwrap();
    let config = minimal_config();

    let python_models =
        discover_python_models(&python_files, &sql_models, &config, project_dir, None).unwrap();

    assert_eq!(
        python_models.len(),
        1,
        "runtime must discover the Python model"
    );
    assert_eq!(python_models[0].name, "dynamic_model");
    assert!(python_models[0].content.contains("SELECT 1"));
    // D-26: path-derived address segments
    assert_eq!(
        python_models[0].address_segments,
        vec!["gen", "dynamic_model"]
    );
}

/// D-22: Python output with `--- name: X ---` section-delimiter format where X
/// matches the function name must be treated as single-model — no mismatch error.
#[test]
fn python_multimodel_delimiter_not_a_section() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project_dir = tmp.path();
    setup_sdk(project_dir);

    let models_dir = project_dir.join("models");
    std::fs::create_dir_all(&models_dir).unwrap();
    std::fs::write(models_dir.join("anchor.sql"), "SELECT 1 AS anchor_id").unwrap();

    std::fs::write(
        models_dir.join("matching_func.py"),
        r#"from smelt import model

@model
def matching_func(project):
    return """--- name: matching_func ---
materialization: table
---
SELECT 1 AS id
"""
"#,
    )
    .unwrap();

    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), vec!["models".to_string()]);
    let python_files = discovery.discover_python_files().unwrap();
    let config = minimal_config();

    let python_models =
        discover_python_models(&python_files, &[], &config, project_dir, None).unwrap();

    assert_eq!(
        python_models.len(),
        1,
        "section-delimiter must produce exactly 1 model (D-22)"
    );
    let model = &python_models[0];
    assert_eq!(model.name, "matching_func");
    assert!(
        model.parse_errors.is_empty(),
        "matching name must produce no errors; got: {:#?}",
        model.parse_errors
    );
    let meta = model.metadata.as_ref().expect("metadata must be populated");
    assert_eq!(meta.materialization, Some(Materialization::Table));
}

/// D-27: Python name mismatch must block the build AND retain other frontmatter keys.
#[test]
fn python_name_mismatch_blocks_and_retains_other_keys() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project_dir = tmp.path();
    setup_sdk(project_dir);

    let models_dir = project_dir.join("models");
    std::fs::create_dir_all(&models_dir).unwrap();
    std::fs::write(models_dir.join("anchor.sql"), "SELECT 1 AS anchor_id").unwrap();

    std::fs::write(
        models_dir.join("mismatch.py"),
        r#"from smelt import model

@model
def my_func(project):
    return """--- name: other_name ---
materialization: table
---
SELECT 1 AS id
"""
"#,
    )
    .unwrap();

    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), vec!["models".to_string()]);
    let python_files = discovery.discover_python_files().unwrap();
    let config = minimal_config();

    let python_models =
        discover_python_models(&python_files, &[], &config, project_dir, None).unwrap();

    assert_eq!(python_models.len(), 1, "model must still be produced");
    let model = &python_models[0];
    assert_eq!(model.name, "my_func");

    // PythonModelNameMismatch diagnostic must be present
    let mismatch_errs: Vec<_> = model
        .parse_errors
        .iter()
        .filter(|e| e.message.starts_with("PythonModelNameMismatch"))
        .collect();
    assert_eq!(
        mismatch_errs.len(),
        1,
        "expected exactly one PythonModelNameMismatch"
    );
    let msg = &mismatch_errs[0].message;
    assert!(msg.contains("other_name"), "must mention frontmatter name");
    assert!(msg.contains("my_func"), "must mention function name");

    // D-27: other frontmatter keys (materialization) must be RETAINED
    let meta = model
        .metadata
        .as_ref()
        .expect("metadata must be retained (D-27)");
    assert_eq!(meta.materialization, Some(Materialization::Table));
    assert!(meta.name.is_none(), "name: field must be stripped");
}

/// Non-convergent Python model (output changes every round) must produce an error.
#[test]
fn non_convergent_set_errors() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project_dir = tmp.path();
    setup_sdk(project_dir);

    let models_dir = project_dir.join("models");
    std::fs::create_dir_all(&models_dir).unwrap();

    let counter_file = tmp.path().join("counter.txt");
    std::fs::write(&counter_file, "0").unwrap();
    let counter_path = counter_file.display().to_string();

    let py_content = format!(
        r#"from smelt import model
import os

@model
def unstable(project):
    counter_file = r"{counter_path}"
    n = int(open(counter_file).read().strip())
    n += 1
    open(counter_file, "w").write(str(n))
    return f"SELECT {{n}}"
"#
    );
    std::fs::write(models_dir.join("unstable.py"), &py_content).unwrap();

    let python_files = vec![(
        models_dir.join("unstable.py"),
        vec![5u32],
        py_content.clone(),
    )];
    let config = minimal_config();

    let result = discover_python_models(&python_files, &[], &config, project_dir, None);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("converge") || err_msg.contains("circular"),
        "expected non-convergence error, got: {err_msg}"
    );
}

/// Regression (#189): concurrent discovery must not leak one model file's
/// registrations into another's results.
///
/// The embedded-PyO3 path mutates process-global interpreter state
/// (`smelt.core._registered_models`). Python releases the GIL during I/O,
/// so without a lock two concurrent `run_python_model_file` calls interleave
/// their clear/exec/collect sections and each can observe the other's models.
#[test]
fn concurrent_discovery_does_not_leak_models_across_files() {
    const THREADS: usize = 8;

    let handles: Vec<_> = (0..THREADS)
        .map(|i| {
            std::thread::spawn(move || {
                let tmp = tempfile::TempDir::new().unwrap();
                let project_dir = tmp.path();
                setup_sdk(project_dir);

                let models_dir = project_dir.join("models");
                std::fs::create_dir_all(&models_dir).unwrap();

                // A file the model reads, to force a GIL release inside the
                // critical section and widen the interleaving window.
                let marker = project_dir.join("marker.txt");
                std::fs::write(&marker, "ok").unwrap();

                let py_content = format!(
                    r#"from smelt import model

@model
def model_{i}(project):
    open(r"{marker}").read()
    return "SELECT {i} AS v"
"#,
                    i = i,
                    marker = marker.display()
                );
                let file = models_dir.join(format!("m{i}.py"));
                std::fs::write(&file, &py_content).unwrap();

                let python_files = vec![(file, vec![2u32], py_content)];
                let config = minimal_config();

                for _ in 0..5 {
                    let models =
                        discover_python_models(&python_files, &[], &config, project_dir, None)
                            .unwrap_or_else(|e| panic!("thread {i} discovery failed: {e}"));

                    assert_eq!(
                        models.len(),
                        1,
                        "thread {i} must see only its own model, got {:?}",
                        models.iter().map(|m| m.name.clone()).collect::<Vec<_>>()
                    );
                    assert_eq!(models[0].name, format!("model_{i}"));
                }
            })
        })
        .collect();

    for h in handles {
        h.join().expect("no thread may observe another's models");
    }
}

//! P3 smoke test: run `examples/multi_engine` Spark-targeted models against
//! a live Spark Connect server and **record every failure** as the break list
//! that scopes W2+.
//!
//! Success criterion for P3: the smoke ran and the break list was captured.
//! The test itself stays green even when every Spark model fails — W2 waves
//! fix the breaks. Only flip this assertion off after W2 ships.

mod common;
use common::spark_connect_url;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

fn multi_engine_dir() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/multi_engine")
}

fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let ty = entry.file_type().unwrap();
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), dst.join(entry.file_name())).unwrap();
        }
    }
}

/// Run `smelt run --target <target> --select <model>` and return (stdout, stderr, success).
fn smelt_run_select(
    project_dir: &std::path::Path,
    target: &str,
    model: &str,
) -> (String, String, bool) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_smelt"))
        .args([
            "run",
            "--project-dir",
            project_dir.to_str().unwrap(),
            "--target",
            target,
            "--select",
            model,
        ])
        .output()
        .expect("failed to spawn `smelt run`");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    )
}

/// P3 smoke: probe each Spark-targeted model in `examples/multi_engine`.
///
/// The test collects (model, error) pairs and prints them as a break list.
/// It does NOT assert that the models succeed — that is W2's job.
#[test]
fn spark_smoke_multi_engine() {
    // Skip when the `spark` feature wasn't compiled in (no Spark support in the binary).
    if !cfg!(feature = "spark") {
        eprintln!("spark feature not enabled — skipping spark_smoke_multi_engine");
        return;
    }
    let connect_url = match spark_connect_url() {
        Some(u) => u,
        None => {
            eprintln!("SPARK_CONNECT_URL unset — skipping spark_smoke_multi_engine");
            return;
        }
    };

    let src = multi_engine_dir();
    assert!(src.exists(), "examples/multi_engine not found at {src:?}");

    let tmp = TempDir::new().unwrap();
    let proj = tmp.path().join("multi_engine");
    copy_dir_all(&src, &proj);

    // Inject live connect_url and a temp warehouse path into smelt.yml.
    let warehouse = tmp.path().join("spark_warehouse");
    fs::create_dir_all(&warehouse).unwrap();
    let yml_path = proj.join("smelt.yml");
    let yml = fs::read_to_string(&yml_path).unwrap();
    let yml = yml
        .replace("sc://localhost:15002", &connect_url)
        .replace("target/spark", warehouse.to_str().unwrap());
    fs::write(&yml_path, yml).unwrap();

    // Probe each Spark-targeted model individually to isolate failures.
    // Models live under subdirectories so the selector requires the dotted path.
    let spark_models = &["staging.stg_sessions", "intermediate.int_visitor_daily"];
    let mut breaks: Vec<(String, String)> = Vec::new();

    for &model in spark_models {
        let (stdout, stderr, ok) = smelt_run_select(&proj, "spark_docker", model);
        if ok {
            println!("[PASS] {model}");
        } else {
            let first_err = stderr
                .lines()
                .chain(stdout.lines())
                .find(|l| !l.trim().is_empty())
                .unwrap_or("(no output)")
                .to_string();
            println!("[FAIL] {model}: {first_err}");
            println!("  --- stderr ---\n{stderr}");
            if !stdout.is_empty() {
                println!("  --- stdout ---\n{stdout}");
            }
            breaks.push((model.to_string(), first_err));
        }
    }

    // Print break-list summary for human recording in the plan.
    println!(
        "\n=== Spark Smoke P3 Break List ({} failure(s)) ===",
        breaks.len()
    );
    if breaks.is_empty() {
        println!("  (none — all Spark models passed on live Spark)");
    } else {
        for (i, (model, err)) in breaks.iter().enumerate() {
            println!("  BL-{}  model={}  err={}", i + 2, model, err);
        }
    }
    println!("=== End Break List ===\n");

    // P3 goal: break list captured. Do NOT assert success here.
    // After W2 ships dialect lowerings, flip this to assert breaks.is_empty().
}

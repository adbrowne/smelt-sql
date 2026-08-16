//! Walking-skeleton gate for the BigQuery backend: one model must materialize
//! in a real BigQuery dataset through the ordinary `smelt run` pipeline.
//!
//! Local-gated exactly as the Spark suites are (`multi_backend.md` §Surface —
//! `SMELT_BQ_PROJECT`): with the environment unset every test here **skips
//! green**, so the default `cargo test` stays backend-agnostic.
//!
//! Run it with:
//!   bash scripts/bigquery-auth.sh && source scripts/bigquery-env.sh
//!   cargo test -p smelt-cli --features bigquery --test bigquery_smoke

#![cfg(feature = "bigquery")]

use std::fs;
use std::path::Path;
use std::process::Command;

use smelt_backend::Backend;
use smelt_backend_bigquery::BigQueryBackend;
use tempfile::TempDir;

struct BqEnv {
    project: String,
    dataset: String,
    location: Option<String>,
    token: String,
}

/// The BigQuery environment, or `None` when the suite should skip.
///
/// Both the project and a live token are required: `SMELT_BQ_PROJECT` alone
/// would let the suite *fail* with an auth error rather than skip, which is
/// exactly the failure mode the skip gate exists to prevent.
fn bq_env() -> Option<BqEnv> {
    Some(BqEnv {
        project: std::env::var("SMELT_BQ_PROJECT").ok()?,
        token: std::env::var("SMELT_BQ_ACCESS_TOKEN").ok()?,
        dataset: std::env::var("SMELT_BQ_DATASET").unwrap_or_else(|_| "smelt_test".to_string()),
        location: std::env::var("SMELT_BQ_LOCATION").ok(),
    })
}

/// A dataset name unique to this run, so two worktrees — or a developer beside
/// an autonomy loop — never collide.
fn unique_dataset(base: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{}_{}_{}", base, std::process::id(), nanos)
}

/// Pick the dataset this run isolates itself in.
///
/// A dataset per run is the cleanest isolation (a whole run drops with one
/// `DROP SCHEMA … CASCADE`), but creating one needs `bigquery.datasets.create`,
/// which the least-privilege service account deliberately does not hold — it is
/// granted `WRITER` on one dataset and nothing wider. So when dataset creation
/// is refused, fall back to the granted dataset and isolate at table level
/// instead. Both paths are safe for concurrent runs; only the teardown differs.
async fn choose_dataset(env: &BqEnv) -> (String, bool) {
    let candidate = unique_dataset(&env.dataset);
    match BigQueryBackend::new(
        &env.project,
        &candidate,
        env.location.as_deref(),
        &env.token,
    )
    .await
    {
        Ok(_) => (candidate, true),
        Err(e) if e.to_string().contains("datasets.create") => {
            eprintln!(
                "note: service account cannot create datasets (least privilege); \
                 isolating by table name inside `{}` instead",
                env.dataset
            );
            (env.dataset.clone(), false)
        }
        Err(e) => panic!("unexpected failure preparing BigQuery dataset: {e}"),
    }
}

fn write_project(dir: &Path, project: &str, dataset: &str, location: Option<&str>) {
    let loc = location
        .map(|l| format!("    location: {l}\n"))
        .unwrap_or_default();
    fs::write(
        dir.join("smelt.yml"),
        format!(
            "name: bigquery_smoke\nversion: 1\ndefault_materialization: table\npaths:\n  - models\ntargets:\n  bq:\n    type: bigquery\n    project: {project}\n    dataset: {dataset}\n    schema: {dataset}\n{loc}"
        ),
    )
    .expect("write smelt.yml");

    fs::create_dir_all(dir.join("models")).expect("create models dir");
}

/// Write one model. The name is run-unique so concurrent runs sharing a dataset
/// never write the same table.
fn write_model(dir: &Path, model: &str) {
    fs::write(
        dir.join(format!("models/{model}.sql")),
        "SELECT 1 AS order_id, 'alice' AS customer\nUNION ALL\nSELECT 2 AS order_id, 'bob' AS customer\n",
    )
    .expect("write model");
}

/// One model materializes in a real BigQuery dataset via `smelt run`.
///
/// This is the phase gate: it proves the whole loop — target parsing, backend
/// construction from an explicit token, dataset creation, and
/// `CREATE OR REPLACE TABLE` — works end to end against a live warehouse.
#[test]
fn one_model_materializes_in_bigquery() {
    let Some(env) = bq_env() else {
        eprintln!("SMELT_BQ_PROJECT / SMELT_BQ_ACCESS_TOKEN unset — skipping BigQuery smoke");
        return;
    };

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let (dataset, owns_dataset) = runtime.block_on(choose_dataset(&env));
    let model = unique_dataset("smoke_orders");

    let tmp = TempDir::new().expect("tempdir");
    write_project(tmp.path(), &env.project, &dataset, env.location.as_deref());
    write_model(tmp.path(), &model);

    // Any table this run leaks expires on its own, so an interrupted run cannot
    // accumulate orphans in the project.
    let out = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .args([
            "run",
            "--project-dir",
            tmp.path().to_str().expect("utf8 path"),
            "--target",
            "bq",
        ])
        .env("SMELT_BQ_DEFAULT_TABLE_EXPIRATION_MS", "3600000")
        .output()
        .expect("spawn smelt");

    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();

    let verdict = runtime.block_on(async {
        let backend =
            BigQueryBackend::new(&env.project, &dataset, env.location.as_deref(), &env.token)
                .await
                .expect("construct BigQuery backend for verification");

        let exists = backend
            .table_exists(&dataset, &model)
            .await
            .expect("table_exists");
        let count = if exists {
            backend.get_row_count(&dataset, &model).await.unwrap_or(0)
        } else {
            0
        };

        // Drop the run-scoped dataset when this run created it; otherwise drop
        // just the table it wrote into the shared one. The default table
        // expiration set above is the backstop for either path not running.
        if owns_dataset {
            let _ = backend
                .execute_sql(&format!(
                    "DROP SCHEMA IF EXISTS `{}.{}` CASCADE",
                    env.project, dataset
                ))
                .await;
        } else {
            let _ = backend.drop_table_if_exists(&dataset, &model).await;
        }

        (exists, count)
    });

    assert!(
        out.status.success(),
        "smelt run failed against BigQuery.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        verdict.0,
        "model {model} was not materialized in dataset {dataset}.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(
        verdict.1, 2,
        "expected 2 rows in {model}, found {}",
        verdict.1
    );
}

/// The backend refuses to construct without an explicit token rather than
/// silently reaching for application-default credentials
/// (`multi_backend.md` §Surface — "BigQuery authenticates from an explicit
/// token"). This is a security property, so it is asserted, not assumed.
#[test]
fn empty_token_is_refused_rather_than_falling_back_to_adc() {
    let Some(env) = bq_env() else {
        eprintln!("SMELT_BQ_PROJECT / SMELT_BQ_ACCESS_TOKEN unset — skipping BigQuery smoke");
        return;
    };

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let result = runtime.block_on(BigQueryBackend::new(
        &env.project,
        &env.dataset,
        env.location.as_deref(),
        "",
    ));

    let err = result
        .err()
        .expect("an empty token must be refused, never resolved via ADC");
    let msg = err.to_string();
    assert!(
        msg.contains("explicit OAuth access token"),
        "refusal should name the missing token explicitly, got: {msg}"
    );
}

//! Structural tests for the Databricks Asset Bundle (criterion 11,
//! docs/outcomes/20260912-databricks-dogfood-spine/outcome.md,
//! phases/11a-plan.md). These parse the committed YAML with `serde_yaml`
//! rather than the full Databricks bundle schema, so they need no
//! workspace and no CLI to run — only `databricks_bundle_validate_is_clean`
//! shells out, and only when the CLI happens to be on `PATH`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn bundle_dir() -> PathBuf {
    repo_root().join("examples/github_activity")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

fn yaml(path: &Path) -> serde_yaml::Value {
    serde_yaml::from_str(&read(path)).unwrap_or_else(|e| panic!("parse {path:?} as YAML: {e}"))
}

fn databricks_yml() -> serde_yaml::Value {
    yaml(&bundle_dir().join("databricks.yml"))
}

fn job_yml() -> serde_yaml::Value {
    yaml(&bundle_dir().join("resources/github_activity_job.yml"))
}

fn the_one_job() -> serde_yaml::Value {
    named_job("github_activity_daily")
}

fn named_job(name: &str) -> serde_yaml::Value {
    let doc = job_yml();
    let jobs = doc["resources"]["jobs"]
        .as_mapping()
        .expect("resources.jobs is a mapping");
    jobs.get(serde_yaml::Value::String(name.to_string()))
        .unwrap_or_else(|| panic!("expected a job resource named `{name}`"))
        .clone()
}

#[test]
fn bundle_declares_one_daily_scheduled_serverless_job() {
    let job = the_one_job();

    let cron_ref = job["schedule"]["quartz_cron_expression"]
        .as_str()
        .expect("job.schedule.quartz_cron_expression is a string");
    assert_eq!(
        cron_ref, "${var.schedule_cron}",
        "job schedule must reference the schedule_cron variable, not a literal cron, so the \
         phase-11c proof deploy can compress the cadence without editing this file"
    );

    let bundle = databricks_yml();
    let cron = bundle["variables"]["schedule_cron"]["default"]
        .as_str()
        .expect("variables.schedule_cron.default is a string");
    assert!(!cron.trim().is_empty(), "cron expression must not be empty");
    // A daily cron in Quartz syntax fires once per day: fixed second/minute/hour
    // fields and `*` (every day of month) with `?` in the day-of-week field, or
    // the reverse. Either way every field except day-of-month/day-of-week is a
    // single fixed value, so no comma/slash-list appears in the first three
    // (second, minute, hour) fields.
    let fields: Vec<&str> = cron.split_whitespace().collect();
    assert!(
        fields.len() >= 6,
        "cron expression `{cron}` has too few fields for Quartz syntax"
    );
    for field in &fields[0..3] {
        assert!(
            !field.contains(',') && !field.contains('/'),
            "cron expression `{cron}` field `{field}` looks like it fires more than once a day"
        );
    }
    assert!(
        job["schedule"]["timezone_id"].as_str().is_some(),
        "job.schedule.timezone_id must be set"
    );

    let job_text = read(&bundle_dir().join("resources/github_activity_job.yml"));
    for forbidden in [
        "new_cluster",
        "existing_cluster_id",
        "node_type_id",
        "job_clusters",
    ] {
        assert!(
            !job_text.contains(forbidden),
            "job resource contains `{forbidden}` — Free Edition is serverless-only, no cluster \
             shape may appear"
        );
    }
}

#[test]
fn bundle_tasks_are_loader_then_smelt_run_in_order() {
    let job = the_one_job();
    let tasks = job["tasks"].as_sequence().expect("job.tasks is a sequence");
    assert_eq!(
        tasks.len(),
        2,
        "expected exactly two tasks (loader, smelt run)"
    );

    let key = |t: &serde_yaml::Value| t["task_key"].as_str().unwrap().to_string();
    assert_eq!(key(&tasks[0]), "load_next_day");
    assert_eq!(key(&tasks[1]), "smelt_run");

    assert!(
        tasks[0].get("depends_on").is_none(),
        "the loader task must not depend on anything"
    );
    let depends_on = tasks[1]["depends_on"]
        .as_sequence()
        .expect("smelt_run task has depends_on");
    assert_eq!(
        depends_on.len(),
        1,
        "smelt_run must depend on exactly one task"
    );
    assert_eq!(
        depends_on[0]["task_key"].as_str().unwrap(),
        "load_next_day",
        "smelt_run must depend on the loader task"
    );
}

#[test]
fn bundle_smelt_comes_from_the_locally_built_wheel() {
    let bundle = databricks_yml();
    let artifact = &bundle["artifacts"]["smelt_wheel"];
    assert_eq!(artifact["type"].as_str().unwrap(), "whl");
    let build = artifact["build"]
        .as_str()
        .expect("artifacts.smelt_wheel.build is a string");
    assert!(
        build.contains("maturin build"),
        "wheel artifact must be built by maturin: {build}"
    );

    let job_text = read(&bundle_dir().join("resources/github_activity_job.yml"));
    assert!(
        job_text.contains(".whl"),
        "the smelt_env environment must depend on a locally-built wheel"
    );
    assert!(
        !job_text.contains("smelt-sql=="),
        "the job environment must not pin a published `smelt-sql==<version>` yet — dev is ahead \
         of the last PyPI release"
    );

    let bundle_text = read(&bundle_dir().join("databricks.yml"));
    assert!(
        bundle_text.contains("PyPI"),
        "databricks.yml must name the PyPI-release placeholder in a comment so the swap is not \
         forgotten once a release tracks dev"
    );
}

#[test]
fn bundle_job_target_is_ambient_and_carries_no_literal_credential() {
    let smelt_yml = read(&bundle_dir().join("smelt.yml"));
    let doc: serde_yaml::Value = serde_yaml::from_str(&smelt_yml).unwrap();
    let target = &doc["targets"]["databricks_job"];
    assert_eq!(target["type"].as_str().unwrap(), "databricks");
    assert!(
        target.get("token").is_none(),
        "databricks_job target must carry no `token` key — the job's own environment supplies \
         Databricks credentials"
    );

    for path in [
        bundle_dir().join("databricks.yml"),
        bundle_dir().join("resources/github_activity_job.yml"),
        bundle_dir().join("dbx_job/load_next_day.py"),
        bundle_dir().join("dbx_job/run_smelt.py"),
    ] {
        let text = read(&path);
        assert!(
            !text.to_lowercase().contains("dapi"),
            "{path:?} must not contain a literal Databricks PAT (`dapi...`)"
        );
        assert!(
            !text.contains("https://") || !text.contains("token="),
            "{path:?} must not contain a URL-embedded token literal"
        );
    }
}

#[test]
fn bundle_project_and_state_live_on_a_unity_catalog_volume() {
    let job = the_one_job();
    let tasks = job["tasks"].as_sequence().unwrap();
    let smelt_run = tasks
        .iter()
        .find(|t| t["task_key"].as_str() == Some("smelt_run"))
        .expect("smelt_run task exists");
    let params = smelt_run["spark_python_task"]["parameters"]
        .as_sequence()
        .expect("smelt_run task has parameters");
    let volume_path = params[0]
        .as_str()
        .expect("first parameter is the project path");

    assert!(
        volume_path.starts_with("/Volumes/"),
        "smelt_run's project directory `{volume_path}` must be a Unity Catalog Volume path"
    );
    for var in ["${var.catalog}", "${var.schema}", "${var.volume_name}"] {
        assert!(
            volume_path.contains(var),
            "the Volume path `{volume_path}` must derive from the bundle variable `{var}` \
             rather than a hard-coded catalog/schema/volume name"
        );
    }
}

#[test]
fn bundle_targets_name_the_dogfood_workspace_as_a_target_entry() {
    let bundle = databricks_yml();
    let targets = bundle["targets"]
        .as_mapping()
        .expect("databricks.yml has a targets: mapping");
    assert!(
        !targets.is_empty(),
        "at least one bundle target must be declared"
    );
    assert!(
        targets.contains_key(serde_yaml::Value::String("dogfood".to_string())),
        "the dogfood workspace must be a named target entry"
    );

    // `workspace.host` cannot be templated (the Databricks CLI hard-refuses
    // variable interpolation on that field — measured against CLI v1.16.1: it
    // is an authentication field and the CLI insists on DATABRICKS_HOST
    // instead), so a second workspace is a different DATABRICKS_HOST at
    // invocation time, not a bundle variable — this asserts no real hostname
    // is committed either way.
    let bundle_text = read(&bundle_dir().join("databricks.yml"));
    assert!(
        !bundle_text.contains("cloud.databricks.com"),
        "databricks.yml must not commit a literal workspace hostname"
    );
}

/// `docs/outcomes/20260912-databricks-dogfood-spine/phases/11b-plan.md` test
/// 6: the `smelt_run` task's `--project-dir` and the declared Volume
/// resource must derive from the *same* `${var.…}` references, so they
/// cannot drift into naming two different paths.
#[test]
fn bundle_declares_the_volume_the_smelt_run_task_points_at() {
    let volume_yml = read(&bundle_dir().join("resources/volume.yml"));
    let doc: serde_yaml::Value = serde_yaml::from_str(&volume_yml).unwrap();
    let volumes = doc["resources"]["volumes"]
        .as_mapping()
        .expect("resources.volumes is a mapping");
    assert_eq!(volumes.len(), 1, "expected exactly one volume resource");
    let volume = volumes.values().next().unwrap();

    let catalog = volume["catalog_name"]
        .as_str()
        .expect("catalog_name is a string");
    let schema = volume["schema_name"]
        .as_str()
        .expect("schema_name is a string");
    let name = volume["name"].as_str().expect("name is a string");
    assert_eq!(catalog, "${var.catalog}");
    assert_eq!(schema, "${var.schema}");
    assert_eq!(name, "${var.volume_name}");

    let job = the_one_job();
    let tasks = job["tasks"].as_sequence().unwrap();
    let smelt_run = tasks
        .iter()
        .find(|t| t["task_key"].as_str() == Some("smelt_run"))
        .expect("smelt_run task exists");
    let volume_path = smelt_run["spark_python_task"]["parameters"][0]
        .as_str()
        .expect("first parameter is the project path");

    for var in ["${var.catalog}", "${var.schema}", "${var.volume_name}"] {
        assert!(
            volume_path.contains(var),
            "smelt_run's --project-dir `{volume_path}` must reference the same bundle \
             variable `{var}` the volume resource does"
        );
    }
}

/// Phase 11c test 2: the schedule declares `pause_status: UNPAUSED` so a
/// deploy enables it rather than relying on the Jobs API's create-time
/// default (which leaves a newly created schedule paused).
#[test]
fn bundle_schedule_is_explicitly_unpaused() {
    let job = the_one_job();
    let pause_status = job["schedule"]["pause_status"]
        .as_str()
        .expect("job.schedule.pause_status is a string");
    assert_eq!(
        pause_status, "UNPAUSED",
        "schedule must explicitly declare UNPAUSED so a deploy enables it rather than \
         inheriting the API's paused-by-default create behaviour"
    );
}

/// Phase 11c test 3: the `volume_probe` job's Volume path must compose from
/// the same `${var.…}` references the `smelt_run` task uses, so probe and
/// job cannot measure different paths.
#[test]
fn bundle_volume_probe_targets_the_declared_volume() {
    let job = named_job("github_activity_volume_probe");
    let tasks = job["tasks"].as_sequence().unwrap();
    let probe_task = tasks
        .iter()
        .find(|t| t["task_key"].as_str() == Some("volume_probe"))
        .expect("volume_probe task exists");
    let volume_path = probe_task["spark_python_task"]["parameters"][0]
        .as_str()
        .expect("first parameter is the project path");

    for var in ["${var.catalog}", "${var.schema}", "${var.volume_name}"] {
        assert!(
            volume_path.contains(var),
            "volume_probe's path `{volume_path}` must reference the same bundle variable \
             `{var}` the smelt_run task and volume resource do"
        );
    }

    let smelt_run_job = the_one_job();
    let smelt_run_tasks = smelt_run_job["tasks"].as_sequence().unwrap();
    let smelt_run = smelt_run_tasks
        .iter()
        .find(|t| t["task_key"].as_str() == Some("smelt_run"))
        .expect("smelt_run task exists");
    let smelt_run_path = smelt_run["spark_python_task"]["parameters"][0]
        .as_str()
        .expect("first parameter is the project path");
    assert_eq!(
        volume_path, smelt_run_path,
        "volume_probe and smelt_run must resolve to the identical Volume path"
    );
}

/// Test 7: `loader_env`'s `dependencies:` must cover every module the loader
/// imports at runtime (parsed from the loader file, not restated) — the
/// live gap was `duckdb`, added by 11b's Python-module DuckDB access path.
#[test]
fn bundle_loader_environment_declares_every_dependency_the_loader_imports() {
    let loader_text = read(&repo_root().join("scripts/dbx-dogfood-loader.py"));
    // Third-party imports the loader performs at module scope or inside a
    // function body — stdlib names are excluded by this fixed list rather
    // than derived, since there is no cheap way to distinguish stdlib from
    // third-party without a resolved environment.
    let stdlib = [
        "os",
        "re",
        "sys",
        "json",
        "argparse",
        "subprocess",
        "io",
        "smelt",
    ];
    let mut imported = std::collections::BTreeSet::new();
    for line in loader_text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("import ") {
            let module = rest.split(&[' ', '.', ','][..]).next().unwrap_or("").trim();
            if !module.is_empty() {
                imported.insert(module.to_string());
            }
        }
    }
    imported.retain(|m| !stdlib.contains(&m.as_str()));

    let job = the_one_job();
    let environments = job["environments"]
        .as_sequence()
        .expect("job.environments is a sequence");
    let loader_env = environments
        .iter()
        .find(|e| e["environment_key"].as_str() == Some("loader_env"))
        .expect("loader_env exists");
    let deps = loader_env["spec"]["dependencies"]
        .as_sequence()
        .expect("loader_env.spec.dependencies is a sequence")
        .iter()
        .map(|d| d.as_str().unwrap().to_string())
        .collect::<Vec<_>>();

    for module in &imported {
        assert!(
            deps.iter()
                .any(|d| d == module || d.starts_with(&format!("{module}=="))),
            "loader imports `{module}` but loader_env.dependencies is {deps:?}"
        );
    }
}

/// Test 8: the seed stage's copy list must name `smelt.yml` and `models/`
/// and must never name `.smelt` — re-seeding a deployed project must not be
/// able to destroy the ledger that makes each run incremental.
#[test]
fn bundle_seed_never_overwrites_run_state() {
    let script = read(&repo_root().join("scripts/dbx-bundle.sh"));
    let seed_items_line = script
        .lines()
        .find(|l| l.trim_start().starts_with("SEED_ITEMS="))
        .unwrap_or_else(|| panic!("expected a SEED_ITEMS= line in dbx-bundle.sh:\n{script}"));

    assert!(
        seed_items_line.contains("smelt.yml"),
        "seed copy list must name smelt.yml: {seed_items_line}"
    );
    assert!(
        seed_items_line.contains("models"),
        "seed copy list must name models/: {seed_items_line}"
    );
    assert!(
        !seed_items_line.contains(".smelt"),
        "seed copy list must never name .smelt (the run-state ledger): {seed_items_line}"
    );

    assert!(
        script.contains("seed"),
        "expected a seed subcommand wired into dbx-bundle.sh"
    );
}

#[test]
fn databricks_bundle_validate_is_clean() {
    if Command::new("databricks")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("databricks CLI not found on PATH — skipping (run `mise run setup-databricks`)");
        return;
    }

    let status = Command::new("bash")
        .arg(repo_root().join("scripts/dbx-bundle.sh"))
        .arg("validate")
        .current_dir(repo_root())
        .env_remove("DATABRICKS_HOST")
        .env_remove("DATABRICKS_TOKEN")
        .status()
        .expect("run scripts/dbx-bundle.sh validate");

    assert!(
        status.success(),
        "databricks bundle validate failed: {status:?}"
    );
}

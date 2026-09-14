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
    // Two artifacts, not one (phase 11i): a single `type: whl` artifact
    // producing both architectures' wheels hit databricks/cli#2969 (the
    // artifact mechanism assumes one file per artifact and deletes the one
    // it doesn't pick). Each is independently checked here.
    for name in ["smelt_wheel_x86_64", "smelt_wheel_aarch64"] {
        let artifact = &bundle["artifacts"][name];
        assert_eq!(artifact["type"].as_str().unwrap(), "whl", "{name}");
        let build = artifact["build"]
            .as_str()
            .unwrap_or_else(|| panic!("artifacts.{name}.build is a string"));
        assert!(
            build.contains("dbx-wheel-build.sh"),
            "{name} must be built by scripts/dbx-wheel-build.sh (which itself calls maturin): {build}"
        );
    }

    let job_text = read(&bundle_dir().join("resources/github_activity_job.yml"));
    assert!(
        job_text.contains("wheel_name"),
        "the smelt_env environment must depend on a locally-built wheel by its exact-filename \
         variable"
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

/// `docs/outcomes/20260912-databricks-dogfood-spine/phases/11d-plan.md` test
/// 7: the `databricks_job` target is fully ambient — neither `host` nor
/// `token` — and the bundle's job environments export no
/// `SMELT_DBX_HOST`/`DATABRICKS_HOST` for it.
#[test]
fn job_target_is_ambient() {
    let smelt_yml = read(&bundle_dir().join("smelt.yml"));
    let doc: serde_yaml::Value = serde_yaml::from_str(&smelt_yml).unwrap();
    let target = &doc["targets"]["databricks_job"];
    assert_eq!(target["type"].as_str().unwrap(), "databricks");
    assert!(
        target.get("token").is_none(),
        "databricks_job target must carry no `token` key — the job's own environment supplies \
         Databricks credentials"
    );
    assert!(
        target.get("host").is_none(),
        "databricks_job target must carry no `host` key — the ambient form builds its session \
         with no explicit host at all"
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

    // `databricks.yml` legitimately documents `DATABRICKS_HOST` as the
    // Databricks CLI's own workspace-auth mechanism for `bundle
    // validate`/`deploy`/`run` — that is the *deployer's* credential, not
    // the deployed job task's environment, so only the job resource and its
    // task scripts are checked for a host-bearing variable.
    //
    // `run_smelt.py` legitimately sets `SMELT_DBX_HOSTNAME` (note: distinct
    // from `SMELT_DBX_HOST`, which this still catches) to an inert
    // placeholder string — `smelt.yml` interpolates every target's env-var
    // references eagerly at load time, including the unused `databricks`/
    // `databricks_oracle` targets', so `databricks_job` (the target this
    // task actually selects, and which carries neither `host` nor `token`)
    // needs *something* there to load at all (measured phase 11i). The
    // placeholder is never read by the ambient session this target builds.
    for path in [
        bundle_dir().join("resources/github_activity_job.yml"),
        bundle_dir().join("dbx_job/load_next_day.py"),
        bundle_dir().join("dbx_job/run_smelt.py"),
    ] {
        let text = read(&path);
        assert!(
            !text.contains("SMELT_DBX_HOST\"") && !text.contains("DATABRICKS_HOST"),
            "{path:?} must export no host-bearing env var for the job target — measured \
             (phase 11c) that a Free Edition serverless job task never receives one"
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

/// Phase 11c test 1: every branch of dbx-bundle.sh that invokes `databricks`
/// against a real workspace must export both `DATABRICKS_HOST` and
/// `DATABRICKS_TOKEN` — the 11c deploy attempt found `seed`/`deploy`/`run`
/// silently missing the token export, which broke every live subcommand
/// outright. This locks the fix in structurally rather than trusting a live
/// failure to catch a regression.
#[test]
fn every_live_subcommand_exports_both_credentials() {
    let script = read(&repo_root().join("scripts/dbx-bundle.sh"));

    // Every non-validate `databricks` invocation site in the script must have
    // a `DATABRICKS_HOST=... DATABRICKS_TOKEN=...` prefix somewhere in the
    // two lines above it.
    let lines: Vec<&str> = script.lines().collect();
    let mut checked_any = false;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let is_live_invocation = (trimmed.starts_with("exec databricks")
            || trimmed.starts_with("databricks jobs")
            || trimmed.starts_with("databricks fs cp"))
            && !trimmed.contains("--version");
        if !is_live_invocation {
            continue;
        }
        checked_any = true;
        let window_start = i.saturating_sub(2);
        let window = lines[window_start..=i].join("\n");
        assert!(
            window.contains("DATABRICKS_HOST=\"${SMELT_DBX_HOST}\""),
            "live invocation at line {} must export DATABRICKS_HOST from SMELT_DBX_HOST:\n{}",
            i + 1,
            window
        );
        assert!(
            window.contains("DATABRICKS_TOKEN=\"${SMELT_DBX_TOKEN}\""),
            "live invocation at line {} must export DATABRICKS_TOKEN from SMELT_DBX_TOKEN:\n{}",
            i + 1,
            window
        );
    }
    assert!(
        checked_any,
        "expected at least one live `databricks` invocation site in dbx-bundle.sh"
    );
}

/// Phase 11c test 2: the `runs` subcommand must be read-only — no mutating
/// `databricks` verb (`deploy`, `run`, `bundle run`, `fs cp`, `destroy`) may
/// appear in its branch, since it exists purely to poll scheduled-run status
/// during the wait loop.
#[test]
fn runs_subcommand_is_read_only() {
    let script = read(&repo_root().join("scripts/dbx-bundle.sh"));
    let start = script
        .find("if [[ \"${SUBCOMMAND}\" == \"runs\" ]]")
        .expect("expected a runs subcommand branch in dbx-bundle.sh");
    let end = script[start..]
        .find("if [[ \"${SUBCOMMAND}\" == \"seed\" ]]")
        .map(|i| start + i)
        .expect("expected the seed branch to follow the runs branch");
    let runs_block = &script[start..end];

    assert!(
        runs_block.contains("jobs list-runs") || runs_block.contains("jobs list"),
        "runs branch must call `databricks jobs list`/`list-runs`"
    );
    assert!(
        runs_block.contains("jobs get-run"),
        "runs branch must call `databricks jobs get-run`"
    );
    for forbidden in [
        "bundle deploy",
        "bundle run",
        "bundle destroy",
        "fs cp",
        "jobs run-now",
    ] {
        assert!(
            !runs_block.contains(forbidden),
            "runs branch must not contain the mutating verb `{forbidden}`"
        );
    }
}

/// Phase 11f test 1: `dbx-wheel-build.sh verify` rejects a wheel tagged
/// above the declared manylinux_2_28 floor, naming both the tag and the
/// floor in its error.
#[test]
fn wheel_verify_rejects_a_too_new_manylinux_tag() {
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/dbx-wheel-build.sh"))
        .arg("verify")
        .arg("smelt_sql-0.3.2-cp312-cp312-manylinux_2_39_x86_64.whl")
        .output()
        .expect("run dbx-wheel-build.sh verify");

    assert!(
        !output.status.success(),
        "verify must reject a manylinux_2_39 wheel"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("manylinux_2_39"),
        "error must name the wheel's own tag: {stderr}"
    );
    assert!(
        stderr.contains("manylinux_2_28"),
        "error must name the declared floor: {stderr}"
    );
}

/// Phase 11f test 2: `verify` accepts the declared floor and anything below
/// it, including the legacy numbered tags.
#[test]
fn wheel_verify_accepts_the_declared_floor_and_below() {
    for wheel in [
        "smelt_sql-0.3.2-cp312-cp312-manylinux_2_28_x86_64.whl",
        "smelt_sql-0.3.2-cp312-cp312-manylinux_2_17_x86_64.whl",
        "smelt_sql-0.3.2-cp312-cp312-manylinux2014_x86_64.whl",
    ] {
        let status = Command::new("bash")
            .arg(repo_root().join("scripts/dbx-wheel-build.sh"))
            .arg("verify")
            .arg(wheel)
            .status()
            .expect("run dbx-wheel-build.sh verify");
        assert!(status.success(), "verify must accept {wheel}");
    }
}

/// Phase 11f test 3: `verify` rejects a plain (unrepaired) `linux_x86_64`
/// wheel — auditwheel repair never ran, so the vendored libduckdb is missing.
#[test]
fn wheel_verify_rejects_an_unrepaired_linux_tag() {
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/dbx-wheel-build.sh"))
        .arg("verify")
        .arg("smelt_sql-0.3.2-cp312-cp312-linux_x86_64.whl")
        .output()
        .expect("run dbx-wheel-build.sh verify");
    assert!(
        !output.status.success(),
        "verify must reject an unrepaired linux_x86_64 wheel"
    );
}

/// Phase 11f test 4: each wheel artifact's `build:` must invoke the
/// single-owner script rather than a bare `maturin build`, so a second
/// spelling of the build (and its manylinux floor) cannot drift back in.
#[test]
fn smelt_wheel_build_uses_the_manylinux_build_script() {
    let bundle = databricks_yml();
    for name in ["smelt_wheel_x86_64", "smelt_wheel_aarch64"] {
        let build = bundle["artifacts"][name]["build"]
            .as_str()
            .unwrap_or_else(|| panic!("artifacts.{name}.build is a string"));
        assert!(
            build.contains("dbx-wheel-build.sh"),
            "{name} build must invoke scripts/dbx-wheel-build.sh: {build}"
        );
        assert!(
            !build.contains("maturin build"),
            "{name} build must not contain a bare 'maturin build' — that belongs solely to \
             dbx-wheel-build.sh: {build}"
        );
    }
}

/// Phase 11f test 5: the manylinux floor literal is stated exactly once, in
/// the build script — `databricks.yml` and the job resource must not restate
/// it, so the floor cannot drift out of sync between the two.
#[test]
fn smelt_wheel_floor_is_stated_once() {
    let script = read(&repo_root().join("scripts/dbx-wheel-build.sh"));
    assert!(
        script.contains("manylinux_2_28"),
        "dbx-wheel-build.sh must declare the manylinux_2_28 floor"
    );

    for path in [
        bundle_dir().join("databricks.yml"),
        bundle_dir().join("resources/github_activity_job.yml"),
    ] {
        let text = read(&path);
        assert!(
            !text.contains("manylinux"),
            "{path:?} must not restate the manylinux floor — that is dbx-wheel-build.sh's alone"
        );
    }
}

/// Phase 11h test 1: `verify` accepts an `aarch64` manylinux tag exactly as
/// it accepts `x86_64` — `glibc_version_for_tag` parses the glibc
/// major/minor from the tag body and never looks at the trailing arch
/// suffix, so this is a regression guard on existing behaviour, not new
/// parsing logic.
#[test]
fn wheel_verify_accepts_an_aarch64_manylinux_tag() {
    let status = Command::new("bash")
        .arg(repo_root().join("scripts/dbx-wheel-build.sh"))
        .arg("verify")
        .arg("smelt_sql-0.3.2-cp311-cp311-manylinux_2_28_aarch64.whl")
        .status()
        .expect("run dbx-wheel-build.sh verify");
    assert!(
        status.success(),
        "verify must accept an aarch64 manylinux_2_28 wheel"
    );
}

/// Phase 11h test 2: `SMELT_WHEEL_BUILDER=docker build aarch64` refuses
/// outright — cross-arch emulation inside the manylinux_2_28_aarch64
/// container needs binfmt/QEMU this box is not known to have, so the script
/// must say so rather than silently building the wrong arch or hanging on
/// an emulated pull.
#[test]
fn dbx_wheel_build_rejects_docker_for_aarch64() {
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/dbx-wheel-build.sh"))
        .arg("build")
        .arg("aarch64")
        .env("SMELT_WHEEL_BUILDER", "docker")
        .output()
        .expect("run dbx-wheel-build.sh build aarch64");
    assert!(
        !output.status.success(),
        "build aarch64 under SMELT_WHEEL_BUILDER=docker must fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("aarch64"),
        "error must name aarch64: {stderr}"
    );
    assert!(
        stderr.contains("docker"),
        "error must name docker: {stderr}"
    );
}

/// Phase 11h test 3: `smelt_env.dependencies` is exactly two entries, each
/// scoped to one architecture by a `platform_machine` marker, replacing the
/// old bare `../../../dist/*.whl` — Databricks serverless compute can land a
/// job task on either `aarch64` or `x86_64` with no pinning mechanism on the
/// platform side (docs/outcomes/20260912-databricks-dogfood-spine/phases/
/// 11h-plan.md).
#[test]
fn smelt_env_dependencies_are_arch_scoped() {
    let job = the_one_job();
    let environments = job["environments"]
        .as_sequence()
        .expect("job.environments is a sequence");
    let smelt_env = environments
        .iter()
        .find(|e| e["environment_key"].as_str() == Some("smelt_env"))
        .expect("smelt_env exists");
    let deps = smelt_env["spec"]["dependencies"]
        .as_sequence()
        .expect("smelt_env.spec.dependencies is a sequence")
        .iter()
        .map(|d| d.as_str().unwrap().to_string())
        .collect::<Vec<_>>();

    assert_eq!(
        deps.len(),
        4,
        "smelt_env.dependencies must be exactly two arch-scoped wheel entries plus \
         databricks-connect and pyarrow (needed by the embedded PyO3 interpreter's \
         `smelt.databricks_adapter` import once it reaches the Databricks backend — \
         measured phase 11m): {deps:?}"
    );
    assert!(
        !deps.iter().any(|d| d == "../../../dist/*.whl"),
        "smelt_env.dependencies must not contain the old unscoped glob: {deps:?}"
    );
    assert!(
        deps.iter().any(|d| d == "databricks-connect==15.4.5"),
        "smelt_env.dependencies must pin databricks-connect the same way loader_env does: {deps:?}"
    );
    assert!(
        deps.iter().any(|d| d == "pyarrow"),
        "smelt_env.dependencies must include pyarrow: {deps:?}"
    );

    // The glob disambiguates by the wheel filename's own trailing arch
    // suffix (`_x86_64.whl` / `_aarch64.whl`), not by restating the
    // manylinux floor — that spelling stays single-owned in
    // dbx-wheel-build.sh per `smelt_wheel_floor_is_stated_once`.
    let x86 = deps
        .iter()
        .find(|d| d.contains("x86_64"))
        .unwrap_or_else(|| panic!("expected an x86_64-scoped entry: {deps:?}"));
    assert!(
        x86.contains("platform_machine == \"x86_64\""),
        "x86_64 entry must carry a platform_machine marker: {x86}"
    );
    // References the exact deployed filename via a bundle variable, not a
    // literal glob suffix (phase 11i): pip's `/Workspace/...` requirement
    // resolver does not expand `*`, so the exact filename `dbx-bundle.sh
    // deploy` just built is passed through `${var.x86_64_wheel_name}` /
    // `${var.aarch64_wheel_name}` instead.
    assert!(
        x86.contains("${var.x86_64_wheel_name}"),
        "x86_64 entry must reference the exact-filename variable: {x86}"
    );

    let arm = deps
        .iter()
        .find(|d| d.contains("aarch64"))
        .unwrap_or_else(|| panic!("expected an aarch64-scoped entry: {deps:?}"));
    assert!(
        arm.contains("platform_machine == \"aarch64\""),
        "aarch64 entry must carry a platform_machine marker: {arm}"
    );
    assert!(
        arm.contains("${var.aarch64_wheel_name}"),
        "aarch64 entry must reference the exact-filename variable: {arm}"
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

// --- Phase 11j: `smelt state seed-interval` bootstrap tool -----------------
//
// `docs/outcomes/20260912-databricks-dogfood-spine/phases/11j-plan.md` test 5.
// Fully offline: a scaffolded temp project with a `duckdb`-typed target (the
// target *type* is irrelevant to this command — it never connects to a
// backend — only the target *name* used to key `.smelt/targets/<name>/`).

fn stage_seed_interval_project(dir: &Path) {
    fs::create_dir_all(dir.join("models")).unwrap();
    fs::write(
        dir.join("smelt.yml"),
        "name: seed_interval_test\nversion: 1\npaths:\n  - models\ntargets:\n  databricks_job:\n    type: duckdb\n    database: db.duckdb\n    schema: main\ndefault_materialization: table\n",
    )
    .unwrap();
    fs::write(dir.join("models/m.sql"), "SELECT 1 AS x\n").unwrap();
}

#[test]
fn seed_intervals_subcommand_prints_the_written_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project_dir = tmp.path().join("proj");
    stage_seed_interval_project(&project_dir);

    let smelt_bin = PathBuf::from(env!("CARGO_BIN_EXE_smelt"));
    let out = Command::new(smelt_bin)
        .arg("state")
        .arg("seed-interval")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .args(["--target", "databricks_job"])
        .args(["--model", "m"])
        .args(["--start", "2026-08-01"])
        .args(["--end", "2026-08-16"])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt state seed-interval`: {e}"));

    assert!(
        out.status.success(),
        "seed-interval should exit 0.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let intervals_path = project_dir
        .join(".smelt")
        .join("targets")
        .join("databricks_job")
        .join("intervals.json");
    assert!(
        intervals_path.is_file(),
        "expected {intervals_path:?} to be written"
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&intervals_path.display().to_string()),
        "stdout should print the written path: {stdout}"
    );

    let contents = fs::read_to_string(&intervals_path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&contents).unwrap();
    let entry = &json["m"];
    assert_eq!(entry["covered_intervals"][0]["start"], "2026-08-01");
    assert_eq!(entry["covered_intervals"][0]["end"], "2026-08-16");
}

#[test]
fn seed_intervals_subcommand_refuses_an_unknown_model() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project_dir = tmp.path().join("proj");
    stage_seed_interval_project(&project_dir);

    let smelt_bin = PathBuf::from(env!("CARGO_BIN_EXE_smelt"));
    let out = Command::new(smelt_bin)
        .arg("state")
        .arg("seed-interval")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .args(["--target", "databricks_job"])
        .args(["--model", "does_not_exist"])
        .args(["--start", "2026-08-01"])
        .args(["--end", "2026-08-16"])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt state seed-interval`: {e}"));

    assert!(
        !out.status.success(),
        "seed-interval must refuse a model that isn't in the project"
    );
}

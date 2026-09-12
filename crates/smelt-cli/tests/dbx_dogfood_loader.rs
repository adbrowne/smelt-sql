#![cfg(feature = "duckdb")]
//! Tests for `scripts/dbx-dogfood-loader.py`/`.sh`, the pinned
//! `databricks-connect` client venv, and `scripts/dbx-dogfood-env.sh`
//! (`docs/outcomes/20260912-databricks-dogfood-spine/phases/03-plan.md`).
//!
//! The loader is external to smelt — smelt's source declaration is the
//! contract it is trusted against, exactly as `scripts/bq-dogfood-loader.sh`
//! is for BigQuery (`crates/smelt-cli/tests/github_activity_loader.rs`). Its
//! `--emit-ddl`/`--emit-sql`/`--emit-slice-sql` modes touch no network and
//! need no `databricks-connect` package or workspace, so this file proves
//! the loader is byte-for-byte faithful to
//! `examples/github_activity/load_day.sh`'s own redelivery rule with no
//! cloud in the loop.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

mod github_activity_support;
use github_activity_support::{load_day, stage_workspace, FIXTURE_DAYS};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn loader_py() -> PathBuf {
    repo_root().join("scripts/dbx-dogfood-loader.py")
}

fn load_day_sh() -> PathBuf {
    repo_root().join("examples/github_activity/load_day.sh")
}

/// Python interpreter with `pyarrow` and `databricks-connect` importable —
/// the pinned `.smelt-dbx-venv` built by `scripts/dbx-dogfood-venv.sh`, if
/// present. Tests that drive `python/smelt/databricks_adapter.py` for real
/// (rather than faking the module out of `sys.modules`) need it and skip
/// when it hasn't been built yet, matching this repo's `SPARK_CONNECT_URL`
/// gating convention for tests that need an optional local environment.
fn dbx_venv_python() -> Option<PathBuf> {
    let candidate = repo_root().join(".smelt-dbx-venv/bin/python");
    if !candidate.exists() {
        return None;
    }
    let ok = Command::new(&candidate)
        .args(["-c", "import pyarrow"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    ok.then_some(candidate)
}

fn run_loader(args: &[&str]) -> Output {
    Command::new("python3")
        .arg(loader_py())
        .args(args)
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn dbx-dogfood-loader.py: {e}"))
}

fn duckdb_scalar(sql: &str) -> i64 {
    let out = Command::new("duckdb")
        .args([":memory:", "-json", "-c", sql])
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn duckdb: {e}"));
    assert!(
        out.status.success(),
        "duckdb query failed: {}\nsql: {sql}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let rows: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("invalid JSON: {e}\n{stdout}"));
    rows[0]
        .as_object()
        .and_then(|obj| obj.values().next())
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| panic!("expected one scalar column in query result: {rows}"))
}

fn duckdb_scalar_against(db: &Path, sql: &str) -> i64 {
    let out = Command::new("duckdb")
        .arg(db)
        .args(["-json", "-c", sql])
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn duckdb: {e}"));
    assert!(
        out.status.success(),
        "duckdb query failed: {}\nsql: {sql}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let rows: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("invalid JSON: {e}\n{stdout}"));
    rows[0]
        .as_object()
        .and_then(|obj| obj.values().next())
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| panic!("expected one scalar column in query result: {rows}"))
}

/// Splits `--emit-slice-sql`'s output into (events_query, arrival_query),
/// delimited by the `-- events` / `-- arrival` markers the loader prints.
fn split_slice_sql(emitted: &str) -> (String, String) {
    let events_marker = "-- events";
    let arrival_marker = "-- arrival";
    let events_start = emitted
        .find(events_marker)
        .unwrap_or_else(|| panic!("missing '{events_marker}' marker in:\n{emitted}"));
    let arrival_start = emitted
        .find(arrival_marker)
        .unwrap_or_else(|| panic!("missing '{arrival_marker}' marker in:\n{emitted}"));
    assert!(events_start < arrival_start);
    let events = emitted[events_start + events_marker.len()..arrival_start]
        .trim()
        .trim_end_matches(';')
        .to_string();
    let arrival = emitted[arrival_start + arrival_marker.len()..]
        .trim()
        .trim_end_matches(';')
        .to_string();
    (events, arrival)
}

fn parsed_modulus() -> u32 {
    let text = std::fs::read_to_string(load_day_sh()).expect("read load_day.sh");
    let line = text
        .lines()
        .find(|l| l.contains("MOD(CAST(id AS BIGINT),"))
        .expect("load_day.sh names its redelivery modulus");
    let after = line
        .split("MOD(CAST(id AS BIGINT),")
        .nth(1)
        .expect("modulus follows MOD(CAST(id AS BIGINT),");
    after
        .trim_start()
        .split(')')
        .next()
        .expect("modulus is followed by a closing paren")
        .trim()
        .parse()
        .expect("modulus is an integer")
}

#[test]
fn slice_matches_load_day_rows_for_every_fixture_day() {
    for day in FIXTURE_DAYS {
        let tmp = TempDir::new().expect("tempdir");
        let (_workspace, db, sample) = stage_workspace(tmp.path());
        load_day(&db, &sample, day, None);

        let out = run_loader(&["--emit-slice-sql", "--date", day]);
        assert!(
            out.status.success(),
            "day {day} stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let emitted = String::from_utf8_lossy(&out.stdout).to_string();
        let (events_sql, _) = split_slice_sql(&emitted);

        let left = duckdb_scalar_against(
            &db,
            &format!(
                "SELECT count(*) AS c FROM (SELECT * FROM main.sources_raw_github_events \
                 EXCEPT ALL ({events_sql})) t"
            ),
        );
        let right = duckdb_scalar_against(
            &db,
            &format!(
                "SELECT count(*) AS c FROM (({events_sql}) \
                 EXCEPT ALL SELECT * FROM main.sources_raw_github_events) t"
            ),
        );
        assert_eq!(
            left, 0,
            "day {day}: loaded rows not covered by the emitted slice"
        );
        assert_eq!(
            right, 0,
            "day {day}: emitted slice rows not present in load_day.sh's output"
        );

        let loaded_count = duckdb_scalar_against(
            &db,
            "SELECT count(*) AS c FROM main.sources_raw_github_events",
        );
        let slice_count =
            duckdb_scalar_against(&db, &format!("SELECT count(*) AS c FROM ({events_sql}) t"));
        assert_eq!(loaded_count, slice_count, "day {day}: row counts diverge");
    }
}

#[test]
fn arrival_slice_stamps_ingested_date_like_load_day() {
    let day = "2026-08-06";
    let tmp = TempDir::new().expect("tempdir");
    let (_workspace, db, sample) = stage_workspace(tmp.path());
    load_day(&db, &sample, day, None);

    let out = run_loader(&["--emit-slice-sql", "--date", day]);
    assert!(out.status.success());
    let emitted = String::from_utf8_lossy(&out.stdout).to_string();
    let (_, arrival_sql) = split_slice_sql(&emitted);

    let left = duckdb_scalar_against(
        &db,
        &format!(
            "SELECT count(*) AS c FROM (SELECT * FROM main.sources_raw_github_events_arrival \
             EXCEPT ALL ({arrival_sql})) t"
        ),
    );
    let right = duckdb_scalar_against(
        &db,
        &format!(
            "SELECT count(*) AS c FROM (({arrival_sql}) \
             EXCEPT ALL SELECT * FROM main.sources_raw_github_events_arrival) t"
        ),
    );
    assert_eq!(left, 0);
    assert_eq!(right, 0);

    let stamped = duckdb_scalar_against(
        &db,
        &format!("SELECT count(*) AS c FROM ({arrival_sql}) t WHERE ingested_date <> DATE '{day}'"),
    );
    assert_eq!(
        stamped, 0,
        "every row in the arrival slice must be stamped ingested_date = {day}"
    );
}

#[test]
fn first_fixture_day_has_an_empty_redelivery_arm() {
    let day = FIXTURE_DAYS[0];
    let out = run_loader(&["--emit-slice-sql", "--date", day]);
    assert!(out.status.success());
    let emitted = String::from_utf8_lossy(&out.stdout).to_string();
    let (events_sql, _) = split_slice_sql(&emitted);

    let sample = repo_root().join("examples/github_activity/seeds/github_events_sample.parquet");
    let plain_day_count = duckdb_scalar(&format!(
        "SELECT count(*) AS c FROM read_parquet('{}') WHERE CAST(created_at AS DATE) = DATE '{day}'",
        sample.display()
    ));
    let slice_count = duckdb_scalar(&format!("SELECT count(*) AS c FROM ({events_sql}) t"));
    assert_eq!(
        plain_day_count, slice_count,
        "the earliest fixture day has no D-1 rows in the sample, so its redelivery arm must \
         match zero rows and the loader needs no special case"
    );
}

#[test]
fn redelivery_modulus_is_parsed_from_load_day_not_restated() {
    let expected = parsed_modulus();
    let out = run_loader(&["--emit-slice-sql", "--date", "2026-08-06"]);
    assert!(out.status.success());
    let emitted = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        emitted.contains(&format!("MOD(CAST(id AS BIGINT), {expected})")),
        "expected the loader to use load_day.sh's own modulus ({expected}) in:\n{emitted}"
    );
}

#[test]
fn loader_is_idempotent_per_day() {
    let tmp = TempDir::new().expect("tempdir");
    let store = tmp.path().join("store");
    std::fs::create_dir_all(&store).expect("mkdir store");

    let out = run_loader(&[
        "--date",
        "2026-08-06",
        "--dry-run-store",
        store.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let first_stdout = String::from_utf8_lossy(&out.stdout).to_lowercase();
    assert!(
        first_stdout.contains("loaded"),
        "first run should report it loaded: {first_stdout}"
    );
    let ledger = store.join("loader_days.txt");
    assert!(
        ledger.exists(),
        "first run should record the day in the store's ledger"
    );
    let ledger_after_first = std::fs::read_to_string(&ledger).expect("read ledger");
    assert!(ledger_after_first.contains("2026-08-06"));

    let out = run_loader(&[
        "--date",
        "2026-08-06",
        "--dry-run-store",
        store.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "second invocation should also exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout).to_lowercase(),
        String::from_utf8_lossy(&out.stderr).to_lowercase()
    );
    assert!(
        combined.contains("already loaded"),
        "second run should report already loaded: {combined}"
    );
    let ledger_after_second = std::fs::read_to_string(&ledger).expect("read ledger");
    assert_eq!(
        ledger_after_first, ledger_after_second,
        "the second, already-loaded run must write nothing further to the ledger"
    );
}

#[test]
fn emitted_load_statements_reference_no_host_path() {
    for (args, _label) in [
        (vec!["--emit-sql", "--date", "2026-08-06"], "emit-sql"),
        (vec!["--emit-ddl"], "emit-ddl"),
    ] {
        let out = run_loader(&args);
        assert!(
            out.status.success(),
            "{args:?} stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let emitted = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            !emitted.contains("seeds/github_events_sample.parquet"),
            "{args:?} must not reference the fixture's host path:\n{emitted}"
        );
        assert!(
            !emitted.to_lowercase().contains(".parquet"),
            "{args:?} must not reference any parquet file path:\n{emitted}"
        );
        assert!(
            emitted.contains("smelt_dogfood.github_events"),
            "{args:?} should name the catalog-qualified target table:\n{emitted}"
        );
    }
}

#[test]
fn client_env_is_pinned_and_disjoint_from_the_spark_venv() {
    let pin_file = repo_root().join("scripts/dbx-dogfood-requirements.txt");
    let pins = std::fs::read_to_string(&pin_file).expect("read dbx-dogfood-requirements.txt");
    assert!(
        pins.lines().any(|l| {
            let l = l.trim();
            l.starts_with("databricks-connect==")
        }),
        "expected an exact databricks-connect== pin in:\n{pins}"
    );
    assert!(
        !pins.lines().any(|l| {
            let l = l.trim();
            l == "pyspark" || l.starts_with("pyspark==") || l.starts_with("pyspark>=")
        }),
        "the databricks-connect client must not co-install bare pyspark:\n{pins}"
    );

    let venv_script = repo_root().join("scripts/dbx-dogfood-venv.sh");
    let script = std::fs::read_to_string(&venv_script).expect("read dbx-dogfood-venv.sh");
    assert!(
        script.contains(".smelt-dbx-venv"),
        "expected the dbx venv script to target .smelt-dbx-venv"
    );
    assert!(
        !script.contains(".smelt-spark-venv"),
        "the dbx venv script must not target the local-Spark venv"
    );
}

#[test]
fn dbx_dogfood_env_exports_the_dbx_venv_and_no_secret() {
    let env_script = repo_root().join("scripts/dbx-dogfood-env.sh");
    assert!(env_script.exists());

    // A real credential config now lives at the default
    // $HOME/.config/databricks-smelt-dogfood (phase 4b's live provisioning), so
    // merely removing the three env vars falls back to reading it and the "no
    // credential config on disk" assertion below would see a real host. Point
    // SMELT_DBX_CONFIG_DIR at an empty tempdir to genuinely isolate this case.
    let tmp = TempDir::new().expect("tempdir");
    let out = Command::new("bash")
        .arg("-c")
        .arg(format!(
            "set -e; source {}; echo \"PYTHONPATH=$PYTHONPATH\"; echo \"GATE=${{SMELT_DBX_HOST-UNSET}}\"",
            env_script.display()
        ))
        .current_dir(repo_root())
        .env_remove("SMELT_DBX_HOST")
        .env_remove("SMELT_DBX_TOKEN")
        .env("SMELT_DBX_CONFIG_DIR", tmp.path())
        .output()
        .unwrap_or_else(|e| panic!("failed to source dbx-dogfood-env.sh: {e}"));
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        stdout.contains(".smelt-dbx-venv"),
        "expected the dbx venv's site-packages on PYTHONPATH:\n{stdout}"
    );
    assert!(
        !stdout.contains(".smelt-spark-venv"),
        "must never put the local-Spark venv on PYTHONPATH:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("{}", repo_root().join("python").display())),
        "expected the repo python/ package on PYTHONPATH:\n{stdout}"
    );
    assert!(
        stdout.contains("GATE=UNSET"),
        "with no credential config on disk, the live gate variable must stay unset:\n{stdout}"
    );

    let combined = format!("{}{}", stdout, String::from_utf8_lossy(&out.stderr));
    assert!(
        !combined.to_lowercase().contains("dapi"),
        "must never print a token-shaped value:\n{combined}"
    );
}

/// Shared fake-`spark` driver for `DatabricksAdapter.load_arrow_table`: a
/// `spark` object built with `object.__new__` (never runs `__init__`, so no
/// `databricks.connect` import and no network), recording every call it
/// receives as a JSON list of tuples on stdout.
fn fake_spark_load_arrow_table_script(explicit_mode: Option<&str>) -> String {
    let mode_call = match explicit_mode {
        Some(m) => format!(r#"adapter.load_arrow_table(ipc_bytes, "cat.sch.tbl", mode="{m}")"#),
        None => r#"adapter.load_arrow_table(ipc_bytes, "cat.sch.tbl")"#.to_string(),
    };
    let python_dir = format!("{:?}", repo_root().join("python").display().to_string());
    format!(
        r#"
import io, json, sys
sys.path.insert(0, {python_dir})
import pyarrow as pa
from smelt.databricks_adapter import DatabricksAdapter

log = []

class FakeWriter:
    def __init__(self, log):
        self.log = log
    def mode(self, m):
        self.log.append(["mode", m])
        return self
    def saveAsTable(self, name):
        self.log.append(["saveAsTable", name])

class FakeDF:
    def __init__(self, log):
        self.write = FakeWriter(log)

class FakeCatalog:
    def __init__(self, log):
        self.log = log
    def tableExists(self, name):
        self.log.append(["tableExists", name])
        return True

class FakeSpark:
    def __init__(self, log):
        self.log = log
        self.catalog = FakeCatalog(log)
    def sql(self, s):
        self.log.append(["sql", s])
    def createDataFrame(self, table):
        self.log.append(["createDataFrame", type(table).__module__ + "." + type(table).__name__])
        return FakeDF(self.log)

adapter = object.__new__(DatabricksAdapter)
adapter.spark = FakeSpark(log)
adapter.host = "fake"

table = pa.table({{"a": [1, 2, 3]}})
sink = pa.BufferOutputStream()
writer = pa.ipc.new_stream(sink, table.schema)
writer.write_table(table)
writer.close()
ipc_bytes = sink.getvalue().to_pybytes()

{mode_call}

print(json.dumps(log))
"#,
        python_dir = python_dir,
    )
}

fn run_python(python: &Path, script: &str) -> Output {
    Command::new(python)
        .args(["-c", script])
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn {}: {e}", python.display()))
}

#[test]
fn load_arrow_table_appends_without_dropping() {
    let Some(python) = dbx_venv_python() else {
        eprintln!("Skipping — build .smelt-dbx-venv via scripts/dbx-dogfood-venv.sh to enable");
        return;
    };
    let script = fake_spark_load_arrow_table_script(Some("append"));
    let out = run_python(&python, &script);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let log: Vec<Vec<String>> =
        serde_json::from_slice(&out.stdout).expect("expected a JSON call log");

    assert!(
        !log.iter().any(|c| c[0] == "sql"),
        "append mode must issue no SQL (no DROP TABLE): {log:?}"
    );
    assert!(
        !log.iter().any(|c| c[0] == "tableExists"),
        "append mode must not probe tableExists: {log:?}"
    );
    assert!(
        log.iter()
            .any(|c| c[0] == "mode" && c.get(1).map(String::as_str) == Some("append")),
        "expected a .mode(\"append\") call: {log:?}"
    );
    assert!(
        log.iter()
            .any(|c| c[0] == "saveAsTable" && c.get(1).map(String::as_str) == Some("cat.sch.tbl")),
        "expected a saveAsTable call: {log:?}"
    );
    assert!(
        log.iter().any(|c| c[0] == "createDataFrame"
            && c.get(1).map(String::as_str) == Some("pandas.DataFrame")),
        "createDataFrame must receive a pandas DataFrame — Databricks Connect's \
         createDataFrame has no pyarrow.Table overload: {log:?}"
    );
}

#[test]
fn load_arrow_table_default_mode_still_replaces() {
    let Some(python) = dbx_venv_python() else {
        eprintln!("Skipping — build .smelt-dbx-venv via scripts/dbx-dogfood-venv.sh to enable");
        return;
    };
    let script = fake_spark_load_arrow_table_script(None);
    let out = run_python(&python, &script);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let log: Vec<Vec<String>> =
        serde_json::from_slice(&out.stdout).expect("expected a JSON call log");

    assert!(
        log.iter().any(|c| c[0] == "tableExists"),
        "default mode must still probe tableExists: {log:?}"
    );
    assert!(
        log.iter()
            .any(|c| c[0] == "sql" && c.get(1).is_some_and(|s| s.contains("DROP TABLE"))),
        "default mode must still drop the table first: {log:?}"
    );
    assert!(
        !log.iter().any(|c| c[0] == "mode"),
        "default mode must not call .mode(...) — unchanged two-positional-argument behaviour: {log:?}"
    );
    assert!(
        log.iter()
            .any(|c| c[0] == "saveAsTable" && c.get(1).map(String::as_str) == Some("cat.sch.tbl")),
        "expected a saveAsTable call: {log:?}"
    );
    assert!(
        log.iter().any(|c| c[0] == "createDataFrame"
            && c.get(1).map(String::as_str) == Some("pandas.DataFrame")),
        "createDataFrame must receive a pandas DataFrame — Databricks Connect's \
         createDataFrame has no pyarrow.Table overload: {log:?}"
    );
}

/// Loads `dbx-dogfood-loader.py` as a module with `smelt.databricks_adapter`
/// faked out of `sys.modules` before import, so the loader's own control
/// flow (mode selection, DDL statement list) can be exercised with no
/// `pyarrow`/`databricks-connect` dependency and no network.
fn run_loader_with_fake_adapter(driver_tail: &str) -> Output {
    let loader_path = format!("{:?}", loader_py().display().to_string());
    let script = format!(
        r#"
import sys, types, importlib.util, json, os

log = []

class FakeAdapter:
    def __init__(self, host, catalog=None, token=None):
        log.append(["init", host, catalog, token])
    def table_exists(self, name):
        log.append(["table_exists", name])
        return False
    def execute_sql(self, sql):
        log.append(["execute_sql", sql])
        class _Col:
            def __init__(self, v):
                self._v = v
            def as_py(self):
                return self._v
        class _Result:
            def column(self, name):
                return [_Col(0)]
        return _Result()
    def execute_sql_no_result(self, sql):
        log.append(["execute_sql_no_result", sql])
    def load_arrow_table(self, ipc_bytes, full_table_name, mode="overwrite"):
        log.append(["load_arrow_table", full_table_name, mode])
    def close(self):
        log.append(["close"])

fake_mod = types.ModuleType("smelt.databricks_adapter")
fake_mod.DatabricksAdapter = FakeAdapter
fake_pkg = types.ModuleType("smelt")
fake_pkg.databricks_adapter = fake_mod
sys.modules["smelt"] = fake_pkg
sys.modules["smelt.databricks_adapter"] = fake_mod

os.environ["SMELT_DBX_HOST"] = "fake-host"
os.environ.pop("SMELT_DBX_TOKEN", None)

spec = importlib.util.spec_from_file_location("dbx_loader", {loader_path})
loader_mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(loader_mod)

{driver_tail}

print(json.dumps(log))
"#,
        loader_path = loader_path,
    );
    Command::new("python3")
        .args(["-c", &script])
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn python3: {e}"))
}

/// The commands driven here (`cmd_execute`, `cmd_apply_ddl`) print their own
/// user-facing progress lines to stdout ahead of the driver's final
/// `json.dumps(log)` line, so the call log is the LAST line, not the whole
/// stream.
fn last_line_json(out: &Output) -> Vec<Vec<serde_json::Value>> {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let last = stdout
        .lines()
        .next_back()
        .unwrap_or_else(|| panic!("expected at least one stdout line:\n{stdout}"));
    serde_json::from_str(last)
        .unwrap_or_else(|e| panic!("expected a JSON call log on the last line: {e}\n{stdout}"))
}

#[test]
fn loader_execute_path_appends_rather_than_replacing() {
    let out = run_loader_with_fake_adapter("loader_mod.cmd_execute(\"2026-08-06\")");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let log = last_line_json(&out);

    let load_calls: Vec<_> = log.iter().filter(|c| c[0] == "load_arrow_table").collect();
    assert_eq!(
        load_calls.len(),
        2,
        "expected exactly two load_arrow_table calls (events, arrival): {log:?}"
    );
    for call in &load_calls {
        assert_eq!(
            call[2], "append",
            "every load_arrow_table call in the execute path must use append mode: {log:?}"
        );
    }
}

#[test]
fn apply_ddl_executes_exactly_the_emitted_ddl() {
    let out = run_loader_with_fake_adapter("loader_mod.cmd_apply_ddl()");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let log = last_line_json(&out);
    let executed: Vec<String> = log
        .iter()
        .filter(|c| c[0] == "execute_sql_no_result")
        .map(|c| c[1].as_str().unwrap().to_string())
        .collect();

    let emit_out = run_loader(&["--emit-ddl"]);
    assert!(emit_out.status.success());
    let emitted = String::from_utf8_lossy(&emit_out.stdout).to_string();

    assert!(
        !executed.is_empty(),
        "expected --apply-ddl to execute statements"
    );
    for stmt in &executed {
        assert!(
            emitted.contains(stmt.as_str()),
            "executed statement not found verbatim in --emit-ddl output:\n{stmt}\n---\n{emitted}"
        );
    }
}

#[test]
fn apply_ddl_needs_no_network_to_emit() {
    let out = Command::new("python3")
        .arg(loader_py())
        .arg("--emit-ddl")
        .current_dir(repo_root())
        .env_remove("SMELT_DBX_HOST")
        .env_remove("SMELT_DBX_TOKEN")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn dbx-dogfood-loader.py: {e}"));
    assert!(
        out.status.success(),
        "--emit-ddl must need no credential or network: stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

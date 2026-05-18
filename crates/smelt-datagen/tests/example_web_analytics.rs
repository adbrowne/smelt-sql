//! Integration tests for the `examples/web_analytics` datagen configuration.
//!
//! These tests validate:
//!
//! 1. Smoke test at `--scale-factor 0.01`: the `smelt-datagen` binary exits 0,
//!    Parquet files exist and have rows, hive partitions are present, and FK
//!    values in the first 100 event rows satisfy referential-integrity
//!    constraints (`device_id > 0`, `user_id` null-or-positive).
//!
//! 2. Pool-snapshot test: build the `device_user` linked pool directly via
//!    `LinkedPool::new` and verify:
//!    - The anonymous-pool fraction (entries with `user_id = Null`) is ≈ 19.2%
//!      (±3pp), the correct value given weights [0.25, 0.60, 0.10, 0.05] and
//!      emits [1, 1, 3, 3]:  avg_emit = 1.30; anon_frac = 0.25/1.30 ≈ 19.2%.
//!    - Building the pool twice with the same explicit seed (1337) produces
//!      identical null counts, validating the spec's determinism guarantee.
//!
//! 3. Source-loading test: after running `smelt-datagen`, execute
//!    `setup_sources.sql` against a fresh DuckDB file and verify that
//!    `raw.users`, `raw.devices`, and `raw.events` each contain > 0 rows.
//!
//! 4. Bronze-view test: extend the source-loading test by additionally
//!    running `smelt build` and verifying the `raw_events` view materializes
//!    with a row count equal to the total event rows generated.
//!
//! 5. Parse-function compile test: call `smelt-parser` directly on
//!    `functions/parse_event_payload.sql` and assert the declared signature
//!    matches `parse_event_payload(payload_json: Expr<Text>) ->
//!    Expr<Struct<{event_name: Text, platform: Text, url: Text}>>`. This is
//!    the lightweight equivalent of the spec's "load the function via
//!    smelt-db's function-loading path" check — it pins the signature shape
//!    so a future drift in the function body or types is caught immediately.
//!
//! 6. End-to-end build test: run the full datagen → setup_sources →
//!    `smelt build` pipeline and assert `silver_events_parsed` materializes
//!    with the expected row count and that JSON-extracted `event_name` /
//!    `platform` / `url` columns are non-null for at least one row.
//!
//! The binary-invocation tests write to a temp directory (NOT
//! `examples/web_analytics/data/`) to avoid file-system collisions when tests
//! run in parallel.

use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::record::RowAccessor;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// Path to the compiled `smelt-datagen` binary, injected by Cargo at
/// test-link time via the `CARGO_BIN_EXE_*` env var convention.
fn datagen_bin() -> &'static str {
    env!("CARGO_BIN_EXE_smelt-datagen")
}

/// Absolute path to the repo root (two levels above the crate root).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/ dir")
        .parent()
        .expect("repo root")
        .to_owned()
}

/// Rewrite all dataset `output:` paths in a datagen YAML config so that they
/// point at absolute paths under `output_base`, then write the modified YAML
/// to `dest_path`.
///
/// This lets us redirect datagen output to a temp dir without touching the
/// checked-in config.  The source config uses relative paths like
/// `data/users`; we keep the leaf component and root it under `output_base`.
fn rewrite_outputs(src_yaml: &Path, dest_path: &Path, output_base: &Path) {
    let content =
        fs::read_to_string(src_yaml).unwrap_or_else(|e| panic!("cannot read {src_yaml:?}: {e}"));

    let mut out = String::with_capacity(content.len() + 256);
    for line in content.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("output:") {
            // Extract the last path component (leaf name) from `output: data/foo`
            let val = rest.trim().trim_matches('"');
            let leaf = val.split('/').next_back().unwrap_or("dataset");
            let abs = output_base.join(leaf);
            // Preserve original indentation
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            out.push_str(&format!("{}output: {}\n", indent, abs.display()));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    fs::write(dest_path, &out).unwrap_or_else(|e| panic!("cannot write {dest_path:?}: {e}"));
}

/// Count the rows in a single Parquet file by reading its footer metadata.
fn parquet_row_count(path: &Path) -> u64 {
    let file = fs::File::open(path).unwrap_or_else(|e| panic!("open {path:?}: {e}"));
    let reader =
        SerializedFileReader::new(file).unwrap_or_else(|e| panic!("parse parquet {path:?}: {e}"));
    reader.metadata().file_metadata().num_rows() as u64
}

/// Recursively find all `data.parquet` files under `dir`.
fn find_parquet_files(dir: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    if !dir.exists() {
        return result;
    }
    fn walk(dir: &Path, result: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, result);
            } else if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n == "data.parquet")
                .unwrap_or(false)
            {
                result.push(path);
            }
        }
    }
    walk(dir, &mut result);
    result
}

/// Run smelt-datagen against a config; return (success, combined stdout+stderr).
fn run_datagen(config_path: &Path, scale_factor: f64) -> (bool, String) {
    let output = Command::new(datagen_bin())
        .arg("--config")
        .arg(config_path)
        .arg("--scale-factor")
        .arg(scale_factor.to_string())
        .output()
        .expect("failed to invoke smelt-datagen");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.success(), format!("{stdout}{stderr}"))
}

/// Recursively list files under `dir` for diagnostic output.
fn list_dir(dir: &Path) -> String {
    let mut out = String::new();
    fn walk(dir: &Path, depth: usize, out: &mut String) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            out.push_str(&"  ".repeat(depth));
            out.push_str(&name);
            out.push('\n');
            if path.is_dir() {
                walk(&path, depth + 1, out);
            }
        }
    }
    walk(dir, 0, &mut out);
    out
}

/// Smoke test: at `--scale-factor 0.01` the config exits 0 and produces
/// non-empty Parquet files for `users`, `devices`, and `events` (partitioned).
/// Also verifies that `device_id` and `user_id` values in the first 100
/// event rows satisfy referential-integrity constraints: `device_id > 0`,
/// `user_id` is either null or > 0.
#[test]
fn test_datagen_runs_at_small_scale() {
    let tmp = TempDir::new().expect("tempdir");
    let tmp_path = tmp.path();

    let src_config = repo_root().join("examples/web_analytics/datagen.yaml");
    let dest_config = tmp_path.join("datagen.yaml");
    rewrite_outputs(&src_config, &dest_config, tmp_path);

    let (ok, combined) = run_datagen(&dest_config, 0.01);
    assert!(
        ok,
        "smelt-datagen exited non-zero at scale-factor 0.01:\n{combined}"
    );

    // 1. users parquet exists and has rows
    let users_pq = tmp_path.join("users/data.parquet");
    assert!(
        users_pq.exists(),
        "users/data.parquet not found; tmp contents:\n{}",
        list_dir(tmp_path)
    );
    assert!(parquet_row_count(&users_pq) > 0, "users parquet is empty");

    // 2. devices parquet exists and has rows
    let devices_pq = tmp_path.join("devices/data.parquet");
    assert!(
        devices_pq.exists(),
        "devices/data.parquet not found; tmp contents:\n{}",
        list_dir(tmp_path)
    );
    assert!(
        parquet_row_count(&devices_pq) > 0,
        "devices parquet is empty"
    );

    // 3. events: at least one hive-partition file exists
    let events_dir = tmp_path.join("events");
    let event_files = find_parquet_files(&events_dir);
    assert!(
        !event_files.is_empty(),
        "no event parquet partitions found under {events_dir:?}; tmp contents:\n{}",
        list_dir(tmp_path)
    );

    // 4. Partition directories follow hive naming (event_date=YYYY-MM-DD)
    {
        let has_hive = fs::read_dir(&events_dir)
            .expect("read events dir")
            .flatten()
            .any(|e| {
                e.path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("event_date="))
                    .unwrap_or(false)
            });
        assert!(has_hive, "no event_date=* partition directories found");
    }

    // 5. Total event row count > 0
    let total_event_rows: u64 = event_files.iter().map(|p| parquet_row_count(p)).sum();
    assert!(total_event_rows > 0, "total event row count is 0");

    // 6. Spot-check the first 100 event rows: device_id > 0, user_id null-or-positive.
    //    The Row API returns Field::Int(v) for non-null Int32, and returns Err
    //    (not Ok(0)) for Field::Null — so get_int().is_err() reliably detects null.
    {
        let first = &event_files[0];
        let file = fs::File::open(first).expect("open event parquet");
        let reader = SerializedFileReader::new(file).expect("parse event parquet");
        let schema = reader.metadata().file_metadata().schema();
        let fields = schema.get_fields();
        let device_idx = fields.iter().position(|f| f.name() == "device_id");
        let user_idx = fields.iter().position(|f| f.name() == "user_id");

        if let (Some(di), Some(ui)) = (device_idx, user_idx) {
            let mut checked = 0usize;
            for row_result in reader.get_row_iter(None).expect("row iter") {
                let row = row_result.expect("row");
                let device_id = row.get_int(di).expect("device_id must be non-null Int32");
                assert!(
                    device_id > 0,
                    "device_id must be a positive FK integer, got {device_id}"
                );
                // user_id is nullable (anonymous pool entries write null)
                if let Ok(uid) = row.get_int(ui) {
                    assert!(uid > 0, "user_id when non-null must be > 0, got {uid}");
                }
                checked += 1;
                if checked >= 100 {
                    break;
                }
            }
            assert!(checked > 0, "could not read any event rows from {first:?}");
        }
    }
}

/// Pool-snapshot test: build the `device_user` linked pool directly using the
/// public `LinkedPool::new` API and verify the anonymous fraction and
/// deterministic reproducibility.
///
/// With weights \[0.25, 0.60, 0.10, 0.05\] and emits \[1, 1, 3, 3\] the
/// expected anonymous pool-entry fraction is:
/// avg_emit = 0.25·1 + 0.60·1 + 0.10·3 + 0.05·3 = 1.30;
/// anon_frac = 0.25·1 / 1.30 ≈ 19.2%.
///
/// The anonymous shape emits `user_id = Null`; every other shape emits a
/// non-null FK integer, so counting Null entries gives the anonymous fraction.
/// The tolerance is ±3pp (plan spec for pool-snapshot tests).
///
/// A second build with the same seed (1337) must produce an identical null
/// count, validating the spec's determinism guarantee.
///
/// Limitation: distinguishing single-owner (60%) from shared-device (10%) from
/// multi-device-user (5%) requires per-entry shape labels that `LinkedPool`
/// does not expose. This test covers only the anonymous fraction and
/// determinism; full category breakdown coverage lives in the linked_choice
/// plan's own tests.
#[test]
fn test_pool_snapshot_anonymous_fraction_and_determinism() {
    use smelt_datagen::config::{DatagenConfig, FkCounts};
    use smelt_datagen::generic::{GenericValue, LinkedPool};

    // Load the real datagen config so the pool parameters are always in sync
    // with datagen.yaml (no hard-coded duplicates).
    let config_path = repo_root().join("examples/web_analytics/datagen.yaml");
    let config_str = fs::read_to_string(&config_path)
        .unwrap_or_else(|e| panic!("cannot read {config_path:?}: {e}"));
    let config: DatagenConfig = serde_yaml::from_str(&config_str)
        .unwrap_or_else(|e| panic!("cannot parse datagen.yaml: {e}"));

    // Find the `events` dataset (the one with linked_pools).
    let events_ds = config
        .datasets
        .iter()
        .find(|d| d.name == "events")
        .expect("events dataset not found in datagen.yaml");

    let pools = events_ds
        .linked_pools
        .as_deref()
        .expect("events dataset has no linked_pools");

    let pool_cfg = pools
        .iter()
        .find(|p| p.name == "device_user")
        .expect("device_user pool not found in events.linked_pools");

    // Dimension sizes at scale_factor 1.0 (pool size is absolute per spec
    // invariant 6 — it does not scale with --scale-factor, so fk_counts must
    // reflect the full dimension sizes).
    let mut fk_counts: FkCounts = FkCounts::new();
    for ds in &config.datasets {
        fk_counts.insert(ds.name.clone(), ds.num_rows);
    }

    // The pool has an explicit seed: 1337 (from datagen.yaml).
    let seed = pool_cfg
        .seed
        .expect("device_user pool must have an explicit seed");

    // Helper: count how many pool entries have user_id = Null.
    // The field_index in LinkedPool maps field name → column position in each tuple.
    let count_nulls = |pool: &LinkedPool| -> usize {
        let user_id_idx = *pool
            .field_index
            .get("user_id")
            .expect("user_id field not found in pool field_index");
        pool.rows
            .iter()
            .filter(|row| matches!(row[user_id_idx], GenericValue::Null))
            .count()
    };

    // Build the pool once and measure the anonymous fraction.
    let pool1 = LinkedPool::new(seed, pool_cfg, &fk_counts);
    let pool_size = pool1.rows.len();
    assert_eq!(
        pool_size, pool_cfg.pool_size,
        "pool should contain exactly pool_size={} entries",
        pool_cfg.pool_size
    );

    let null_count1 = count_nulls(&pool1);
    let null_pct1 = null_count1 as f64 / pool_size as f64 * 100.0;

    // Expected anonymous fraction: (weight_anon * emit_anon) / avg_emit_per_draw
    // = (0.25 * 1) / (0.25*1 + 0.60*1 + 0.10*3 + 0.05*3)
    // = 0.25 / 1.30 ≈ 19.23%
    let expected_pct = 19.23_f64;
    let tolerance = 3.0_f64; // ±3pp as per plan §pool-snapshot test spec

    assert!(
        (null_pct1 - expected_pct).abs() <= tolerance,
        "anonymous (null user_id) pool fraction = {null_pct1:.2}% \
         (expected {expected_pct:.2}% ±{tolerance}pp; \
         pool_size={pool_size}, null_count={null_count1})"
    );

    // Build the pool a second time with the same seed to verify determinism.
    let pool2 = LinkedPool::new(seed, pool_cfg, &fk_counts);
    let null_count2 = count_nulls(&pool2);

    assert_eq!(
        null_count1, null_count2,
        "LinkedPool with seed={seed} must be deterministic: \
         first build null_count={null_count1}, second build null_count={null_count2}"
    );
    assert_eq!(
        pool2.rows.len(),
        pool_size,
        "second pool build should have the same number of rows"
    );
}

// ---------------------------------------------------------------------------
// Helpers shared by the source-loading and bronze-view tests
// ---------------------------------------------------------------------------

/// Path to the compiled `smelt` CLI binary.
///
/// The smelt binary lives in the same target directory as `smelt-datagen`.
/// `CARGO_BIN_EXE_smelt-datagen` is set by Cargo for all integration tests in
/// this package; we strip the binary name and substitute "smelt" to locate the
/// sibling binary built from `crates/smelt-cli`.
fn smelt_bin() -> PathBuf {
    let datagen = PathBuf::from(env!("CARGO_BIN_EXE_smelt-datagen"));
    datagen
        .parent()
        .expect("datagen binary has a parent directory")
        .join("smelt")
}

/// Copy a source file from the project tree into `dest`, creating parent
/// directories as needed.
fn copy_file(src: &Path, dest: &Path) {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|e| panic!("mkdir {parent:?}: {e}"));
    }
    fs::copy(src, dest).unwrap_or_else(|e| panic!("copy {src:?} → {dest:?}: {e}"));
}

/// Recursively copy a directory tree from `src` into `dest`.
fn copy_dir_all(src: &Path, dest: &Path) {
    fs::create_dir_all(dest).unwrap_or_else(|e| panic!("mkdir {dest:?}: {e}"));
    for entry in fs::read_dir(src).unwrap_or_else(|e| panic!("readdir {src:?}: {e}")) {
        let entry = entry.expect("dir entry");
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            copy_dir_all(&from, &to);
        } else {
            copy_file(&from, &to);
        }
    }
}

/// Rewrite `setup_sources.sql` so the relative `data/` prefix in
/// `read_parquet(...)` calls points to the absolute paths where
/// `smelt-datagen` actually wrote the Parquet files.
///
/// `setup_sources.sql` uses paths like `'data/users/data.parquet'`.
/// `rewrite_outputs` strips the `data/` component and roots datasets directly
/// under `output_base` (e.g. `<tmp>/users/data.parquet`), so `setup_sources.sql`
/// must map `'data/` → `'<output_base>/` to match.
///
/// The single-quote is included in the substitution to anchor it to SQL string
/// literals and avoid false positives in comments.
fn rewrite_setup_sources_sql(src: &Path, dest: &Path, output_base: &Path) {
    let content = fs::read_to_string(src).unwrap_or_else(|e| panic!("read {src:?}: {e}"));
    // Map 'data/<leaf> → '<output_base>/<leaf> (rewrite_outputs strips 'data/' prefix)
    let rewritten = content.replace("'data/", &format!("'{}/", output_base.display()));
    fs::write(dest, &rewritten).unwrap_or_else(|e| panic!("write {dest:?}: {e}"));
}

// ---------------------------------------------------------------------------
// Test 3: setup_sources.sql populates raw.users / raw.devices / raw.events
// ---------------------------------------------------------------------------

/// Run `smelt-datagen` at `--scale-factor 0.01` into a temp dir, then execute
/// `setup_sources.sql` against a fresh DuckDB file, and verify that the three
/// source tables (`raw.users`, `raw.devices`, `raw.events`) each contain at
/// least one row.
#[test]
fn test_setup_sources_sql_runs() {
    let tmp = TempDir::new().expect("tempdir");
    let tmp_path = tmp.path();

    // --- Step 1: run smelt-datagen to produce Parquet files in tmp_path ---
    let src_config = repo_root().join("examples/web_analytics/datagen.yaml");
    let dest_config = tmp_path.join("datagen.yaml");
    rewrite_outputs(&src_config, &dest_config, tmp_path);

    let (ok, combined) = run_datagen(&dest_config, 0.01);
    assert!(ok, "smelt-datagen failed at scale-factor 0.01:\n{combined}");

    // --- Step 2: rewrite setup_sources.sql with absolute data/ paths ---
    let src_sql = repo_root().join("examples/web_analytics/setup_sources.sql");
    let dest_sql = tmp_path.join("setup_sources.sql");
    rewrite_setup_sources_sql(&src_sql, &dest_sql, tmp_path);

    // --- Step 3: execute the rewritten SQL against a fresh DuckDB file ---
    let db_path = tmp_path.join("target/dev.duckdb");
    fs::create_dir_all(db_path.parent().unwrap()).expect("mkdir target/");

    let conn = duckdb::Connection::open(&db_path)
        .unwrap_or_else(|e| panic!("open duckdb at {db_path:?}: {e}"));

    let sql = fs::read_to_string(&dest_sql).unwrap_or_else(|e| panic!("read {dest_sql:?}: {e}"));
    conn.execute_batch(&sql)
        .unwrap_or_else(|e| panic!("execute setup_sources.sql: {e}\nSQL:\n{sql}"));

    // --- Step 4: assert each source table has > 0 rows ---
    for table in &["raw.users", "raw.devices", "raw.events"] {
        let count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap_or_else(|e| panic!("SELECT COUNT(*) FROM {table}: {e}"));
        assert!(
            count > 0,
            "{table} has 0 rows after setup_sources.sql executed"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 4: bronze/raw_events view materializes via `smelt build`
// ---------------------------------------------------------------------------

/// Extend the source-loading test by additionally running `smelt build` and
/// verifying the `raw_events` view materializes with a non-zero row count.
///
/// The workspace is cloned into a temp dir so the build artifacts (DuckDB
/// file, `.smelt/` schema cache) never land in the checked-in source tree.
/// `smelt build` is invoked with `current_dir` set to the temp workspace, so
/// that the relative `database: target/dev.duckdb` path in `smelt.yml` points
/// to the temp dir's `target/dev.duckdb` (the same file populated by
/// `setup_sources.sql`).
#[test]
fn test_bronze_raw_events_view() {
    let tmp = TempDir::new().expect("tempdir");
    let tmp_path = tmp.path();

    // --- Step 1: clone the web_analytics project tree into tmp_path ---
    let project_src = repo_root().join("examples/web_analytics");
    copy_dir_all(&project_src, tmp_path);

    // --- Step 2: run smelt-datagen with rewritten outputs into tmp_path ---
    let src_config = tmp_path.join("datagen.yaml");
    let dest_config = tmp_path.join("datagen_rewritten.yaml");
    rewrite_outputs(&src_config, &dest_config, tmp_path);

    let (ok, combined) = run_datagen(&dest_config, 0.01);
    assert!(ok, "smelt-datagen failed at scale-factor 0.01:\n{combined}");

    // --- Step 3: rewrite setup_sources.sql with absolute paths, execute ---
    let setup_sql_src = tmp_path.join("setup_sources.sql");
    let setup_sql_abs = tmp_path.join("setup_sources_abs.sql");
    rewrite_setup_sources_sql(&setup_sql_src, &setup_sql_abs, tmp_path);

    let db_path = tmp_path.join("target/dev.duckdb");
    fs::create_dir_all(db_path.parent().unwrap()).expect("mkdir target/");

    let conn = duckdb::Connection::open(&db_path).unwrap_or_else(|e| panic!("open duckdb: {e}"));

    let sql = fs::read_to_string(&setup_sql_abs)
        .unwrap_or_else(|e| panic!("read setup_sources_abs.sql: {e}"));
    conn.execute_batch(&sql)
        .unwrap_or_else(|e| panic!("execute setup_sources_abs.sql: {e}\nSQL:\n{sql}"));

    // Count events rows for later comparison (connection must be closed before
    // smelt build opens the same file via its own DuckDB connection).
    let events_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM raw.events", [], |row| row.get(0))
        .expect("count raw.events");
    assert!(events_count > 0, "raw.events has 0 rows before smelt build");

    // Close connection so smelt build can open the file exclusively.
    drop(conn);

    // --- Step 4: run `smelt build` from the temp workspace directory ---
    let smelt = smelt_bin();
    assert!(
        smelt.exists(),
        "smelt binary not found at {smelt:?}; run `cargo build -p smelt-cli` first"
    );

    let build_out = Command::new(&smelt)
        .args(["build", "--target", "dev"])
        .current_dir(tmp_path)
        .env("RUST_LOG", "warn")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));

    assert!(
        build_out.status.success(),
        "`smelt build` exited {:?}\nstdout:\n{}\nstderr:\n{}",
        build_out.status,
        String::from_utf8_lossy(&build_out.stdout),
        String::from_utf8_lossy(&build_out.stderr),
    );

    // --- Step 5: verify bronze_raw_events view has the expected row count ---
    //
    // smelt derives the DuckDB view/table name from the model's address segments
    // joined with `_`.  The file `models/bronze/raw_events.sql` has segments
    // `["bronze", "raw_events"]`, so the materialized name is `bronze_raw_events`.
    let conn2 = duckdb::Connection::open(&db_path).unwrap_or_else(|e| panic!("reopen duckdb: {e}"));

    let view_count: i64 = conn2
        .query_row("SELECT COUNT(*) FROM main.bronze_raw_events", [], |row| {
            row.get(0)
        })
        .unwrap_or_else(|e| panic!("SELECT COUNT(*) FROM main.bronze_raw_events: {e}"));

    assert_eq!(
        view_count, events_count,
        "bronze_raw_events view row count ({view_count}) should equal raw.events row count ({events_count})"
    );
}

// ---------------------------------------------------------------------------
// Test 5: parse_event_payload smelt function signature parses correctly
// ---------------------------------------------------------------------------

/// Verify the `parse_event_payload` function file parses with no errors and
/// declares the expected signature shape. This is the lightweight equivalent
/// of "load the function via smelt-db's function-loading path" — we call
/// `smelt-parser` directly on the file's bytes, which gives a deterministic,
/// fast contract check on the declared name, parameter, and return type.
///
/// If a future change to the function body or signature drifts from this
/// shape (e.g. accidentally drops a struct field), this test catches the
/// drift before downstream silver/gold models inherit a wrong column.
#[test]
fn test_parse_event_payload_function_compiles() {
    use smelt_parser::ast::{File, Param, SmeltDefine};
    use smelt_parser::parse;

    let fn_path = repo_root().join("examples/web_analytics/functions/parse_event_payload.sql");
    let source = fs::read_to_string(&fn_path).unwrap_or_else(|e| panic!("read {fn_path:?}: {e}"));

    let parsed = parse(&source);
    assert!(
        parsed.errors.is_empty(),
        "parse errors in parse_event_payload.sql:\n{:?}",
        parsed.errors
    );

    let file = File::cast(parsed.syntax()).expect("syntax root is a FILE");
    let defines: Vec<SmeltDefine> = file.defines().collect();
    assert_eq!(
        defines.len(),
        1,
        "expected exactly one smelt.define in parse_event_payload.sql"
    );

    let def = &defines[0];
    assert_eq!(
        def.name().as_deref(),
        Some("parse_event_payload"),
        "function name should be parse_event_payload"
    );

    // One parameter: payload_json: Expr<Text>
    let params: Vec<Param> = def
        .param_list()
        .expect("function has a param list")
        .params()
        .collect();
    assert_eq!(params.len(), 1, "expected exactly one parameter");
    assert_eq!(
        params[0].name().as_deref(),
        Some("payload_json"),
        "parameter name should be payload_json"
    );
    let p0_type: String = params[0]
        .type_ref()
        .expect("param has type")
        .text()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert_eq!(p0_type, "Expr<Text>", "parameter type should be Expr<Text>");

    // Return type: Expr<Struct<{event_name: Text, platform: Text, url: Text}>>
    let ret: String = def
        .return_type()
        .expect("function declares a return type")
        .text()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert_eq!(
        ret, "Expr<Struct<{event_name:Text,platform:Text,url:Text}>>",
        "return type should be Expr<Struct<{{event_name: Text, platform: Text, url: Text}}>>"
    );

    assert!(
        def.body().is_some(),
        "function declaration must have a body"
    );
}

// ---------------------------------------------------------------------------
// Test 6: sessionize smelt function signature parses correctly
// ---------------------------------------------------------------------------

/// Verify the `sessionize` function file parses with no errors and declares
/// the expected signature shape. Mirrors `test_parse_event_payload_function_compiles`
/// but for the sessionize function, which uses window functions to assign a
/// `session_seq` value partitioned by user/device and ordered by timestamp,
/// with a session boundary on inactivity gap or platform change.
///
/// Checks: (i) no parse errors; (ii) exactly five parameters with the correct
/// names and types; (iii) the function body is present.
#[test]
fn test_sessionize_function_compiles() {
    use smelt_parser::ast::{File, Param, SmeltDefine};
    use smelt_parser::parse;

    let fn_path = repo_root().join("examples/web_analytics/functions/sessionize.sql");
    let source = fs::read_to_string(&fn_path).unwrap_or_else(|e| panic!("read {fn_path:?}: {e}"));

    let parsed = parse(&source);
    assert!(
        parsed.errors.is_empty(),
        "parse errors in sessionize.sql:\n{:?}",
        parsed.errors
    );

    let file = File::cast(parsed.syntax()).expect("syntax root is a FILE");
    let defines: Vec<SmeltDefine> = file.defines().collect();
    assert_eq!(
        defines.len(),
        1,
        "expected exactly one smelt.define in sessionize.sql"
    );

    let def = &defines[0];
    assert_eq!(
        def.name().as_deref(),
        Some("sessionize"),
        "function name should be sessionize"
    );

    // Five parameters: source, partition_col, ts_col, platform_col, gap
    let params: Vec<Param> = def
        .param_list()
        .expect("function has a param list")
        .params()
        .collect();
    assert_eq!(params.len(), 5, "expected exactly five parameters");

    // Parameter 0: source: TableExpr
    assert_eq!(
        params[0].name().as_deref(),
        Some("source"),
        "first parameter name should be source"
    );
    let p0_type: String = params[0]
        .type_ref()
        .expect("param 0 has type")
        .text()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert_eq!(
        p0_type, "TableExpr",
        "first parameter type should be TableExpr"
    );

    // Parameter 1: partition_col: Expr<Integer>
    assert_eq!(
        params[1].name().as_deref(),
        Some("partition_col"),
        "second parameter name should be partition_col"
    );
    let p1_type: String = params[1]
        .type_ref()
        .expect("param 1 has type")
        .text()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert_eq!(
        p1_type, "Expr<Integer>",
        "second parameter type should be Expr<Integer>"
    );

    // Parameter 2: ts_col: Expr<Timestamp>
    assert_eq!(
        params[2].name().as_deref(),
        Some("ts_col"),
        "third parameter name should be ts_col"
    );
    let p2_type: String = params[2]
        .type_ref()
        .expect("param 2 has type")
        .text()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert_eq!(
        p2_type, "Expr<Timestamp>",
        "third parameter type should be Expr<Timestamp>"
    );

    // Parameter 3: platform_col: Expr<Text>
    assert_eq!(
        params[3].name().as_deref(),
        Some("platform_col"),
        "fourth parameter name should be platform_col"
    );
    let p3_type: String = params[3]
        .type_ref()
        .expect("param 3 has type")
        .text()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert_eq!(
        p3_type, "Expr<Text>",
        "fourth parameter type should be Expr<Text>"
    );

    // Parameter 4: gap: Expr<Interval> (with default)
    assert_eq!(
        params[4].name().as_deref(),
        Some("gap"),
        "fifth parameter name should be gap"
    );
    let p4_type: String = params[4]
        .type_ref()
        .expect("param 4 has type")
        .text()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert_eq!(
        p4_type, "Expr<Interval>",
        "fifth parameter type should be Expr<Interval>"
    );
    assert!(
        params[4].default_value().is_some(),
        "gap parameter must have a default value (INTERVAL '30 minutes')"
    );

    // Return type: TableExpr
    let ret: String = def
        .return_type()
        .expect("function declares a return type")
        .text()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert_eq!(ret, "TableExpr", "return type should be TableExpr");

    assert!(
        def.body().is_some(),
        "function declaration must have a body"
    );
}

// ---------------------------------------------------------------------------
// Test 7: end-to-end build materializes silver/events_parsed
// ---------------------------------------------------------------------------

/// Full pipeline test: run `smelt-datagen`, execute `setup_sources.sql`, invoke
/// `smelt build`, then verify that `main.silver_events_parsed` has the same row
/// count as `raw.events` and that the JSON-extracted `event_name` / `platform`
/// / `url` columns are non-null for at least one row.
///
/// `models/silver/events_parsed.sql` address segments are `["silver",
/// "events_parsed"]`, so smelt materializes the view as `silver_events_parsed`
/// in the `main` schema — analogous to how `models/bronze/raw_events.sql`
/// materializes as `bronze_raw_events`.
#[test]
fn test_end_to_end_smelt_build() {
    let tmp = TempDir::new().expect("tempdir");
    let tmp_path = tmp.path();

    // --- Step 1: clone the web_analytics project tree into tmp_path ---
    let project_src = repo_root().join("examples/web_analytics");
    copy_dir_all(&project_src, tmp_path);

    // --- Step 2: run smelt-datagen with rewritten outputs into tmp_path ---
    let src_config = tmp_path.join("datagen.yaml");
    let dest_config = tmp_path.join("datagen_rewritten.yaml");
    rewrite_outputs(&src_config, &dest_config, tmp_path);

    let (ok, combined) = run_datagen(&dest_config, 0.01);
    assert!(ok, "smelt-datagen failed at scale-factor 0.01:\n{combined}");

    // --- Step 3: rewrite setup_sources.sql with absolute paths, execute ---
    let setup_sql_src = tmp_path.join("setup_sources.sql");
    let setup_sql_abs = tmp_path.join("setup_sources_abs.sql");
    rewrite_setup_sources_sql(&setup_sql_src, &setup_sql_abs, tmp_path);

    let db_path = tmp_path.join("target/dev.duckdb");
    fs::create_dir_all(db_path.parent().unwrap()).expect("mkdir target/");

    let conn = duckdb::Connection::open(&db_path).unwrap_or_else(|e| panic!("open duckdb: {e}"));

    let sql = fs::read_to_string(&setup_sql_abs)
        .unwrap_or_else(|e| panic!("read setup_sources_abs.sql: {e}"));
    conn.execute_batch(&sql)
        .unwrap_or_else(|e| panic!("execute setup_sources_abs.sql: {e}\nSQL:\n{sql}"));

    // Capture total event row count for later comparison.
    let events_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM raw.events", [], |row| row.get(0))
        .expect("count raw.events");
    assert!(events_count > 0, "raw.events has 0 rows before smelt build");

    // Close connection so smelt build can open the file exclusively.
    drop(conn);

    // --- Step 4: run `smelt build` from the temp workspace directory ---
    let smelt = smelt_bin();
    assert!(
        smelt.exists(),
        "smelt binary not found at {smelt:?}; run `cargo build -p smelt-cli` first"
    );

    let build_out = Command::new(&smelt)
        .args(["build", "--target", "dev"])
        .current_dir(tmp_path)
        .env("RUST_LOG", "warn")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));

    assert!(
        build_out.status.success(),
        "`smelt build` exited {:?}\nstdout:\n{}\nstderr:\n{}",
        build_out.status,
        String::from_utf8_lossy(&build_out.stdout),
        String::from_utf8_lossy(&build_out.stderr),
    );

    // --- Step 5: verify silver_events_parsed row count matches events ---
    let conn2 = duckdb::Connection::open(&db_path).unwrap_or_else(|e| panic!("reopen duckdb: {e}"));

    let silver_count: i64 = conn2
        .query_row(
            "SELECT COUNT(*) FROM main.silver_events_parsed",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|e| panic!("SELECT COUNT(*) FROM main.silver_events_parsed: {e}"));

    assert_eq!(
        silver_count, events_count,
        "silver_events_parsed row count ({silver_count}) should equal raw.events row count ({events_count})"
    );

    // --- Step 6: verify JSON-extracted fields are non-null in at least one row ---
    // The silver model extracts event_name, platform, url from the JSON payload.
    // We verify these columns are populated (i.e. the JSON decode worked end-to-end).
    let (event_name, platform, url): (Option<String>, Option<String>, Option<String>) = conn2
        .query_row(
            "SELECT event_name, platform, url \
             FROM main.silver_events_parsed \
             WHERE event_name IS NOT NULL \
               AND platform IS NOT NULL \
               AND url IS NOT NULL \
             LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap_or_else(|e| {
            panic!("failed to read event_name/platform/url from silver_events_parsed: {e}")
        });

    assert!(
        event_name.is_some() && !event_name.as_deref().unwrap_or("").is_empty(),
        "event_name should be non-null and non-empty, got: {event_name:?}"
    );
    assert!(
        platform.is_some() && !platform.as_deref().unwrap_or("").is_empty(),
        "platform should be non-null and non-empty, got: {platform:?}"
    );
    assert!(
        url.is_some() && !url.as_deref().unwrap_or("").is_empty(),
        "url should be non-null and non-empty, got: {url:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 8: silver/sessions model materializes with sessionization invariants
// ---------------------------------------------------------------------------

/// Full pipeline test: run `smelt-datagen`, execute `setup_sources.sql`, invoke
/// `smelt build`, then verify that `main.silver_sessions` materializes with at
/// least one row, every row has a non-null platform, and the maximum
/// `session_seq` across all sessions is >= 1 (confirming that the 30-minute
/// inactivity / platform-boundary rule fired at least once).
///
/// `models/silver/sessions.sql` address segments are `["silver", "sessions"]`,
/// so smelt materializes the table as `silver_sessions` in the `main` schema.
#[test]
fn test_sessions_model_materializes() {
    let tmp = TempDir::new().expect("tempdir");
    let tmp_path = tmp.path();

    // --- Step 1: clone the web_analytics project tree into tmp_path ---
    let project_src = repo_root().join("examples/web_analytics");
    copy_dir_all(&project_src, tmp_path);

    // --- Step 2: run smelt-datagen with rewritten outputs into tmp_path ---
    let src_config = tmp_path.join("datagen.yaml");
    let dest_config = tmp_path.join("datagen_rewritten.yaml");
    rewrite_outputs(&src_config, &dest_config, tmp_path);

    let (ok, combined) = run_datagen(&dest_config, 0.01);
    assert!(ok, "smelt-datagen failed at scale-factor 0.01:\n{combined}");

    // --- Step 3: rewrite setup_sources.sql with absolute paths, execute ---
    let setup_sql_src = tmp_path.join("setup_sources.sql");
    let setup_sql_abs = tmp_path.join("setup_sources_abs.sql");
    rewrite_setup_sources_sql(&setup_sql_src, &setup_sql_abs, tmp_path);

    let db_path = tmp_path.join("target/dev.duckdb");
    fs::create_dir_all(db_path.parent().unwrap()).expect("mkdir target/");

    let conn = duckdb::Connection::open(&db_path).unwrap_or_else(|e| panic!("open duckdb: {e}"));

    let sql = fs::read_to_string(&setup_sql_abs)
        .unwrap_or_else(|e| panic!("read setup_sources_abs.sql: {e}"));
    conn.execute_batch(&sql)
        .unwrap_or_else(|e| panic!("execute setup_sources_abs.sql: {e}\nSQL:\n{sql}"));

    // Close connection so smelt build can open the file exclusively.
    drop(conn);

    // --- Step 4: run `smelt build` from the temp workspace directory ---
    let smelt = smelt_bin();
    assert!(
        smelt.exists(),
        "smelt binary not found at {smelt:?}; run `cargo build -p smelt-cli` first"
    );

    let build_out = Command::new(&smelt)
        .args(["build", "--target", "dev"])
        .current_dir(tmp_path)
        .env("RUST_LOG", "warn")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));

    assert!(
        build_out.status.success(),
        "`smelt build` exited {:?}\nstdout:\n{}\nstderr:\n{}",
        build_out.status,
        String::from_utf8_lossy(&build_out.stdout),
        String::from_utf8_lossy(&build_out.stderr),
    );

    // --- Step 5: verify silver_sessions has > 0 rows ---
    let conn2 = duckdb::Connection::open(&db_path).unwrap_or_else(|e| panic!("reopen duckdb: {e}"));

    let session_count: i64 = conn2
        .query_row("SELECT COUNT(*) FROM main.silver_sessions", [], |row| {
            row.get(0)
        })
        .unwrap_or_else(|e| panic!("SELECT COUNT(*) FROM main.silver_sessions: {e}"));

    assert!(
        session_count > 0,
        "silver_sessions should have at least one row, got 0"
    );

    // --- Step 6: verify every row has a non-null platform ---
    let null_platform_count: i64 = conn2
        .query_row(
            "SELECT COUNT(*) FROM main.silver_sessions WHERE platform IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|e| panic!("platform null check on silver_sessions: {e}"));

    assert_eq!(
        null_platform_count, 0,
        "every silver_sessions row must have a non-null platform; {null_platform_count} rows have NULL platform"
    );

    // --- Step 6b: verify each session has exactly one platform (boundary rule) ---
    let multi_platform_sessions: i64 = conn2
        .query_row(
            "SELECT COUNT(*) FROM (
                 SELECT session_id, COUNT(DISTINCT platform) AS plats
                 FROM main.silver_sessions
                 GROUP BY session_id
                 HAVING plats > 1
             )",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|e| panic!("per-session platform uniqueness query failed: {e}"));

    assert_eq!(
        multi_platform_sessions, 0,
        "every session must have exactly one distinct platform value (boundary rule); {multi_platform_sessions} sessions have multiple platforms"
    );

    // --- Step 7: verify max session_seq >= 1 (sessionization fired at least once) ---
    let max_seq: i64 = conn2
        .query_row(
            "SELECT MAX(session_seq) FROM main.silver_sessions",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|e| panic!("MAX(session_seq) from silver_sessions: {e}"));

    assert!(
        max_seq >= 1,
        "MAX(session_seq) should be >= 1, indicating at least one session boundary was detected; got {max_seq}"
    );
}

// ---------------------------------------------------------------------------
// Test 9: silver/device_user_edges view materializes with aggregation invariants
// ---------------------------------------------------------------------------

/// Full pipeline test: run `smelt-datagen`, execute `setup_sources.sql`, invoke
/// `smelt build`, then verify that `main.silver_device_user_edges` materializes
/// with at least one row, its row count matches the distinct (device_id, user_id)
/// pairs in `events_parsed` with non-null `user_id`, every edge has a non-zero
/// event count, and no edge has `first_seen > last_seen`.
///
/// `models/silver/device_user_edges.sql` address segments are
/// `["silver", "device_user_edges"]`, so smelt materializes the view as
/// `silver_device_user_edges` in the `main` schema.
#[test]
fn test_device_user_edges_view() {
    let tmp = TempDir::new().expect("tempdir");
    let tmp_path = tmp.path();

    // --- Step 1: clone the web_analytics project tree into tmp_path ---
    let project_src = repo_root().join("examples/web_analytics");
    copy_dir_all(&project_src, tmp_path);

    // --- Step 2: run smelt-datagen with rewritten outputs into tmp_path ---
    let src_config = tmp_path.join("datagen.yaml");
    let dest_config = tmp_path.join("datagen_rewritten.yaml");
    rewrite_outputs(&src_config, &dest_config, tmp_path);

    let (ok, combined) = run_datagen(&dest_config, 0.01);
    assert!(ok, "smelt-datagen failed at scale-factor 0.01:\n{combined}");

    // --- Step 3: rewrite setup_sources.sql with absolute paths, execute ---
    let setup_sql_src = tmp_path.join("setup_sources.sql");
    let setup_sql_abs = tmp_path.join("setup_sources_abs.sql");
    rewrite_setup_sources_sql(&setup_sql_src, &setup_sql_abs, tmp_path);

    let db_path = tmp_path.join("target/dev.duckdb");
    fs::create_dir_all(db_path.parent().unwrap()).expect("mkdir target/");

    let conn = duckdb::Connection::open(&db_path).unwrap_or_else(|e| panic!("open duckdb: {e}"));

    let sql = fs::read_to_string(&setup_sql_abs)
        .unwrap_or_else(|e| panic!("read setup_sources_abs.sql: {e}"));
    conn.execute_batch(&sql)
        .unwrap_or_else(|e| panic!("execute setup_sources_abs.sql: {e}\nSQL:\n{sql}"));

    // Close connection so smelt build can open the file exclusively.
    drop(conn);

    // --- Step 4: run `smelt build` from the temp workspace directory ---
    let smelt = smelt_bin();
    assert!(
        smelt.exists(),
        "smelt binary not found at {smelt:?}; run `cargo build -p smelt-cli` first"
    );

    let build_out = Command::new(&smelt)
        .args(["build", "--target", "dev"])
        .current_dir(tmp_path)
        .env("RUST_LOG", "warn")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));

    assert!(
        build_out.status.success(),
        "`smelt build` exited {:?}\nstdout:\n{}\nstderr:\n{}",
        build_out.status,
        String::from_utf8_lossy(&build_out.stdout),
        String::from_utf8_lossy(&build_out.stderr),
    );

    // --- Step 5: verify silver_device_user_edges has > 0 rows ---
    let conn2 = duckdb::Connection::open(&db_path).unwrap_or_else(|e| panic!("reopen duckdb: {e}"));

    let edge_count: i64 = conn2
        .query_row(
            "SELECT COUNT(*) FROM main.silver_device_user_edges",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|e| panic!("SELECT COUNT(*) FROM main.silver_device_user_edges: {e}"));

    assert!(
        edge_count > 0,
        "silver_device_user_edges should have at least one row, got 0"
    );

    // --- Step 6: verify row count matches distinct (device_id, user_id) pairs ---
    // The view should have exactly one row per distinct (device_id, user_id) pair
    // from events_parsed where user_id is non-null.
    let expected_edge_count: i64 = conn2
        .query_row(
            "SELECT COUNT(*) FROM (\
                 SELECT DISTINCT device_id, user_id \
                 FROM main.silver_events_parsed \
                 WHERE user_id IS NOT NULL\
             )",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|e| panic!("distinct (device_id, user_id) count query failed: {e}"));

    assert_eq!(
        edge_count, expected_edge_count,
        "silver_device_user_edges row count ({edge_count}) should equal \
         distinct (device_id, user_id) pairs in events_parsed with non-null user_id ({expected_edge_count})"
    );

    // --- Step 7: verify every edge has event_count >= 1 ---
    let min_event_count: i64 = conn2
        .query_row(
            "SELECT MIN(event_count) FROM main.silver_device_user_edges",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|e| panic!("MIN(event_count) from silver_device_user_edges: {e}"));

    assert!(
        min_event_count >= 1,
        "every edge must have event_count >= 1, got MIN(event_count) = {min_event_count}"
    );

    // --- Step 8: verify no edge has first_seen > last_seen ---
    let bad_temporal_count: i64 = conn2
        .query_row(
            "SELECT COUNT(*) FROM main.silver_device_user_edges WHERE first_seen > last_seen",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|e| panic!("temporal ordering check on silver_device_user_edges: {e}"));

    assert_eq!(
        bad_temporal_count, 0,
        "no edge should have first_seen > last_seen; {bad_temporal_count} rows violate this"
    );
}

// ---------------------------------------------------------------------------
// Test 10: session_boundary_invariants inline .test.sql passes
// ---------------------------------------------------------------------------

/// Inline test gate: run `smelt test --select session_boundary` against the
/// web_analytics project cloned into a temp dir, and assert exit 0 (all
/// matched tests pass).
///
/// The test file `tests/session_boundary_invariants.test.sql` exercises three
/// boundary rules against mock `silver/events_parsed` data:
///   - gap rule: events > 30 min apart on same platform → separate sessions
///   - platform rule: events on different platforms → separate sessions
///   - no-boundary: events < 30 min apart on same platform → one session
#[test]
fn test_sessions_invariants_inline_pass() {
    let tmp = TempDir::new().expect("tempdir");
    let tmp_path = tmp.path();

    // Clone the web_analytics project tree into tmp_path so the build artefacts
    // (DuckDB file, .smelt/ schema cache) never land in the checked-in source.
    let project_src = repo_root().join("examples/web_analytics");
    copy_dir_all(&project_src, tmp_path);

    let smelt = smelt_bin();
    assert!(
        smelt.exists(),
        "smelt binary not found at {smelt:?}; run `cargo build -p smelt-cli` first"
    );

    // Run `smelt test --select session_boundary` from the cloned project dir.
    let test_out = Command::new(&smelt)
        .args([
            "test",
            "--project-dir",
            tmp_path.to_str().expect("tmp_path is valid UTF-8"),
            "--select",
            "session_boundary",
        ])
        .env("RUST_LOG", "warn")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt test`: {e}"));

    let stdout = String::from_utf8_lossy(&test_out.stdout);
    let stderr = String::from_utf8_lossy(&test_out.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        test_out.status.success(),
        "`smelt test --select session_boundary` exited {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        test_out.status,
    );

    // Verify the named test reported PASS in the output.
    assert!(
        combined.contains("PASS") || combined.contains("passed"),
        "expected 'PASS' or 'passed' in smelt test output, got:\n{combined}"
    );
}

// ---------------------------------------------------------------------------
// Test 11: gold/identity_forward_only view materializes with identity invariants
// ---------------------------------------------------------------------------

/// Full pipeline test: run `smelt-datagen`, execute `setup_sources.sql`, invoke
/// `smelt build`, then verify that `main.gold_identity_forward_only` materializes
/// with at least one row, its row count matches `main.silver_sessions` (one row
/// per session), and every session that contains at least one signed-in event
/// has a non-null `forward_only_user_id`.
///
/// `models/gold/identity_forward_only.sql` address segments are
/// `["gold", "identity_forward_only"]`, so smelt materializes the view as
/// `gold_identity_forward_only` in the `main` schema.
#[test]
fn test_identity_forward_only_materializes() {
    let tmp = TempDir::new().expect("tempdir");
    let tmp_path = tmp.path();

    // --- Step 1: clone the web_analytics project tree into tmp_path ---
    let project_src = repo_root().join("examples/web_analytics");
    copy_dir_all(&project_src, tmp_path);

    // --- Step 2: run smelt-datagen with rewritten outputs into tmp_path ---
    let src_config = tmp_path.join("datagen.yaml");
    let dest_config = tmp_path.join("datagen_rewritten.yaml");
    rewrite_outputs(&src_config, &dest_config, tmp_path);

    let (ok, combined) = run_datagen(&dest_config, 0.01);
    assert!(ok, "smelt-datagen failed at scale-factor 0.01:\n{combined}");

    // --- Step 3: rewrite setup_sources.sql with absolute paths, execute ---
    let setup_sql_src = tmp_path.join("setup_sources.sql");
    let setup_sql_abs = tmp_path.join("setup_sources_abs.sql");
    rewrite_setup_sources_sql(&setup_sql_src, &setup_sql_abs, tmp_path);

    let db_path = tmp_path.join("target/dev.duckdb");
    fs::create_dir_all(db_path.parent().unwrap()).expect("mkdir target/");

    let conn = duckdb::Connection::open(&db_path).unwrap_or_else(|e| panic!("open duckdb: {e}"));

    let sql = fs::read_to_string(&setup_sql_abs)
        .unwrap_or_else(|e| panic!("read setup_sources_abs.sql: {e}"));
    conn.execute_batch(&sql)
        .unwrap_or_else(|e| panic!("execute setup_sources_abs.sql: {e}\nSQL:\n{sql}"));

    // Close connection so smelt build can open the file exclusively.
    drop(conn);

    // --- Step 4: run `smelt build` from the temp workspace directory ---
    let smelt = smelt_bin();
    assert!(
        smelt.exists(),
        "smelt binary not found at {smelt:?}; run `cargo build -p smelt-cli` first"
    );

    let build_out = Command::new(&smelt)
        .args(["build", "--target", "dev"])
        .current_dir(tmp_path)
        .env("RUST_LOG", "warn")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));

    assert!(
        build_out.status.success(),
        "`smelt build` exited {:?}\nstdout:\n{}\nstderr:\n{}",
        build_out.status,
        String::from_utf8_lossy(&build_out.stdout),
        String::from_utf8_lossy(&build_out.stderr),
    );

    // --- Step 5: verify gold_identity_forward_only has > 0 rows ---
    let conn2 = duckdb::Connection::open(&db_path).unwrap_or_else(|e| panic!("reopen duckdb: {e}"));

    let fwd_count: i64 = conn2
        .query_row(
            "SELECT COUNT(*) FROM main.gold_identity_forward_only",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|e| panic!("SELECT COUNT(*) FROM main.gold_identity_forward_only: {e}"));

    assert!(
        fwd_count > 0,
        "gold_identity_forward_only should have at least one row, got 0"
    );

    // --- Step 6: verify one-row-per-session cardinality ---
    // gold_identity_forward_only must have exactly as many rows as silver_sessions.
    let session_count: i64 = conn2
        .query_row("SELECT COUNT(*) FROM main.silver_sessions", [], |row| {
            row.get(0)
        })
        .unwrap_or_else(|e| panic!("SELECT COUNT(*) FROM main.silver_sessions: {e}"));

    assert_eq!(
        fwd_count, session_count,
        "gold_identity_forward_only row count ({fwd_count}) must equal silver_sessions row count ({session_count}) — one row per session"
    );

    // --- Step 7: population invariant ---
    // Every session that has at least one signed-in event must have a non-null
    // forward_only_user_id. The query counts sessions where the invariant is violated.
    let violation_count: i64 = conn2
        .query_row(
            "SELECT count(*) FROM (
               SELECT s.session_id
               FROM main.silver_sessions s
               JOIN main.silver_events_parsed e
                 ON e.device_id = s.device_id
                AND e.event_ts BETWEEN s.session_start AND s.session_end
               WHERE e.user_id IS NOT NULL
               GROUP BY s.session_id
             ) sessions_with_user
             JOIN main.gold_identity_forward_only f USING (session_id)
             WHERE f.forward_only_user_id IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|e| panic!("population invariant query failed: {e}"));

    assert_eq!(
        violation_count, 0,
        "every session with at least one signed-in event must have a non-null forward_only_user_id; {violation_count} sessions violate this invariant"
    );
}

// ---------------------------------------------------------------------------
// Test 12: gold/eventstream_with_identity view materializes with join invariants
// ---------------------------------------------------------------------------

/// Full pipeline test: run `smelt-datagen`, execute `setup_sources.sql`, invoke
/// `smelt build`, then verify that `main.gold_eventstream_with_identity`
/// materializes with the correct cardinality, identity column invariants, and
/// expected column shape.
///
/// `models/gold/eventstream_with_identity.sql` address segments are
/// `["gold", "eventstream_with_identity"]`, so smelt materializes the view as
/// `gold_eventstream_with_identity` in the `main` schema.
#[test]
fn test_eventstream_with_identity_end_to_end() {
    let tmp = TempDir::new().expect("tempdir");
    let tmp_path = tmp.path();

    // --- Step 1: clone the web_analytics project tree into tmp_path ---
    let project_src = repo_root().join("examples/web_analytics");
    copy_dir_all(&project_src, tmp_path);

    // --- Step 2: run smelt-datagen with rewritten outputs into tmp_path ---
    let src_config = tmp_path.join("datagen.yaml");
    let dest_config = tmp_path.join("datagen_rewritten.yaml");
    rewrite_outputs(&src_config, &dest_config, tmp_path);

    let (ok, combined) = run_datagen(&dest_config, 0.01);
    assert!(ok, "smelt-datagen failed at scale-factor 0.01:\n{combined}");

    // --- Step 3: rewrite setup_sources.sql with absolute paths, execute ---
    let setup_sql_src = tmp_path.join("setup_sources.sql");
    let setup_sql_abs = tmp_path.join("setup_sources_abs.sql");
    rewrite_setup_sources_sql(&setup_sql_src, &setup_sql_abs, tmp_path);

    let db_path = tmp_path.join("target/dev.duckdb");
    fs::create_dir_all(db_path.parent().unwrap()).expect("mkdir target/");

    let conn = duckdb::Connection::open(&db_path).unwrap_or_else(|e| panic!("open duckdb: {e}"));

    let sql = fs::read_to_string(&setup_sql_abs)
        .unwrap_or_else(|e| panic!("read setup_sources_abs.sql: {e}"));
    conn.execute_batch(&sql)
        .unwrap_or_else(|e| panic!("execute setup_sources_abs.sql: {e}\nSQL:\n{sql}"));

    // Close connection so smelt build can open the file exclusively.
    drop(conn);

    // --- Step 4: run `smelt build` from the temp workspace directory ---
    let smelt = smelt_bin();
    assert!(
        smelt.exists(),
        "smelt binary not found at {smelt:?}; run `cargo build -p smelt-cli` first"
    );

    let build_out = Command::new(&smelt)
        .args(["build", "--target", "dev"])
        .current_dir(tmp_path)
        .env("RUST_LOG", "warn")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));

    assert!(
        build_out.status.success(),
        "`smelt build` exited {:?}\nstdout:\n{}\nstderr:\n{}",
        build_out.status,
        String::from_utf8_lossy(&build_out.stdout),
        String::from_utf8_lossy(&build_out.stderr),
    );

    // --- Step 5: verify gold_eventstream_with_identity has > 0 rows ---
    let conn2 = duckdb::Connection::open(&db_path).unwrap_or_else(|e| panic!("reopen duckdb: {e}"));

    let stream_count: i64 = conn2
        .query_row(
            "SELECT COUNT(*) FROM main.gold_eventstream_with_identity",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|e| {
            panic!("SELECT COUNT(*) FROM main.gold_eventstream_with_identity: {e}")
        });

    assert!(
        stream_count > 0,
        "gold_eventstream_with_identity should have at least one row, got 0"
    );

    // --- Step 6: event-preserving cardinality invariant ---
    // The JOIN to sessions is one-to-one on (device_id, event_ts ∈ [session_start, session_end]);
    // every event is in exactly one session so no row is dropped or duplicated.
    let events_count: i64 = conn2
        .query_row(
            "SELECT COUNT(*) FROM main.silver_events_parsed",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|e| panic!("SELECT COUNT(*) FROM main.silver_events_parsed: {e}"));

    assert_eq!(
        stream_count, events_count,
        "gold_eventstream_with_identity row count ({stream_count}) must equal \
         silver_events_parsed row count ({events_count}) — one row per event"
    );

    // --- Step 7: single-valued forward_only_user_id within session ---
    // No session should resolve to two distinct non-null forward_only_user_id values.
    // (NULL values are ignored by COUNT(DISTINCT ...) by default in SQL.)
    let multi_uid_sessions: i64 = conn2
        .query_row(
            "SELECT COUNT(*) FROM (
                 SELECT session_id, COUNT(DISTINCT forward_only_user_id) AS k
                 FROM main.gold_eventstream_with_identity
                 GROUP BY session_id
                 HAVING k > 1
             )",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|e| {
            panic!("single-valued forward_only_user_id invariant query failed: {e}")
        });

    assert_eq!(
        multi_uid_sessions, 0,
        "no session should have more than one distinct non-null forward_only_user_id; \
         {multi_uid_sessions} sessions violate this invariant"
    );

    // --- Step 8: non-null resolution for signed-in events ---
    // If an event row has a non-null event_user_id, its session must resolve to
    // a non-null forward_only_user_id (the algorithm sees at least one non-null input).
    let unresolved_signed_in: i64 = conn2
        .query_row(
            "SELECT COUNT(*) FROM main.gold_eventstream_with_identity \
             WHERE event_user_id IS NOT NULL AND forward_only_user_id IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|e| panic!("non-null resolution for signed-in events query failed: {e}"));

    assert_eq!(
        unresolved_signed_in, 0,
        "every event with a non-null event_user_id must have a non-null forward_only_user_id; \
         {unresolved_signed_in} rows violate this invariant"
    );

    // --- Step 9: verify column shape ---
    // The SELECT list must include exactly these columns in the expected positions.
    // We query a single row and verify each named column is accessible.
    // All numeric and date/timestamp columns are cast to TEXT for portable comparison.
    #[allow(clippy::type_complexity)]
    let col_check: Result<
        (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
        _,
    > = conn2.query_row(
        "SELECT event_id::TEXT, device_id::TEXT, event_user_id::TEXT, event_ts::TEXT, \
                    event_date::TEXT, event_name, platform, url, session_id::TEXT, \
                    forward_only_user_id::TEXT \
             FROM main.gold_eventstream_with_identity \
             LIMIT 1",
        [],
        |row| {
            Ok((
                row.get::<_, Option<String>>(0)?, // event_id (as text)
                row.get::<_, Option<String>>(1)?, // device_id (as text)
                row.get::<_, Option<String>>(2)?, // event_user_id (nullable, as text)
                row.get::<_, Option<String>>(3)?, // event_ts (as text)
                row.get::<_, Option<String>>(4)?, // event_date (as text)
                row.get::<_, Option<String>>(5)?, // event_name
                row.get::<_, Option<String>>(6)?, // platform
                row.get::<_, Option<String>>(7)?, // url
                row.get::<_, Option<String>>(8)?, // session_id (as text)
                row.get::<_, Option<String>>(9)?, // forward_only_user_id (nullable, as text)
            ))
        },
    );

    assert!(
        col_check.is_ok(),
        "column shape check failed — one or more expected columns missing from \
         gold_eventstream_with_identity: {:?}",
        col_check.err()
    );
}

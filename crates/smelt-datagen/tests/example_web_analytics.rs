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
// Test 5: end-to-end build materializes silver/events_parsed
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

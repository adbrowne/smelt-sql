//! Establishes the Trino/Iceberg capability profile **by execution**: one
//! probe per matrix flag, run against a live coordinator
//! (`scripts/trino-up.sh` + `scripts/trino-env.sh`), asserted against
//! `BackendCapabilities::trino_iceberg()` — the same constructor the spec
//! matrix and `capability_conformance.rs` assert against
//! (`docs/outcomes/20260913-trino-target-spine/outcome.md` phase 8).
//!
//! Gated on `SMELT_TRINO_URL`: unset, every test here skips green (mirrors
//! `backend_live.rs`). Run with:
//!   bash scripts/trino-up.sh
//!   source scripts/trino-env.sh
//!   cargo test -p smelt-backend-trino --test capability_probes
//!   bash scripts/trino-down.sh

use smelt_backend::Backend;
use smelt_backend_trino::{TrinoBackend, TrinoClientConfig};
use smelt_dialect::{BackendCapabilities, NullSafeEqualitySpelling, SqlDialect};

struct LiveEnv {
    backend: TrinoBackend,
    catalog: String,
    schema: String,
}

fn unique_schema() -> String {
    let base = std::env::var("SMELT_TRINO_SCHEMA").unwrap_or_else(|_| "smelt_dev".to_string());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{base}_cap_{}_{nanos}", std::process::id())
}

/// Connect and create the run's isolated schema, or `None` when
/// `SMELT_TRINO_URL` is unset (the caller should skip green).
async fn live_env_or_skip(test_name: &str) -> Option<LiveEnv> {
    let Ok(base_url) = std::env::var("SMELT_TRINO_URL") else {
        eprintln!("Skipping {test_name} — set SMELT_TRINO_URL");
        return None;
    };
    let user = std::env::var("SMELT_TRINO_USER").unwrap_or_else(|_| "smelt".to_string());
    let catalog = std::env::var("SMELT_TRINO_CATALOG").unwrap_or_else(|_| "iceberg".to_string());
    let schema = unique_schema();

    let backend = TrinoBackend::new(TrinoClientConfig {
        base_url,
        user,
        catalog: catalog.clone(),
        schema: schema.clone(),
        password: None,
    });
    backend
        .ensure_schema(&schema)
        .await
        .unwrap_or_else(|e| panic!("ensure_schema must succeed against a live tier: {e}"));

    Some(LiveEnv {
        backend,
        catalog,
        schema,
    })
}

async fn drop_schema(env: &LiveEnv) {
    let _ = env
        .backend
        .execute_sql(&format!(
            "DROP SCHEMA IF EXISTS \"{}\".\"{}\"",
            env.catalog, env.schema
        ))
        .await;
}

impl LiveEnv {
    fn q(&self, name: &str) -> String {
        format!("\"{}\".\"{}\".\"{name}\"", self.catalog, self.schema)
    }

    async fn ok(&self, sql: &str) -> bool {
        self.backend.execute_sql(sql).await.is_ok()
    }
}

/// `QUALIFY` is not Trino grammar.
#[tokio::test]
async fn probe_supports_qualify() {
    let Some(env) = live_env_or_skip("probe_supports_qualify").await else {
        return;
    };
    let measured = env
        .ok("SELECT n FROM (VALUES (1),(2)) AS t(n) QUALIFY ROW_NUMBER() OVER (ORDER BY n) = 1")
        .await;
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_qualify,
        "supports_qualify: measured {measured} against a live coordinator"
    );
    drop_schema(&env).await;
}

/// `CREATE OR REPLACE TABLE ... AS SELECT`.
#[tokio::test]
async fn probe_supports_create_or_replace_table() {
    let Some(env) = live_env_or_skip("probe_supports_create_or_replace_table").await else {
        return;
    };
    env.backend
        .execute_sql(&format!("CREATE TABLE {} AS SELECT 1 AS n", env.q("t")))
        .await
        .unwrap();
    let measured = env
        .ok(&format!(
            "CREATE OR REPLACE TABLE {} AS SELECT 2 AS n",
            env.q("t")
        ))
        .await;
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_create_or_replace_table,
        "supports_create_or_replace_table"
    );
    drop_schema(&env).await;
}

/// `CREATE OR REPLACE VIEW ... AS SELECT`.
#[tokio::test]
async fn probe_supports_create_or_replace_view() {
    let Some(env) = live_env_or_skip("probe_supports_create_or_replace_view").await else {
        return;
    };
    let measured = env
        .ok(&format!(
            "CREATE OR REPLACE VIEW {} AS SELECT 1 AS n",
            env.q("v")
        ))
        .await;
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_create_or_replace_view,
        "supports_create_or_replace_view"
    );
    drop_schema(&env).await;
}

/// `MERGE INTO ... WHEN MATCHED ... WHEN NOT MATCHED ...`.
#[tokio::test]
async fn probe_supports_merge() {
    let Some(env) = live_env_or_skip("probe_supports_merge").await else {
        return;
    };
    env.backend
        .execute_sql(&format!("CREATE TABLE {} AS SELECT 1 AS n", env.q("t")))
        .await
        .unwrap();
    let measured = env
        .ok(&format!(
            "MERGE INTO {} t USING (SELECT 1 AS n) s ON t.n = s.n \
             WHEN MATCHED THEN UPDATE SET n = s.n \
             WHEN NOT MATCHED THEN INSERT (n) VALUES (s.n)",
            env.q("t")
        ))
        .await;
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_merge,
        "supports_merge"
    );
    drop_schema(&env).await;
}

/// An explicit partial `WHEN MATCHED THEN UPDATE SET lbl = s.lbl` — the
/// column-scoped shape (some columns recomputed, the rest passed through
/// untouched), not DuckDB/Spark's `SET *` shorthand (a separate syntactic
/// question this flag does not gate).
#[tokio::test]
async fn probe_supports_column_scoped_merge() {
    let Some(env) = live_env_or_skip("probe_supports_column_scoped_merge").await else {
        return;
    };
    env.backend
        .execute_sql(&format!(
            "CREATE TABLE {} AS SELECT 1 AS n, 'a' AS lbl",
            env.q("t")
        ))
        .await
        .unwrap();
    let measured = env
        .ok(&format!(
            "MERGE INTO {} t USING (SELECT 1 AS n, 'b' AS lbl) s ON t.n = s.n \
             WHEN MATCHED THEN UPDATE SET lbl = s.lbl",
            env.q("t")
        ))
        .await;
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_column_scoped_merge,
        "supports_column_scoped_merge"
    );
    drop_schema(&env).await;
}

/// `WHEN NOT MATCHED BY SOURCE THEN DELETE` — spec-only flag, no struct
/// field yet (§Known Divergences); measured and recorded but not asserted
/// against a `BackendCapabilities` field.
#[tokio::test]
async fn probe_supports_merge_not_matched_by_source() {
    let Some(env) = live_env_or_skip("probe_supports_merge_not_matched_by_source").await else {
        return;
    };
    env.backend
        .execute_sql(&format!("CREATE TABLE {} AS SELECT 1 AS n", env.q("t")))
        .await
        .unwrap();
    let measured = env
        .ok(&format!(
            "MERGE INTO {} t USING (SELECT 999 AS n) s ON t.n = s.n \
             WHEN NOT MATCHED BY SOURCE THEN DELETE",
            env.q("t")
        ))
        .await;
    assert!(
        !measured,
        "supports_merge_not_matched_by_source measured true — update the spec matrix cell"
    );
    drop_schema(&env).await;
}

/// Create, populate, use and drop a named relation as a group — the
/// staged-candidate pattern. Spec-only flag, no struct field yet.
#[tokio::test]
async fn probe_supports_staged_relation_group() {
    let Some(env) = live_env_or_skip("probe_supports_staged_relation_group").await else {
        return;
    };
    let group = async {
        env.backend
            .execute_sql(&format!(
                "CREATE TABLE {} AS SELECT 1 AS n",
                env.q("staged")
            ))
            .await?;
        env.backend
            .execute_sql(&format!("INSERT INTO {} SELECT 2", env.q("staged")))
            .await?;
        let r = env
            .backend
            .execute_sql(&format!("SELECT count(*) FROM {}", env.q("staged")))
            .await;
        env.backend
            .execute_sql(&format!("DROP TABLE {}", env.q("staged")))
            .await?;
        r
    }
    .await;
    assert!(
        group.is_ok(),
        "supports_staged_relation_group measured false — update the spec matrix cell: {group:?}"
    );
    drop_schema(&env).await;
}

/// `PIVOT (... FOR ... IN (...))`.
#[tokio::test]
async fn probe_supports_pivot() {
    let Some(env) = live_env_or_skip("probe_supports_pivot").await else {
        return;
    };
    let measured = env
        .ok("SELECT * FROM (VALUES (1,'a'),(2,'a')) AS t(id,cat) PIVOT (COUNT(id) FOR cat IN ('a'))")
        .await;
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_pivot,
        "supports_pivot"
    );
    drop_schema(&env).await;
}

/// `DATE '2024-01-01'`.
#[tokio::test]
async fn probe_supports_date_literal() {
    let Some(env) = live_env_or_skip("probe_supports_date_literal").await else {
        return;
    };
    let measured = env.ok("SELECT DATE '2024-01-01'").await;
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_date_literal,
        "supports_date_literal"
    );
    drop_schema(&env).await;
}

/// `'a' || 'b'`.
#[tokio::test]
async fn probe_supports_concat_operator() {
    let Some(env) = live_env_or_skip("probe_supports_concat_operator").await else {
        return;
    };
    let measured = env.ok("SELECT 'a' || 'b'").await;
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_concat_operator,
        "supports_concat_operator"
    );
    drop_schema(&env).await;
}

/// `[a,b,c]` bracket array-literal syntax. Uses `cardinality(...)` rather
/// than selecting the array value itself — decoding a Trino `array(...)`
/// result to Arrow is an unrelated, separate gap this flag does not gate.
#[tokio::test]
async fn probe_supports_array_literal() {
    let Some(env) = live_env_or_skip("probe_supports_array_literal").await else {
        return;
    };
    let measured = env.ok("SELECT cardinality([1,2,3])").await;
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_array_literal,
        "supports_array_literal"
    );
    drop_schema(&env).await;
}

/// `START TRANSACTION` / DDL / `ROLLBACK` over the stateless
/// `/v1/statement` client.
#[tokio::test]
async fn probe_supports_transactional_ddl() {
    let Some(env) = live_env_or_skip("probe_supports_transactional_ddl").await else {
        return;
    };
    let txn = async {
        env.backend.execute_sql("START TRANSACTION").await?;
        env.backend
            .execute_sql(&format!("CREATE TABLE {} AS SELECT 1 AS n", env.q("t")))
            .await?;
        env.backend.execute_sql("ROLLBACK").await
    }
    .await;
    let measured = txn.is_ok();
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_transactional_ddl,
        "supports_transactional_ddl: {txn:?}"
    );
    drop_schema(&env).await;
}

/// `x::T`.
#[tokio::test]
async fn probe_supports_double_colon_cast() {
    let Some(env) = live_env_or_skip("probe_supports_double_colon_cast").await else {
        return;
    };
    let measured = env.ok("SELECT 1::INTEGER").await;
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_double_colon_cast,
        "supports_double_colon_cast"
    );
    drop_schema(&env).await;
}

/// A trailing comma before the list's close.
#[tokio::test]
async fn probe_supports_trailing_commas() {
    let Some(env) = live_env_or_skip("probe_supports_trailing_commas").await else {
        return;
    };
    let measured = env.ok("SELECT 1, 2,").await;
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_trailing_commas,
        "supports_trailing_commas"
    );
    drop_schema(&env).await;
}

/// `INSERT OVERWRITE ...`.
#[tokio::test]
async fn probe_supports_insert_overwrite() {
    let Some(env) = live_env_or_skip("probe_supports_insert_overwrite").await else {
        return;
    };
    env.backend
        .execute_sql(&format!("CREATE TABLE {} AS SELECT 1 AS n", env.q("t")))
        .await
        .unwrap();
    let measured = env
        .ok(&format!("INSERT OVERWRITE {} SELECT 2 AS n", env.q("t")))
        .await;
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_insert_overwrite,
        "supports_insert_overwrite"
    );
    drop_schema(&env).await;
}

/// `CREATE MATERIALIZED VIEW` over the Iceberg REST catalog.
#[tokio::test]
async fn probe_supports_native_ivm() {
    let Some(env) = live_env_or_skip("probe_supports_native_ivm").await else {
        return;
    };
    env.backend
        .execute_sql(&format!("CREATE TABLE {} AS SELECT 1 AS n", env.q("base")))
        .await
        .unwrap();
    let measured = env
        .ok(&format!(
            "CREATE MATERIALIZED VIEW {} AS SELECT count(*) AS c FROM {}",
            env.q("mv"),
            env.q("base")
        ))
        .await;
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_native_ivm,
        "supports_native_ivm"
    );
    drop_schema(&env).await;
}

/// `ALTER TABLE ... ADD COLUMN s.b INTEGER` against a `ROW(a INTEGER)`
/// column — the dot-notation nested-struct-field DDL path.
#[tokio::test]
async fn probe_supports_struct_field_ddl() {
    let Some(env) = live_env_or_skip("probe_supports_struct_field_ddl").await else {
        return;
    };
    env.backend
        .execute_sql(&format!(
            "CREATE TABLE {} (id INTEGER, s ROW(a INTEGER))",
            env.q("t")
        ))
        .await
        .unwrap();
    let measured = env
        .ok(&format!(
            "ALTER TABLE {} ADD COLUMN s.b INTEGER",
            env.q("t")
        ))
        .await;
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_struct_field_ddl,
        "supports_struct_field_ddl"
    );
    drop_schema(&env).await;
}

/// `ALTER TABLE ... ALTER COLUMN ... SET DATA TYPE ... USING ...`.
#[tokio::test]
async fn probe_supports_alter_column_using() {
    let Some(env) = live_env_or_skip("probe_supports_alter_column_using").await else {
        return;
    };
    env.backend
        .execute_sql(&format!("CREATE TABLE {} AS SELECT 1 AS n", env.q("t")))
        .await
        .unwrap();
    let measured = env
        .ok(&format!(
            "ALTER TABLE {} ALTER COLUMN n SET DATA TYPE VARCHAR USING CAST(n AS VARCHAR)",
            env.q("t")
        ))
        .await;
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_alter_column_using,
        "supports_alter_column_using"
    );
    drop_schema(&env).await;
}

/// `ALTER TABLE ... ADD COLUMN items.element.b INTEGER` against an
/// `ARRAY(ROW(a INTEGER))` column.
#[tokio::test]
async fn probe_supports_nested_array_ddl() {
    let Some(env) = live_env_or_skip("probe_supports_nested_array_ddl").await else {
        return;
    };
    env.backend
        .execute_sql(&format!(
            "CREATE TABLE {} (id INTEGER, items ARRAY(ROW(a INTEGER)))",
            env.q("t")
        ))
        .await
        .unwrap();
    let measured = env
        .ok(&format!(
            "ALTER TABLE {} ADD COLUMN items.element.b INTEGER",
            env.q("t")
        ))
        .await;
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_nested_array_ddl,
        "supports_nested_array_ddl"
    );
    drop_schema(&env).await;
}

/// Writing a row with an extra column the target schema does not declare.
#[tokio::test]
async fn probe_supports_merge_schema_write() {
    let Some(env) = live_env_or_skip("probe_supports_merge_schema_write").await else {
        return;
    };
    env.backend
        .execute_sql(&format!("CREATE TABLE {} AS SELECT 1 AS n", env.q("t")))
        .await
        .unwrap();
    let measured = env
        .ok(&format!(
            "INSERT INTO {} SELECT 2 AS n, 'x' AS extra",
            env.q("t")
        ))
        .await;
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_merge_schema_write,
        "supports_merge_schema_write"
    );
    drop_schema(&env).await;
}

/// `ALTER TABLE ... RENAME COLUMN old_name TO new_name` followed by
/// `SELECT new_name` — Iceberg's field-ID column tracking should survive the
/// rename with no rewrite, reading the prior data back.
#[tokio::test]
async fn probe_supports_column_mapping() {
    let Some(env) = live_env_or_skip("probe_supports_column_mapping").await else {
        return;
    };
    let round_trip = async {
        env.backend
            .execute_sql(&format!(
                "CREATE TABLE {} AS SELECT 1 AS old_name",
                env.q("t")
            ))
            .await?;
        env.backend
            .execute_sql(&format!(
                "ALTER TABLE {} RENAME COLUMN old_name TO new_name",
                env.q("t")
            ))
            .await?;
        env.backend
            .execute_sql(&format!("SELECT new_name FROM {}", env.q("t")))
            .await
    }
    .await;
    let measured = round_trip.is_ok();
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_column_mapping,
        "supports_column_mapping: {round_trip:?}"
    );
    drop_schema(&env).await;
}

/// `FROM t |> WHERE ...` native pipe syntax.
#[tokio::test]
async fn probe_supports_pipe_syntax() {
    let Some(env) = live_env_or_skip("probe_supports_pipe_syntax").await else {
        return;
    };
    env.backend
        .execute_sql(&format!("CREATE TABLE {} AS SELECT 1 AS n", env.q("t")))
        .await
        .unwrap();
    let measured = env.ok(&format!("FROM {} |> WHERE n > 0", env.q("t"))).await;
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_pipe_syntax,
        "supports_pipe_syntax"
    );
    drop_schema(&env).await;
}

/// `SELECT * EXCLUDE (...)` — one leg of the star-modifier trio; sufficient
/// to establish the flag is `false` (the trio requires all three).
#[tokio::test]
async fn probe_supports_pipe_set_drop_rename() {
    let Some(env) = live_env_or_skip("probe_supports_pipe_set_drop_rename").await else {
        return;
    };
    env.backend
        .execute_sql(&format!("CREATE TABLE {} AS SELECT 1 AS n", env.q("t")))
        .await
        .unwrap();
    let measured = env
        .ok(&format!("SELECT * EXCLUDE (n) FROM {}", env.q("t")))
        .await;
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().supports_pipe_set_drop_rename,
        "supports_pipe_set_drop_rename"
    );
    drop_schema(&env).await;
}

/// Writing into a schema that was never passed to `ensure_schema`.
#[tokio::test]
async fn probe_requires_schema_init() {
    let Some(env) = live_env_or_skip("probe_requires_schema_init").await else {
        return;
    };
    let never_created = format!("{}_never_created", env.schema);
    let measured = env
        .ok(&format!(
            "CREATE TABLE \"{}\".\"{never_created}\".\"x\" AS SELECT 1",
            env.catalog
        ))
        .await;
    // requires_schema_init is true when a write *without* prior schema
    // creation fails.
    assert_eq!(
        !measured,
        BackendCapabilities::trino_iceberg().requires_schema_init,
        "requires_schema_init"
    );
    drop_schema(&env).await;
}

/// `IS NOT DISTINCT FROM` vs `<=>` — which spelling Trino accepts.
#[tokio::test]
async fn probe_null_safe_equality() {
    let Some(env) = live_env_or_skip("probe_null_safe_equality").await else {
        return;
    };
    let is_not_distinct = env.ok("SELECT 1 IS NOT DISTINCT FROM NULL").await;
    let spaceship = env.ok("SELECT 1 <=> NULL").await;
    let measured = match (is_not_distinct, spaceship) {
        (true, false) => NullSafeEqualitySpelling::IsNotDistinctFrom,
        (false, true) => NullSafeEqualitySpelling::Spaceship,
        other => panic!("null_safe_equality: expected exactly one spelling to work, got {other:?}"),
    };
    assert_eq!(
        measured,
        BackendCapabilities::trino_iceberg().null_safe_equality,
        "null_safe_equality"
    );
    drop_schema(&env).await;
}

/// The two `SqlDialect` *language* properties phase 2 landed conservatively
/// `false`.
#[tokio::test]
async fn language_properties_are_measured() {
    let Some(env) = live_env_or_skip("language_properties_are_measured").await else {
        return;
    };
    let filter_clause = env
        .ok("SELECT max(n) FILTER (WHERE n > 0) FROM (VALUES (1),(2)) AS t(n)")
        .await;
    assert_eq!(
        filter_clause,
        SqlDialect::Trino.supports_aggregate_filter_clause(),
        "supports_aggregate_filter_clause"
    );

    let interval_range = env
        .ok(
            "SELECT d, sum(v) OVER (ORDER BY d RANGE BETWEEN INTERVAL '2' DAY PRECEDING AND CURRENT ROW) \
             FROM (VALUES (DATE '2024-01-01', 1), (DATE '2024-01-02', 2)) AS t(d, v)",
        )
        .await;
    assert_eq!(
        interval_range,
        SqlDialect::Trino.supports_interval_range_frame(),
        "supports_interval_range_frame"
    );
    drop_schema(&env).await;
}

/// Reflection-free totality check: every flag named in the spec's Trino
/// column (struct fields and the two spec-only rows) has a probe above,
/// read straight from `docs/specs/multi_backend.md`'s own capability matrix
/// rather than a second hand-typed list — a flag renamed or added in the
/// spec and forgotten here fails by name.
///
/// `supports_retraction` and `supports_fingerprint_sidecar` are excluded:
/// the former is meaningful only alongside `supports_native_ivm` (measured
/// false, so retraction is trivially false, not independently probed), the
/// latter is an implementation-scope fact about smelt's own code (whether
/// the fingerprint sidecar has been built for this backend), not an engine
/// capability a live probe can measure. `dialect` and the free-text
/// parenthetical on `supports_pivot`-style rows are not flags.
#[test]
fn every_capability_field_has_a_probe() {
    let probed_flags = [
        "supports_qualify",
        "supports_create_or_replace_table",
        "supports_create_or_replace_view",
        "supports_merge",
        "supports_column_scoped_merge",
        "supports_merge_not_matched_by_source", // spec-only
        "supports_staged_relation_group",       // spec-only
        "supports_pivot",
        "supports_date_literal",
        "supports_concat_operator",
        "supports_array_literal",
        "supports_transactional_ddl",
        "supports_double_colon_cast",
        "supports_trailing_commas",
        "supports_insert_overwrite",
        "supports_native_ivm",
        "supports_struct_field_ddl",
        "supports_alter_column_using",
        "supports_nested_array_ddl",
        "supports_merge_schema_write",
        "supports_column_mapping",
        "supports_pipe_syntax",
        "supports_pipe_set_drop_rename",
        "requires_schema_init",
        "null_safe_equality",
    ];
    let not_independently_probed = ["supports_retraction", "supports_fingerprint_sidecar"];

    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let spec = std::fs::read_to_string(repo_root.join("docs/specs/multi_backend.md")).unwrap();

    let header_start = spec.find("| Flag |").expect("no capability matrix header");
    let table_end = spec[header_start..]
        .find("\n\n")
        .map(|i| header_start + i)
        .unwrap_or(spec.len());
    let table = &spec[header_start..table_end];

    for line in table.lines().skip(2) {
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        // cells[0] empty (leading `|`), cells[1] is the flag cell, e.g.
        // "`supports_qualify`" or "`supports_pivot` (some prose) ...".
        let flag_cell = cells[1];
        let flag_name = flag_cell
            .strip_prefix('`')
            .and_then(|s| s.split('`').next())
            .unwrap_or_else(|| panic!("could not extract flag name from cell: {flag_cell}"));

        assert!(
            probed_flags.contains(&flag_name) || not_independently_probed.contains(&flag_name),
            "flag `{flag_name}` from the spec's capability matrix has no probe in \
             capability_probes.rs (probed_flags or not_independently_probed)"
        );
    }
}

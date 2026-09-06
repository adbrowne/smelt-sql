//! The succession-patch technique's four emitter outputs
//! (`docs/specs/incremental_shapes.md` §"The succession grain",
//! §"The tombstone ledger (hidden state)"; `docs/specs/model_transforms.md`'s
//! "Succession-patch keyed `MERGE`" row): the event-delta `SELECT`, the
//! patch `MERGE` over the neighbour domain, the ledger-rebuild `SELECT`, and
//! the clock-tie probe. Pure string construction over caller-supplied
//! inputs, matching every other emitter in this module — a caller (the
//! runtime driver, `docs/outcomes/20260906-scd2-keyed-succession/
//! phases/05-plan.md`) resolves the classifier verdict's `lead_cols`/
//! `lag_cols` output-column names to their own rendered expression
//! templates before calling [`emit_succession_patch`]/
//! [`emit_succession_clock_tie_probe`]; this module never re-derives them
//! from the model's SQL.
//!
//! This phase patches the *whole touched-key history* on every window,
//! rather than the minimal immediate-neighbour footprint the maintenance
//! theorem names (`incremental_shapes.md` §"The maintenance theorem
//! (bounded footprint)") — window functions partition by key, so
//! recomputing a touched key's full stored history and re-`MERGE`ing it
//! back is correct (unaffected rows re-write their own unchanged values,
//! an idempotent no-op) but not the theorem's constant-footprint
//! optimisation. Narrowing the `USING` projection to just the new rows and
//! their immediate predecessor/successor is a follow-up, not a correctness
//! gap this phase leaves open.

use super::types::*;

/// The tombstone ledger's reserved name: `<presented table>__tombstones`,
/// schema-qualification preserved because `presented_table` is already the
/// fully qualified `schema.table` spelling and the suffix extends only the
/// trailing identifier (`incremental_shapes.md` §"The tombstone ledger
/// (hidden state)" — "Physical shape").
pub fn tombstone_table_name(presented_table: &str) -> String {
    format!("{presented_table}__tombstones")
}

fn key_col_list(key_cols: &[String]) -> String {
    key_cols.join(", ")
}

fn key_join_cond(left_alias: &str, right_alias: &str, key_cols: &[String]) -> String {
    key_cols
        .iter()
        .map(|k| format!("{left_alias}.{k} = {right_alias}.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn touched_keys_predicate(key_cols: &[String], event_delta_select: &str) -> String {
    let keys = key_col_list(key_cols);
    format!("({keys}) IN (SELECT {keys} FROM ({event_delta_select}) AS __smelt_touched_keys)")
}

/// The event-delta `SELECT` (`model_transforms.md`'s "Succession-patch
/// keyed `MERGE`" row): the model's pre-window filter and row-local
/// projection over the window's own source rows, with no window function —
/// the model SQL itself is never executed incrementally, only used as the
/// full-refresh oracle. `row_local_projection` is `(output_column, source
/// expression)` pairs — the key columns, the clock column, the delete flag
/// (if the grain admits one), and every other row-local payload column the
/// model projects, in the model's own column order.
pub fn emit_succession_event_delta(
    source_table: &str,
    row_local_projection: &[(String, String)],
    pre_filter: Option<&str>,
    window_predicate: &str,
) -> MaintenanceStatement {
    let select_list = row_local_projection
        .iter()
        .map(|(col, expr)| format!("{expr} AS {col}"))
        .collect::<Vec<_>>()
        .join(", ");
    let predicate = match pre_filter {
        Some(pf) => format!("({pf}) AND ({window_predicate})"),
        None => window_predicate.to_string(),
    };
    MaintenanceStatement::new(format!(
        "SELECT {select_list} FROM {source_table} WHERE {predicate}"
    ))
}

/// The tombstone-ledger rebuild `SELECT` (`incremental_shapes.md` §"The
/// tombstone ledger (hidden state)" — "Lifecycle"): `k, t` of every
/// delete-flagged row passing the pre-filter, over the rebuilt range. Used
/// by `--full-refresh` (the whole source) and `smelt repair` (a range) alike
/// — the caller folds either scope into `source_table`/`window_predicate`.
pub fn emit_succession_ledger_rebuild_select(
    source_table: &str,
    key_cols: &[String],
    clock_col: &str,
    pre_filter: Option<&str>,
    delete_flag_expr: &str,
    window_predicate: Option<&str>,
) -> MaintenanceStatement {
    let keys = key_col_list(key_cols);
    let mut predicate = delete_flag_expr.to_string();
    if let Some(pf) = pre_filter {
        predicate = format!("({pf}) AND ({predicate})");
    }
    if let Some(wp) = window_predicate {
        predicate = format!("({predicate}) AND ({wp})");
    }
    MaintenanceStatement::new(format!(
        "SELECT {keys}, {clock_col} FROM {source_table} WHERE {predicate}"
    ))
}

/// One `LEAD`/`LAG`-derived output column: `expr_template` names the raw
/// windowed value with the literal token `{lead}` (for a `lead_derived`
/// entry) or `{lag}` (for a `lag_derived` entry) — e.g. `("valid_to",
/// "{lead}")` or `("is_current", "{lead} IS NULL")`, mirroring the
/// dialect-emission registry's `Template` verdict convention
/// (`CLAUDE.md` §"Function-registry single ownership").
pub type DerivedColumn = (String, String);

/// The union-of-presented-rows/tombstone-ledger/batch relation every
/// neighbour lookup in this module runs over (`incremental_shapes.md`
/// §"The tombstone ledger (hidden state)": "the event sequence ... is the
/// union of that key's presented rows and its ledger rows"), scoped to the
/// keys the batch touches. Ledger rows carry `NULL` payload (a tombstone
/// has no presented row to carry columns on); `__smelt_is_delete` marks
/// which relation a row came from / the batch's own delete flag.
fn build_domain_cte(
    presented_table: &str,
    tombstone_table: &str,
    key_cols: &[String],
    clock_col: &str,
    payload_columns: &[String],
    delete_flag_expr: &str,
    event_delta_select: &str,
) -> String {
    let keys = key_col_list(key_cols);
    let payload_select = if payload_columns.is_empty() {
        String::new()
    } else {
        format!(", {}", payload_columns.join(", "))
    };
    let payload_null = if payload_columns.is_empty() {
        String::new()
    } else {
        format!(
            ", {}",
            payload_columns
                .iter()
                .map(|c| format!("NULL AS {c}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let touched = touched_keys_predicate(key_cols, event_delta_select);
    format!(
        "SELECT {keys}, {clock_col} AS __smelt_t{payload_select}, FALSE AS __smelt_is_delete \
         FROM {presented_table} WHERE {touched} \
         UNION ALL \
         SELECT {keys}, {clock_col} AS __smelt_t{payload_null}, TRUE AS __smelt_is_delete \
         FROM {tombstone_table} WHERE {touched} \
         UNION ALL \
         SELECT {keys}, {clock_col} AS __smelt_t{payload_select}, {delete_flag_expr} AS \
         __smelt_is_delete FROM ({event_delta_select}) AS __smelt_batch"
    )
}

/// The succession-patch keyed `MERGE` (`model_transforms.md`'s
/// "Succession-patch keyed `MERGE`" row): one transactional
/// [`StatementGroup`] — the idempotent tombstone insert (anti-join on
/// `(k, t)`) runs first, then the presented `MERGE` whose `USING`
/// recomputes `LEAD`/`LAG` over the neighbour domain, keyed on `(k, t)`.
///
/// `payload_columns` is every row-local, non-key/non-clock column the
/// presented table stores (excluding the lead/lag-derived columns, which
/// `lead_derived`/`lag_derived` describe). `delete_flag_expr` is `None`
/// when the model's grammar admits no delete filter (every event flows
/// straight to insert); `Some(expr)` names the batch's own delete-flag
/// expression, evaluated in the same scope `emit_succession_event_delta`
/// projected it into.
///
/// # Panics
/// DuckDB is the only dialect this phase renders correctly — Spark and
/// BigQuery take the recorded state downgrade
/// (`docs/outcomes/20260906-scd2-keyed-succession/outcome.md` §"Out of
/// scope") rather than half-right emitted text, so a non-DuckDB dialect
/// panics rather than silently compiling wrong SQL.
#[allow(clippy::too_many_arguments)]
pub fn emit_succession_patch(
    presented_table: &str,
    key_cols: &[String],
    clock_col: &str,
    payload_columns: &[String],
    lead_derived: &[DerivedColumn],
    lag_derived: &[DerivedColumn],
    delete_flag_expr: Option<&str>,
    event_delta_select: &str,
    dialect: MaintenanceDialect,
) -> StatementGroup {
    assert!(
        matches!(dialect, MaintenanceDialect::DuckDb),
        "emit_succession_patch: only MaintenanceDialect::DuckDb is supported today for \
         {presented_table} — Spark/BigQuery take the recorded state downgrade rather than a \
         half-right emitted MERGE"
    );
    let tombstone_table = tombstone_table_name(presented_table);
    let keys = key_col_list(key_cols);
    let delete_expr = delete_flag_expr.unwrap_or("FALSE");

    let domain = build_domain_cte(
        presented_table,
        &tombstone_table,
        key_cols,
        clock_col,
        payload_columns,
        delete_expr,
        event_delta_select,
    );

    let payload_select = if payload_columns.is_empty() {
        String::new()
    } else {
        format!(", {}", payload_columns.join(", "))
    };

    let derived_select = lead_derived
        .iter()
        .map(|(col, tmpl)| format!("{} AS {col}", tmpl.replace("{lead}", "__smelt_lead_t")))
        .chain(
            lag_derived
                .iter()
                .map(|(col, tmpl)| format!("{} AS {col}", tmpl.replace("{lag}", "__smelt_lag_t"))),
        )
        .collect::<Vec<_>>()
        .join(", ");
    let derived_select_part = if derived_select.is_empty() {
        String::new()
    } else {
        format!(", {derived_select}")
    };

    let using_select = format!(
        "WITH __smelt_domain AS ({domain}), \
         __smelt_dedup AS (SELECT * FROM __smelt_domain QUALIFY ROW_NUMBER() OVER (PARTITION BY \
         {keys}, __smelt_t ORDER BY __smelt_is_delete ASC) = 1), \
         __smelt_windowed AS (SELECT {keys}, __smelt_t{payload_select}, __smelt_is_delete, \
         LEAD(__smelt_t) OVER (PARTITION BY {keys} ORDER BY __smelt_t) AS __smelt_lead_t, \
         LAG(__smelt_t) OVER (PARTITION BY {keys} ORDER BY __smelt_t) AS __smelt_lag_t FROM \
         __smelt_dedup) \
         SELECT {keys}, __smelt_t{payload_select}, __smelt_is_delete{derived_select_part} FROM \
         __smelt_windowed"
    );

    let on = format!(
        "{} AND target.{clock_col} = source.__smelt_t",
        key_join_cond("target", "source", key_cols)
    );

    let mut set_cols: Vec<String> = payload_columns
        .iter()
        .map(|c| format!("{c} = source.{c}"))
        .collect();
    set_cols.extend(
        lead_derived
            .iter()
            .chain(lag_derived.iter())
            .map(|(col, _)| format!("{col} = source.{col}")),
    );
    let set = if set_cols.is_empty() {
        // Every succession model projects at least one lead/lag-derived
        // column by grammar (rule 1's window-function requirement), so
        // `set_cols` is never empty in practice — this branch exists only
        // so the emitter never constructs the syntactically invalid
        // `UPDATE SET` an empty list would produce.
        format!("{clock_col} = source.__smelt_t")
    } else {
        set_cols.join(", ")
    };

    let mut insert_cols: Vec<String> = key_cols.to_vec();
    insert_cols.push(clock_col.to_string());
    insert_cols.extend(payload_columns.iter().cloned());
    insert_cols.extend(
        lead_derived
            .iter()
            .chain(lag_derived.iter())
            .map(|(c, _)| c.clone()),
    );
    let insert_col_list = insert_cols.join(", ");
    let insert_values = key_cols
        .iter()
        .map(|k| format!("source.{k}"))
        .chain(std::iter::once("source.__smelt_t".to_string()))
        .chain(payload_columns.iter().map(|c| format!("source.{c}")))
        .chain(
            lead_derived
                .iter()
                .chain(lag_derived.iter())
                .map(|(c, _)| format!("source.{c}")),
        )
        .collect::<Vec<_>>()
        .join(", ");

    let merge_sql = format!(
        "MERGE INTO {presented_table} AS target USING ({using_select}) AS source ON {on} \
         WHEN MATCHED THEN UPDATE SET {set} \
         WHEN NOT MATCHED AND NOT source.__smelt_is_delete THEN INSERT ({insert_col_list}) \
         VALUES ({insert_values})"
    );

    let tombstone_insert = format!(
        "INSERT INTO {tombstone_table} ({keys}, {clock_col}) SELECT {keys}, {clock_col} FROM \
         ({event_delta_select}) AS __smelt_batch WHERE {delete_expr} AND NOT EXISTS (SELECT 1 \
         FROM {tombstone_table} AS __smelt_existing WHERE {} AND \
         __smelt_existing.{clock_col} = __smelt_batch.{clock_col})",
        key_join_cond("__smelt_existing", "__smelt_batch", key_cols)
    );

    StatementGroup {
        statements: vec![
            MaintenanceStatement::new(tombstone_insert),
            MaintenanceStatement::new(merge_sql),
        ],
        transactional: true,
    }
}

/// The clock-tie probe (`incremental_shapes.md` §"Run shape and late
/// events" — "Clock ties"): a read-only query the caller executes and
/// inspects *before* running [`emit_succession_patch`], so a violation is
/// caught without ever writing to the target — the same pattern
/// [`super::emit_recurrence_bound_probe`] uses. Fires when one `(k, t)`
/// resolves to more than one distinct `(row-local content, delete flag)`
/// pair across the presented rows, the tombstone ledger, and the batch; a
/// redelivered-identical row (same content and flag) is silent, matching
/// the run-shape's re-presentation rule.
pub fn emit_succession_clock_tie_probe(
    presented_table: &str,
    key_cols: &[String],
    clock_col: &str,
    payload_columns: &[String],
    delete_flag_expr: Option<&str>,
    event_delta_select: &str,
    dialect: MaintenanceDialect,
) -> MaintenanceStatement {
    let tombstone_table = tombstone_table_name(presented_table);
    let keys = key_col_list(key_cols);
    let delete_expr = delete_flag_expr.unwrap_or("FALSE");
    let domain = build_domain_cte(
        presented_table,
        &tombstone_table,
        key_cols,
        clock_col,
        payload_columns,
        delete_expr,
        event_delta_select,
    );
    let cast_type = super::probes::probe_dialect_string_type(dialect);
    let sig_expr = if payload_columns.is_empty() {
        "''".to_string()
    } else {
        payload_columns
            .iter()
            .map(|c| format!("COALESCE(CAST({c} AS {cast_type}), '')"))
            .collect::<Vec<_>>()
            .join(" || '|' || ")
    };
    let key_display = payload_columns_display(key_cols, cast_type);
    let sample_expr = clock_tie_sample_agg(dialect);
    let sql = format!(
        "WITH __smelt_domain AS ({domain}), __smelt_tie_violations AS (SELECT {key_display} AS \
         violation_key FROM __smelt_domain GROUP BY {keys}, __smelt_t HAVING COUNT(DISTINCT \
         CAST(__smelt_is_delete AS {cast_type}) || '|' || ({sig_expr})) > 1) SELECT COUNT(*) AS \
         violation_count, (SELECT {sample_expr} FROM (SELECT violation_key FROM \
         __smelt_tie_violations LIMIT 5) AS __smelt_sample) AS sample_keys FROM \
         __smelt_tie_violations"
    );
    MaintenanceStatement::new(sql)
}

fn payload_columns_display(columns: &[String], cast_type: &str) -> String {
    columns
        .iter()
        .map(|c| format!("CAST({c} AS {cast_type})"))
        .collect::<Vec<_>>()
        .join(" || '|' || ")
}

/// Duplicated from `super::probes`'s private `probe_dialect_sample_agg`
/// (module-private there, same "join up to 5 sampled `violation_key` values"
/// shape every probe in this crate uses) rather than widening that
/// function's visibility for one caller.
fn clock_tie_sample_agg(dialect: MaintenanceDialect) -> String {
    match dialect {
        MaintenanceDialect::DuckDb => "STRING_AGG(violation_key, ', ')".to_string(),
        MaintenanceDialect::Spark => "CONCAT_WS(', ', COLLECT_LIST(violation_key))".to_string(),
        MaintenanceDialect::BigQuery => "STRING_AGG(violation_key, ', ')".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Vec<String> {
        vec!["customer_id".to_string()]
    }

    #[test]
    fn tombstone_table_name_appends_the_reserved_suffix() {
        assert_eq!(
            tombstone_table_name("main.customer_history"),
            "main.customer_history__tombstones"
        );
    }

    #[test]
    fn event_delta_select_projects_row_local_columns_and_the_delete_flag_with_no_window_function() {
        let projection = vec![
            ("customer_id".to_string(), "customer_id".to_string()),
            ("changed_at".to_string(), "changed_at".to_string()),
            ("tier".to_string(), "tier".to_string()),
            ("is_deleted".to_string(), "is_deleted".to_string()),
        ];
        let stmt = emit_succession_event_delta(
            "raw.customer_changes",
            &projection,
            Some("ingested_at < changed_at + INTERVAL '7 days'"),
            "ingested_date >= DATE '2026-01-01' AND ingested_date < DATE '2026-01-02'",
        );
        assert!(!stmt.sql.contains("OVER ("), "{}", stmt.sql);
        assert!(
            stmt.sql
                .contains("ingested_at < changed_at + INTERVAL '7 days'"),
            "{}",
            stmt.sql
        );
        assert!(
            stmt.sql.contains(
                "ingested_date >= DATE '2026-01-01' AND ingested_date < DATE '2026-01-02'"
            ),
            "{}",
            stmt.sql
        );
        assert!(stmt.sql.starts_with(
            "SELECT customer_id AS customer_id, changed_at AS changed_at, tier AS tier, \
             is_deleted AS is_deleted FROM raw.customer_changes WHERE"
        ));
    }

    #[test]
    fn patch_group_is_transactional_and_records_tombstones_before_the_presented_merge() {
        let group = emit_succession_patch(
            "main.customer_history",
            &keys(),
            "changed_at",
            &["tier".to_string()],
            &[("valid_to".to_string(), "{lead}".to_string())],
            &[],
            Some("is_deleted"),
            "SELECT customer_id, changed_at, tier, is_deleted FROM raw.customer_changes",
            MaintenanceDialect::DuckDb,
        );
        assert!(group.transactional);
        assert_eq!(group.statements.len(), 2);
        assert!(
            group.statements[0]
                .sql
                .starts_with("INSERT INTO main.customer_history__tombstones"),
            "{}",
            group.statements[0].sql
        );
        assert!(
            group.statements[1].sql.starts_with("MERGE INTO"),
            "{}",
            group.statements[1].sql
        );
    }

    #[test]
    fn patch_merge_neighbour_domain_unions_presented_ledger_and_batch() {
        let group = emit_succession_patch(
            "main.customer_history",
            &keys(),
            "changed_at",
            &["tier".to_string()],
            &[("valid_to".to_string(), "{lead}".to_string())],
            &[],
            None,
            "SELECT customer_id, changed_at, tier FROM raw.customer_changes",
            MaintenanceDialect::DuckDb,
        );
        let merge_sql = &group.statements[1].sql;
        assert!(
            merge_sql.contains("FROM main.customer_history WHERE"),
            "{merge_sql}"
        );
        assert!(
            merge_sql.contains("FROM main.customer_history__tombstones WHERE"),
            "{merge_sql}"
        );
        assert!(
            merge_sql.contains(
                "FROM (SELECT customer_id, changed_at, tier FROM \
             raw.customer_changes) AS __smelt_batch"
            ),
            "{merge_sql}"
        );
        assert!(
            merge_sql.contains(
                "LEAD(__smelt_t) OVER (PARTITION BY customer_id ORDER BY \
             __smelt_t)"
            ),
            "{merge_sql}"
        );
        assert!(
            !merge_sql.contains(
                "LEAD(__smelt_t) OVER (PARTITION BY customer_id ORDER BY \
             __smelt_t) AS __smelt_lead_t FROM main.customer_history"
            ),
            "the LEAD/LAG recomputation must run over the domain union, not the presented table \
             alone: {merge_sql}"
        );
    }

    #[test]
    fn patch_merge_keys_on_key_columns_and_the_clock() {
        let group = emit_succession_patch(
            "main.customer_history",
            &keys(),
            "changed_at",
            &["tier".to_string()],
            &[("valid_to".to_string(), "{lead}".to_string())],
            &[],
            None,
            "SELECT customer_id, changed_at, tier FROM raw.customer_changes",
            MaintenanceDialect::DuckDb,
        );
        let merge_sql = &group.statements[1].sql;
        assert!(
            merge_sql.contains(
                "ON target.customer_id = source.customer_id AND target.changed_at = \
                 source.__smelt_t"
            ),
            "{merge_sql}"
        );
    }

    #[test]
    fn ledger_rebuild_select_is_key_and_clock_of_delete_flagged_rows_passing_the_pre_filter() {
        let stmt = emit_succession_ledger_rebuild_select(
            "raw.customer_changes",
            &keys(),
            "changed_at",
            Some("ingested_at < changed_at + INTERVAL '7 days'"),
            "is_deleted",
            None,
        );
        assert_eq!(
            stmt.sql,
            "SELECT customer_id, changed_at FROM raw.customer_changes WHERE (ingested_at < \
             changed_at + INTERVAL '7 days') AND (is_deleted)"
        );
    }

    #[test]
    fn clock_tie_probe_selects_key_clock_and_a_sample_for_non_identical_collisions() {
        let stmt = emit_succession_clock_tie_probe(
            "main.customer_history",
            &keys(),
            "changed_at",
            &["tier".to_string()],
            None,
            "SELECT customer_id, changed_at, tier FROM raw.customer_changes",
            MaintenanceDialect::DuckDb,
        );
        assert!(stmt.sql.contains("violation_count"), "{}", stmt.sql);
        assert!(stmt.sql.contains("sample_keys"), "{}", stmt.sql);
        assert!(stmt.sql.contains("HAVING COUNT(DISTINCT"), "{}", stmt.sql);
    }

    #[test]
    #[should_panic(expected = "only MaintenanceDialect::DuckDb is supported today")]
    fn emit_succession_patch_refuses_non_duckdb_dialects() {
        emit_succession_patch(
            "main.customer_history",
            &keys(),
            "changed_at",
            &["tier".to_string()],
            &[("valid_to".to_string(), "{lead}".to_string())],
            &[],
            None,
            "SELECT customer_id, changed_at, tier FROM raw.customer_changes",
            MaintenanceDialect::Spark,
        );
    }
}

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

/// The succession-patch technique was asked for statements in a dialect that
/// has no realisation of it.
///
/// Reaching this means a caller skipped the availability check
/// (`smelt_runtime::maintenance_driver::realises_tombstone_ledger`): the plan
/// layer downgrades a `SuccessionPatch` cell to `DeleteInsert` on a dialect
/// with no tombstone ledger, so a correct run never gets here. A typed
/// refusal rather than the `assert!` this used to be — `CLAUDE.md`
/// §"Fail-loud discipline" wants a diagnostic, not an abort.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "the succession-patch technique has no realisation in dialect '{dialect}': the tombstone \
     ledger is not realisable there, so the plan layer should have downgraded this cell to \
     DeleteInsert before the run reached a succession statement (docs/specs/state.md \
     §\"Which dialects realise which structure\")"
)]
pub struct UnsupportedSuccessionDialect {
    /// The offending dialect, as [`MaintenanceDialect`] spells it.
    pub dialect: &'static str,
}

impl UnsupportedSuccessionDialect {
    fn new(dialect: MaintenanceDialect) -> Self {
        Self {
            dialect: match dialect {
                MaintenanceDialect::DuckDb => "duckdb",
                MaintenanceDialect::Spark => "spark",
                MaintenanceDialect::BigQuery => "bigquery",
            },
        }
    }
}

/// The dialects that realise the tombstone ledger, and therefore the whole
/// succession-patch statement family (`docs/specs/state.md` §"Which dialects
/// realise which structure"). Spark is refused — Delta has no cross-table
/// transaction, so the tombstone record and the presented `MERGE` cannot be
/// made atomic, which is the same permanent absence `smelt_state::ledger`
/// records for the reconciliation ledger.
fn check_succession_dialect(
    dialect: MaintenanceDialect,
) -> Result<(), UnsupportedSuccessionDialect> {
    match dialect {
        MaintenanceDialect::DuckDb | MaintenanceDialect::BigQuery => Ok(()),
        MaintenanceDialect::Spark => Err(UnsupportedSuccessionDialect::new(dialect)),
    }
}

/// The scoping predicate restricting one neighbour-domain relation to the
/// keys the batch touches. Two spellings, one meaning:
///
/// - **DuckDB** — `(k…) IN (SELECT k… FROM (<batch>))`, a row-constructor
///   `IN` over the batch's own key projection.
/// - **BigQuery** — GoogleSQL has no row constructor: `(a, b)` is a
///   parenthesised expression there, not a tuple, so a multi-column `IN`
///   subquery is a syntax error (and the single-column case would be the
///   only one that happened to work). The same semi-join is spelled as a
///   correlated `EXISTS`, which needs the *outer* relation to carry an alias
///   so the correlation can name it — hence `outer_alias`, which
///   [`build_domain_cte`] supplies on the BigQuery path only.
///
/// Spark shares DuckDB's arm but never reaches it: every public entry point
/// refuses Spark via [`check_succession_dialect`] before any helper runs.
/// The arm exists so a new dialect is still a compile error here.
fn touched_keys_predicate(
    key_cols: &[String],
    event_delta_select: &str,
    dialect: MaintenanceDialect,
    outer_alias: &str,
) -> String {
    let keys = key_col_list(key_cols);
    match dialect {
        MaintenanceDialect::DuckDb | MaintenanceDialect::Spark => {
            format!(
                "({keys}) IN (SELECT {keys} FROM ({event_delta_select}) AS __smelt_touched_keys)"
            )
        }
        MaintenanceDialect::BigQuery => format!(
            "EXISTS (SELECT 1 FROM ({event_delta_select}) AS __smelt_touched_keys WHERE {})",
            key_join_cond("__smelt_touched_keys", outer_alias, key_cols)
        ),
    }
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
#[allow(clippy::too_many_arguments)]
fn build_domain_cte(
    presented_table: &str,
    tombstone_table: &str,
    key_cols: &[String],
    clock_col: &str,
    payload_columns: &[String],
    delete_flag_expr: &str,
    event_delta_select: &str,
    dialect: MaintenanceDialect,
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
    // The correlated-`EXISTS` spelling needs each scanned relation to carry a
    // name the correlation can qualify; the row-constructor `IN` spelling
    // needs none, and adding one would change DuckDB's emitted text for no
    // reason. So the alias is part of the per-dialect relation reference, not
    // a uniform addition.
    let (presented_ref, tombstone_ref, presented_alias, tombstone_alias) = match dialect {
        MaintenanceDialect::DuckDb | MaintenanceDialect::Spark => (
            presented_table.to_string(),
            tombstone_table.to_string(),
            "",
            "",
        ),
        MaintenanceDialect::BigQuery => (
            format!("{presented_table} AS __smelt_presented"),
            format!("{tombstone_table} AS __smelt_tombstones"),
            "__smelt_presented",
            "__smelt_tombstones",
        ),
    };
    let touched_presented =
        touched_keys_predicate(key_cols, event_delta_select, dialect, presented_alias);
    let touched_tombstone =
        touched_keys_predicate(key_cols, event_delta_select, dialect, tombstone_alias);
    format!(
        "SELECT {keys}, {clock_col} AS __smelt_t{payload_select}, FALSE AS __smelt_is_delete \
         FROM {presented_ref} WHERE {touched_presented} \
         UNION ALL \
         SELECT {keys}, {clock_col} AS __smelt_t{payload_null}, TRUE AS __smelt_is_delete \
         FROM {tombstone_ref} WHERE {touched_tombstone} \
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
/// # Errors
/// [`UnsupportedSuccessionDialect`] for a dialect with no tombstone-ledger
/// realisation (Spark). A cell on such a dialect takes the recorded state
/// downgrade to `DeleteInsert` in the plan layer and never reaches this
/// emitter; the refusal is the fail-loud backstop for a caller that skipped
/// the availability check.
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
) -> Result<StatementGroup, UnsupportedSuccessionDialect> {
    check_succession_dialect(dialect)?;
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
        dialect,
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

    // Two spellings of one relation — dedup the domain on `(k, t)`, then
    // recompute LEAD/LAG over it.
    //
    // DuckDB's uses `WITH` + `QUALIFY`. GoogleSQL has both, but this path
    // relies on neither: a `WITH` clause inside a `MERGE`'s `USING` subquery
    // and the exact preconditions BigQuery's `QUALIFY` carries are both
    // things only a live engine could settle, and this emitter's failure mode
    // is text the engine rejects on a path no offline test covers. Nested
    // derived tables with an explicit `ROW_NUMBER() … WHERE rn = 1` are the
    // universally-valid spelling of the same relation — the very shape
    // `emit_succession_full_rebuild`'s own fold already uses — so BigQuery
    // gets that instead of a construct whose acceptance is a guess.
    let using_select = match dialect {
        MaintenanceDialect::DuckDb | MaintenanceDialect::Spark => format!(
            "WITH __smelt_domain AS ({domain}), \
             __smelt_dedup AS (SELECT * FROM __smelt_domain QUALIFY ROW_NUMBER() OVER (PARTITION \
             BY {keys}, __smelt_t ORDER BY __smelt_is_delete ASC) = 1), \
             __smelt_windowed AS (SELECT {keys}, __smelt_t{payload_select}, __smelt_is_delete, \
             LEAD(__smelt_t) OVER (PARTITION BY {keys} ORDER BY __smelt_t) AS __smelt_lead_t, \
             LAG(__smelt_t) OVER (PARTITION BY {keys} ORDER BY __smelt_t) AS __smelt_lag_t FROM \
             __smelt_dedup) \
             SELECT {keys}, __smelt_t{payload_select}, __smelt_is_delete{derived_select_part} \
             FROM __smelt_windowed"
        ),
        MaintenanceDialect::BigQuery => format!(
            "SELECT {keys}, __smelt_t{payload_select}, __smelt_is_delete{derived_select_part} \
             FROM (SELECT {keys}, __smelt_t{payload_select}, __smelt_is_delete, \
             LEAD(__smelt_t) OVER (PARTITION BY {keys} ORDER BY __smelt_t) AS __smelt_lead_t, \
             LAG(__smelt_t) OVER (PARTITION BY {keys} ORDER BY __smelt_t) AS __smelt_lag_t FROM \
             (SELECT {keys}, __smelt_t{payload_select}, __smelt_is_delete FROM (SELECT {keys}, \
             __smelt_t{payload_select}, __smelt_is_delete, ROW_NUMBER() OVER (PARTITION BY \
             {keys}, __smelt_t ORDER BY __smelt_is_delete ASC) AS __smelt_dedup_rn FROM \
             ({domain}) AS __smelt_domain) AS __smelt_dedup_ranked WHERE __smelt_dedup_rn = 1) \
             AS __smelt_dedup) AS __smelt_windowed"
        ),
    };

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

    Ok(StatementGroup {
        statements: vec![
            MaintenanceStatement::new(tombstone_insert),
            MaintenanceStatement::new(merge_sql),
        ],
        transactional: true,
    })
}

/// The full-rebuild statement group (`incremental_shapes.md` §"The
/// tombstone ledger (hidden state)" — "Lifecycle"): `--full-refresh` and a
/// `smelt rebuild` range alike re-derive the presented table AND the
/// tombstone ledger from the whole source, in one transaction — the ledger
/// is a pure function of the whole retained source, so there is no
/// range-restricted form of it (see the spec paragraph this emitter
/// implements). `model_select_sql` is the model's own compiled SELECT (the
/// full-refresh oracle every other technique in this crate also uses for
/// its bootstrap arm); the presented arm reuses [`emit_create_table_as`]'s
/// spelling verbatim, so the caller is responsible for dropping any
/// pre-existing presented table before running this group (idempotent DDL,
/// not part of the transactional rebuild itself — mirroring
/// `execute_succession_maintenance`'s own precedent of running idempotent
/// DDL before its transactional write).
///
/// `output_columns` is the model's full resolved output schema, in the
/// model's own projection order — `key_cols` and `clock_col` included, so
/// the fold below can preserve that order rather than forcing a key-first
/// layout. A model's own column order need not put the key or clock first
/// (`examples/github_activity/models/silver/actor_naming.sql` projects
/// `actor_id, actor_login, created_at, ...` — the clock third); the
/// window-forward patch loop's bootstrap shell
/// (`emit_create_empty_table`) always creates the presented table in this
/// same model order, so a rebuild that instead emitted a key-first layout
/// would leave the two run shapes' presented tables column-order-divergent,
/// silently corrupting every position-based comparison of them (including
/// this crate's own `EXCEPT ALL` conformance oracles).
///
/// `lead_derived`/`lag_derived` are the same `(output_column,
/// `{lead}`/`{lag}`-templated expression)` pairs [`emit_succession_patch`]
/// takes — used here only to break a tie deterministically (see below), not
/// to recompute anything; the model's own compiled `LEAD`/`LAG` in
/// `model_select_sql` remains the sole source of every derived value.
///
/// `delete_flag_expr` is `"FALSE"` when the model's grammar admits no
/// delete filter, matching [`emit_succession_patch`]'s own default —
/// callers resolve `recipe.delete_flag_expr.as_deref().unwrap_or("FALSE")`
/// before calling, since this emitter (like
/// [`emit_succession_ledger_rebuild_select`]) takes the resolved
/// expression, not the `Option`.
///
/// # Errors
/// Same [`UnsupportedSuccessionDialect`] refusal as
/// [`emit_succession_patch`].
///
/// The group is marked `transactional`, which every dialect cannot equally
/// honour: it opens with a `CREATE TABLE … AS`, and GoogleSQL forbids
/// permanent-entity DDL inside a multi-statement transaction, so
/// `smelt_backend_bigquery`'s bookkeeping plan runs the three statements as
/// separate jobs. The rebuild stays *correct* — it is a pure function of the
/// whole retained source, so a re-run re-derives both tables from scratch —
/// but it is not atomic there, and `docs/specs/state.md` records that.
#[allow(clippy::too_many_arguments)]
pub fn emit_succession_full_rebuild(
    presented_table: &str,
    model_select_sql: &str,
    source_table: &str,
    key_cols: &[String],
    clock_col: &str,
    output_columns: &[String],
    lead_derived: &[DerivedColumn],
    lag_derived: &[DerivedColumn],
    pre_filter: Option<&str>,
    delete_flag_expr: &str,
    dialect: MaintenanceDialect,
) -> Result<StatementGroup, UnsupportedSuccessionDialect> {
    check_succession_dialect(dialect)?;
    let tombstone_table = tombstone_table_name(presented_table);
    let keys = key_col_list(key_cols);

    // Fold the model's raw compiled output on `(key_cols, clock_col)` — the
    // same addressing the patch loop's `MERGE ... ON` clause uses
    // (`docs/specs/incremental_shapes.md` §"The tombstone ledger (hidden
    // state)" — "Lifecycle") — rather than presenting one row per physically
    // duplicated tie. This picks one WHOLE physical row per group, never a
    // per-column aggregate: `LEAD`/`LAG` are computed over the model's own
    // physically-duplicated rows, so two tied rows can carry genuinely
    // different derived-column values (one row's `LEAD` sees the other tied
    // row as its own "next", a same-`t` artifact of ordering two identical
    // events; the other row correctly sees the true next event, or `NULL`).
    // A per-column `MAX`/`MIN` mixes these into a value no physical row ever
    // held — worse, a `NULL` (the correct "no true successor" case) loses to
    // any non-`NULL` artifact under either aggregate. Picking one row avoids
    // manufacturing new combinations. The tie-break prefers a row whose own
    // raw-passthrough (`{lead}`/`{lag}`, not further transformed) derived
    // columns do NOT equal the group's own clock value — the exact shape of
    // the same-`t` artifact above — falling back to an arbitrary stable pick
    // when a model has no such raw-passthrough column to signal by (any
    // genuine content disagreement within a tie is refused before this
    // statement runs by the clock-tie probe the caller runs over the same
    // scope, so an arbitrary pick among truly identical rows is safe). Every
    // non-key/clock column is projected in the model's own column position,
    // not moved ahead of the key/clock columns.
    let artifact_terms: Vec<String> = lead_derived
        .iter()
        .chain(lag_derived.iter())
        .filter(|(_, tmpl)| tmpl == "{lead}" || tmpl == "{lag}")
        .map(|(col, _)| format!("(CASE WHEN {col} = {clock_col} THEN 1 ELSE 0 END)"))
        .collect();
    let tie_break = if artifact_terms.is_empty() {
        "1".to_string()
    } else {
        artifact_terms.join(" + ")
    };
    let output_col_list = output_columns.join(", ");
    let folded_select = format!(
        "SELECT {output_col_list} FROM (SELECT *, ROW_NUMBER() OVER (PARTITION BY {keys}, \
         {clock_col} ORDER BY {tie_break} ASC) AS __smelt_rn FROM ({model_select_sql}) AS \
         __smelt_model) AS __smelt_ranked WHERE __smelt_rn = 1"
    );

    let presented_create = super::emit_create_table_as(presented_table, &folded_select, dialect);
    // GoogleSQL rejects a `DELETE` with no `WHERE` ("DELETE must have a WHERE
    // clause"); `WHERE TRUE` is its documented spelling for "every row".
    // DuckDB accepts both, and keeps the bare form so its emitted text is
    // unchanged.
    let ledger_delete = MaintenanceStatement::new(match dialect {
        MaintenanceDialect::DuckDb | MaintenanceDialect::Spark => {
            format!("DELETE FROM {tombstone_table}")
        }
        MaintenanceDialect::BigQuery => format!("DELETE FROM {tombstone_table} WHERE TRUE"),
    });
    let ledger_rebuild_select = emit_succession_ledger_rebuild_select(
        source_table,
        key_cols,
        clock_col,
        pre_filter,
        delete_flag_expr,
        None,
    );
    let ledger_insert = MaintenanceStatement::new(format!(
        "INSERT INTO {tombstone_table} ({keys}, {clock_col}) {}",
        ledger_rebuild_select.sql
    ));

    // `emit_create_table_as` always returns exactly one statement (its own
    // doc comment); matching rather than `.expect`-ing keeps this crate's
    // hardening-budget ratchet (`CLAUDE.md` §"Fail-loud discipline") from
    // counting a site that can never actually fail.
    let presented_stmt = match presented_create.statements.into_iter().next() {
        Some(stmt) => stmt,
        None => unreachable!("emit_create_table_as always returns exactly one statement"),
    };

    Ok(StatementGroup {
        statements: vec![presented_stmt, ledger_delete, ledger_insert],
        transactional: true,
    })
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
///
/// A delete row's signature is its flag *alone* — "against a stored
/// tombstone only the delete flag is comparable, since the ledger carries
/// no row-local content" (`incremental_shapes.md` §"Run shape and late
/// events"). [`build_domain_cte`]'s tombstone branch projects `NULL` for
/// every payload column, so comparing content would make every replay of a
/// tombstoned delete collide with itself (the ledger's NULL payload vs. the
/// replayed event's real payload) and fire a spurious tie on every refold
/// of a window containing a delete.
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
        dialect,
    );
    let cast_type = super::probes::probe_dialect_string_type(dialect);
    let content_sig = if payload_columns.is_empty() {
        "''".to_string()
    } else {
        payload_columns
            .iter()
            .map(|c| format!("COALESCE(CAST({c} AS {cast_type}), '')"))
            .collect::<Vec<_>>()
            .join(" || '|' || ")
    };
    let sig_expr = format!("CASE WHEN __smelt_is_delete THEN 'D' ELSE 'I|' || ({content_sig}) END");
    let key_display = payload_columns_display(key_cols, cast_type);
    let sample_expr = clock_tie_sample_agg(dialect);
    let sql = format!(
        "WITH __smelt_domain AS ({domain}), __smelt_tie_violations AS (SELECT {key_display} AS \
         violation_key FROM __smelt_domain GROUP BY {keys}, __smelt_t HAVING COUNT(DISTINCT \
         {sig_expr}) > 1) SELECT COUNT(*) AS violation_count, (SELECT {sample_expr} FROM \
         (SELECT violation_key FROM __smelt_tie_violations LIMIT 5) AS __smelt_sample) AS \
         sample_keys FROM __smelt_tie_violations"
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
mod tests;

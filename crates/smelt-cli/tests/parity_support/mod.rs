//! Shared comparison primitives for the `examples/github_activity/`
//! cross-target sweeps. Generalised over the target rather than duplicated
//! per target: one comparator, one landing seam, one manifest shape, shared
//! by every sweep below rather than restated per pair.
//!
//! The claims made about that pipeline are made by the **same** primitive
//! rather than by comparators that might disagree about what "equal" means:
//!
//! - `github_activity_dual_target.rs` — do DuckDB and BigQuery, and
//!   separately DuckDB and Databricks, compute the same answers?
//!   (`docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md` criterion 6;
//!   `docs/outcomes/20260912-databricks-dogfood-spine/outcome.md`
//!   criterion 7.)
//! - `github_activity_bq_oracle.rs` — does BigQuery's incrementally-maintained
//!   state equal BigQuery's own full refresh over the inputs seen so far?
//!   (criterion 7's BigQuery half; `docs/specs/incremental_models.md`
//!   §"The equivalence invariant".)
//!
//! # The comparator
//!
//! Whole-row multiset difference — `EXCEPT ALL` in both directions over
//! `SELECT *` — run inside DuckDB, exactly the primitive
//! `github_activity_oracle.rs`'s `relation_diff` already gates on. Deliberately
//! **not** `crates/smelt-cli/tests/common/mod.rs`'s `batches_to_sorted_rows`,
//! which stringifies every cell and so collapses type differences into
//! formatting differences, and gives an unactionable diff on a multi-thousand-row
//! relation.
//!
//! Relation discovery is generic — `information_schema` on each side, never a
//! hardcoded model list — and a relation present on only one side is a
//! **coverage failure** rather than a quietly smaller comparison. That is what
//! makes a sweep non-vacuous.
//!
//! Sides are named `left` and `right` here because the module serves both
//! claims; each caller supplies the labels its failure messages should use.
//!
//! A third claim reuses the same primitive on a third target:
//! `github_activity_dual_target.rs`'s Databricks leg — does DuckDB and
//! Databricks compute the same answers?
//! (`docs/outcomes/20260912-databricks-dogfood-spine/outcome.md` criterion 7.)
//!
//! # The landing seam
//!
//! An exported side reaches the comparator through one pluggable step
//! ([`load_exported_snapshot`]): export the rows to typed NDJSON
//! (`scripts/bq_dogfood_export.py` for BigQuery, `scripts/dbx_dogfood_export.py`
//! for Databricks), then cast each column into the **DuckDB leg's own**
//! declared type for that column, read from `information_schema.columns`. The
//! seam itself carries no BigQuery- or Databricks-specific branch — it assumes
//! only the **export encoding contract**: a TIMESTAMP-family column arrives as
//! epoch seconds (with a fractional part for sub-second precision), and every
//! other column arrives as text a `CAST` into the reference type accepts. Each
//! exporter is responsible for producing that encoding from its own source's
//! native types; this module only consumes it.
//!
//! ## Declared normalisation, and nothing else
//!
//! That cast is the **only** admissible normalisation: INT64→BIGINT,
//! FLOAT64→DOUBLE, NUMERIC→DECIMAL, TIMESTAMP→TIMESTAMP at UTC, DATE→DATE,
//! STRING→VARCHAR, BOOL→BOOLEAN. A source value that will not cast is a
//! **finding**, not a tolerance — the loader raises rather than coercing.
//!
//! Any tolerance beyond exact value equality — float epsilon, timestamp
//! truncation, string trimming — is a registered divergence with a root-caused
//! reason in the calling suite, never a quiet comparator setting. There is no
//! comparator knob to loosen.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Relation discovery
// ---------------------------------------------------------------------------

/// Never compared, by prefix:
/// - `sources_` — the raw loaded inputs. Both sides are populated from the
///   same sample by construction, so comparing them would measure the loader
///   rather than the pipeline.
/// - `_smelt_` — run bookkeeping (`_smelt_ledger`, `_smelt_observed_delta`).
///   They record *how* a run happened, not model state: an incremental run's
///   bookkeeping legitimately differs from a `--full-refresh` run's, and the
///   two targets legitimately realise different state structures
///   (`docs/specs/state.md`).
pub const EXCLUDED_PREFIXES: &[&str] = &["sources_", "_smelt_"];

/// Never compared, by suffix: `<model>__tombstones` is a keyed-succession
/// sibling — bookkeeping, for the same reason as `_smelt_*`.
pub const EXCLUDED_SUFFIXES: &[&str] = &["__tombstones"];

/// Never compared, by exact name:
/// - `github_events` / `github_events_arrival` — the BigQuery leg's physical
///   *source* tables (`models/sources/raw/*.yml` map `raw.github_events` onto
///   them via the target-aware `name:` override). They are the `sources_`
///   exclusion's BigQuery-side spelling.
/// - `_loader_days` — `load_day.sh`'s own idempotence bookkeeping on the
///   DuckDB leg.
pub const EXCLUDED_EXACT: &[&str] = &["github_events", "github_events_arrival", "_loader_days"];

/// The two models excluded from every BigQuery leg, so the relation sets are
/// equal by construction rather than by tolerance. Their absence from
/// BigQuery is a compile-time `UnsupportedOnBackend` refusal on GoogleSQL —
/// the `RANGE BETWEEN INTERVAL '2 days' PRECEDING` lookback frame, which
/// GoogleSQL allows only with numeric offsets — not a value divergence:
/// `silver.actor_sessions` carries the frame and
/// `marts.daily_active_contributors` is its only downstream consumer.
pub const BIGQUERY_EXCLUDED_MODELS: &[&str] =
    &["silver.actor_sessions", "marts.daily_active_contributors"];

/// The models excluded from every Databricks leg. Empty: phases 6b-6f closed
/// every construct Spark/Databricks refused (the hash-function spelling, the
/// drop-type-mismatch, the bare-VARCHAR cast target, `epoch_us`, and the
/// `LAG`/`LEAD` window frame), so the Databricks leg runs the whole 16-model
/// set.
pub const DATABRICKS_EXCLUDED_MODELS: &[&str] = &[];

pub fn is_compared(name: &str) -> bool {
    !EXCLUDED_PREFIXES.iter().any(|p| name.starts_with(p))
        && !EXCLUDED_SUFFIXES.iter().any(|s| name.ends_with(s))
        && !EXCLUDED_EXACT.contains(&name)
}

pub fn discover_relations(db: &Path) -> Vec<String> {
    let conn = duckdb::Connection::open(db).unwrap_or_else(|e| panic!("open {db:?}: {e}"));
    let mut stmt = conn
        .prepare(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = 'main' ORDER BY 1",
        )
        .expect("prepare relation discovery");
    let names: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query relation discovery")
        .collect::<Result<_, _>>()
        .expect("collect relation names");
    names.into_iter().filter(|n| is_compared(n)).collect()
}

// ---------------------------------------------------------------------------
// The comparator
// ---------------------------------------------------------------------------

/// Both databases attached read-only under the fixed aliases `left_db` and
/// `right_db`, so every SQL string in this module and in its callers' bound
/// checks spells the two sides the same way.
pub fn attached_conn(left_db: &Path, right_db: &Path) -> duckdb::Connection {
    let conn = duckdb::Connection::open_in_memory().expect("open in-memory duckdb");
    conn.execute_batch(&format!(
        "ATTACH '{}' AS left_db (READ_ONLY); ATTACH '{}' AS right_db (READ_ONLY);",
        left_db.display(),
        right_db.display()
    ))
    .expect("attach both databases");
    conn
}

pub fn scalar_on(conn: &duckdb::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0))
        .unwrap_or_else(|e| panic!("query failed: {e}\nSQL:\n{sql}"))
}

/// Up to 5 offending rows rendered as JSON, for a failure message.
pub fn sample_rows(conn: &duckdb::Connection, diff_sql: &str) -> Vec<String> {
    let sql = format!("SELECT to_json(d) FROM ({diff_sql}) AS d LIMIT 5");
    let mut stmt = conn
        .prepare(&sql)
        .unwrap_or_else(|e| panic!("prepare failed: {e}\nSQL:\n{sql}"));
    stmt.query_map([], |row| row.get::<_, String>(0))
        .unwrap_or_else(|e| panic!("query failed: {e}\nSQL:\n{sql}"))
        .collect::<Result<_, _>>()
        .unwrap_or_else(|e| panic!("collect sample rows failed: {e}\nSQL:\n{sql}"))
}

#[derive(Debug)]
pub struct RelationDiff {
    pub relation: String,
    pub left_rows: i64,
    pub right_rows: i64,
    pub left_only: i64,
    pub right_only: i64,
    pub left_only_sample: Vec<String>,
    pub right_only_sample: Vec<String>,
}

pub fn relation_diff(conn: &duckdb::Connection, relation: &str) -> RelationDiff {
    let left_only_sql = format!(
        "SELECT * FROM left_db.main.{relation} EXCEPT ALL SELECT * FROM right_db.main.{relation}"
    );
    let right_only_sql = format!(
        "SELECT * FROM right_db.main.{relation} EXCEPT ALL SELECT * FROM left_db.main.{relation}"
    );
    RelationDiff {
        relation: relation.to_string(),
        left_rows: scalar_on(
            conn,
            &format!("SELECT count(*) FROM left_db.main.{relation}"),
        ),
        right_rows: scalar_on(
            conn,
            &format!("SELECT count(*) FROM right_db.main.{relation}"),
        ),
        left_only: scalar_on(conn, &format!("SELECT count(*) FROM ({left_only_sql})")),
        right_only: scalar_on(conn, &format!("SELECT count(*) FROM ({right_only_sql})")),
        left_only_sample: sample_rows(conn, &left_only_sql),
        right_only_sample: sample_rows(conn, &right_only_sql),
    }
}

/// What the two sides are called in a failure message: `("duckdb",
/// "bigquery")` for the dual-target sweep, `("bq_incremental", "bq_oracle")`
/// for the equivalence sweep.
#[derive(Clone, Copy)]
pub struct SideLabels {
    pub left: &'static str,
    pub right: &'static str,
}

/// Relation-set totality first, then a whole-row multiset difference per
/// relation. `Err` names a relation present on only one side — coverage
/// totality, never a silently smaller comparison.
pub fn compare_databases(
    left_db: &Path,
    right_db: &Path,
    labels: SideLabels,
) -> Result<Vec<RelationDiff>, String> {
    let left_relations = discover_relations(left_db);
    let right_relations = discover_relations(right_db);
    let mut all: BTreeSet<String> = BTreeSet::new();
    all.extend(left_relations.iter().cloned());
    all.extend(right_relations.iter().cloned());

    for rel in &all {
        let on_left = left_relations.contains(rel);
        let on_right = right_relations.contains(rel);
        if !on_left || !on_right {
            let missing_from = if on_left { labels.right } else { labels.left };
            let (l, r) = (labels.left, labels.right);
            return Err(format!(
                "relation `{rel}` present on only one side ({l}={on_left}, \
                 {r}={on_right}) — missing from {missing_from}"
            ));
        }
    }

    let conn = attached_conn(left_db, right_db);
    Ok(all.iter().map(|rel| relation_diff(&conn, rel)).collect())
}

// ---------------------------------------------------------------------------
// The landing seam: exported rows (any target) -> a scratch DuckDB database
// ---------------------------------------------------------------------------

/// Build `out_db` holding one table per compared relation of `types_db`,
/// populated from `<ndjson_dir>/<relation>.ndjson`.
///
/// Each table is created as `SELECT * FROM types_db.<relation> WHERE false`, so
/// it carries **the DuckDB leg's own declared column names and types** — that
/// is the whole of the declared normalisation (see the module doc comment).
/// Each JSON field is then cast into that column: a TIMESTAMP column from the
/// exporter's epoch-seconds float via `make_timestamp` over microseconds (so
/// the conversion is exact and free of any session timezone — `to_timestamp`
/// would produce a TIMESTAMPTZ and re-interpret it locally), everything else
/// by a plain `CAST` of the extracted text. This is the **export encoding
/// contract** every exporter (`bq_dogfood_export.py`, `dbx_dogfood_export.py`)
/// must produce — it is what makes this seam target-agnostic rather than
/// BigQuery-specific.
///
/// A value that will not cast raises here rather than being coerced: an
/// exported value the DuckDB-side type cannot hold is a finding about the two
/// targets, not something for the comparator to absorb.
///
/// `types_db` is the **type reference**, not necessarily a comparison side:
/// the equivalence sweep lands two BigQuery snapshots and types both from the
/// same DuckDB database, so the two landed sides are byte-comparable by
/// construction.
pub fn load_exported_snapshot(types_db: &Path, ndjson_dir: &Path, out_db: &Path) {
    if out_db.exists() {
        std::fs::remove_file(out_db).unwrap_or_else(|e| panic!("remove {out_db:?}: {e}"));
    }
    let conn = duckdb::Connection::open(out_db).unwrap_or_else(|e| panic!("open {out_db:?}: {e}"));
    conn.execute_batch(&format!(
        "ATTACH '{}' AS types_db (READ_ONLY);",
        types_db.display()
    ))
    .expect("attach the type-reference database");

    for relation in discover_relations(types_db) {
        let columns = relation_columns(&conn, &relation);
        conn.execute_batch(&format!(
            "CREATE TABLE main.{relation} AS \
             SELECT * FROM types_db.main.{relation} WHERE false;"
        ))
        .unwrap_or_else(|e| panic!("create scratch table {relation}: {e}"));

        let file = ndjson_dir.join(format!("{relation}.ndjson"));
        let len = std::fs::metadata(&file)
            .unwrap_or_else(|e| panic!("stat {file:?}: {e}"))
            .len();
        if len == 0 {
            continue;
        }
        let projection: Vec<String> = columns
            .iter()
            .map(|(name, ty)| {
                let extracted = format!("json_extract_string(j.json, '$.\"{name}\"')");
                if ty.starts_with("TIMESTAMP") {
                    format!(
                        "make_timestamp(CAST(round(CAST({extracted} AS DOUBLE) * 1000000) \
                         AS BIGINT)) AS {name}"
                    )
                } else {
                    format!("CAST({extracted} AS {ty}) AS {name}")
                }
            })
            .collect();
        let sql = format!(
            "INSERT INTO main.{relation} SELECT {} FROM read_json_objects('{}') AS j",
            projection.join(", "),
            file.display()
        );
        conn.execute_batch(&sql)
            .unwrap_or_else(|e| panic!("load {relation} from {file:?}: {e}\nSQL:\n{sql}"));
    }
    conn.execute_batch("DETACH types_db;").expect("detach");
}

pub fn relation_columns(conn: &duckdb::Connection, relation: &str) -> Vec<(String, String)> {
    let mut stmt = conn
        .prepare(
            "SELECT column_name, data_type FROM information_schema.columns \
             WHERE table_catalog = 'types_db' AND table_schema = 'main' AND table_name = ? \
             ORDER BY ordinal_position",
        )
        .expect("prepare column discovery");
    stmt.query_map([relation], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })
    .expect("query column discovery")
    .collect::<Result<_, _>>()
    .expect("collect columns")
}

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root")
        .to_owned()
}

// ---------------------------------------------------------------------------
// The divergence vocabulary
// ---------------------------------------------------------------------------

/// Which of the two compared sides a [`DivergenceBound`] licenses to be
/// behind. Which side is which is the calling suite's choice, named in its
/// [`SideLabels`]: the dual-target sweep compares DuckDB (left) against
/// BigQuery (right), the equivalence sweep compares BigQuery's incremental
/// state (left) against its own full refresh (right). Both variants are kept
/// live by each suite's bound-vocabulary tests.
#[derive(PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// A registry entry's checkable bound. A bound never licenses a missing or
/// extra row — only a differing value, and only in columns it names
/// explicitly.
pub enum DivergenceBound {
    /// One target is always at or behind the other on `monotone_columns`
    /// (`behind_side` names the one never allowed to lead), and every row
    /// present on both targets matches exactly on every column in
    /// `exact_columns`.
    MonotoneDivergence {
        key_col: &'static str,
        exact_columns: &'static [&'static str],
        monotone_columns: &'static [&'static str],
        behind_side: Side,
    },
    /// The two legs' **sources** arrive in a different order, so at an
    /// intermediate window one leg's relation is the other's not-yet-arrived
    /// tail short. Admissible only for a relation that is a whole-source
    /// rebuild: `behind_side`'s rows must be a whole-row multiset **subset** of
    /// the other's, and every row only the leading side holds must fall on or
    /// after the behind side's own maximum event-time day — so the difference
    /// is exactly the tail the behind side has not loaded yet, never a row it
    /// lost from a day it has. A row missing from inside the behind side's own
    /// loaded range violates the bound and fails the sweep.
    ArrivalLag {
        event_time_column: &'static str,
        behind_side: Side,
    },
    /// The two sides may differ **without a direction** on
    /// `tolerant_columns` — unlike `MonotoneDivergence`, no ordering is
    /// enforced — while the row-key set matches exactly (no missing or extra
    /// row) and every column named in `exact_columns` matches exactly. For a
    /// mutable enrichment value one target freezes at write time rather than
    /// retroactively healing, so the two sides may show different historical
    /// values for the same row with no "ahead"/"behind" relationship a
    /// `MonotoneDivergence` could check (e.g. a string-valued column with no
    /// natural order).
    UnorderedColumnDivergence {
        key_col: &'static str,
        exact_columns: &'static [&'static str],
        tolerant_columns: &'static [&'static str],
    },
}

/// A registered difference between the two compared sides, carrying a
/// root-caused reason and a checkable bound. An unregistered non-zero diff
/// fails the sweep ([`check_agreement_against`]).
///
/// **The registry itself lives in the calling suite**, not here: it is that
/// suite's measured evidence, and the two suites' registries mean different
/// things (an engine-compatibility note versus a licence to depart from the
/// equivalence invariant, which carries a far higher bar). What is shared is
/// only the vocabulary and the checking.
///
/// An empty registry plus a vacuous sweep would be indistinguishable from
/// success, which is why [`check_agreement_against`] takes the registry as a
/// parameter: each suite's fail-closed control drives this real path over an
/// explicitly empty registry, so adding or removing an entry can never quietly
/// retire the control. The precedent is
/// `github_activity_oracle.rs::assert_matches_oracle_fails_closed_on_an_empty_registry`.
pub struct RegisteredDivergence {
    pub relation: &'static str,
    pub reason: &'static str,
    pub bound: DivergenceBound,
}

pub fn check_bound(
    left_db: &Path,
    right_db: &Path,
    entry: &RegisteredDivergence,
    labels: SideLabels,
) -> Result<(), String> {
    let conn = attached_conn(left_db, right_db);
    match &entry.bound {
        DivergenceBound::MonotoneDivergence {
            key_col,
            exact_columns,
            monotone_columns,
            behind_side,
        } => {
            let r = entry.relation;
            let key_only_duck = scalar_on(
                &conn,
                &format!(
                    "SELECT count(*) FROM left_db.main.{r} d WHERE NOT EXISTS \
                     (SELECT 1 FROM right_db.main.{r} b WHERE b.{key_col} = d.{key_col})"
                ),
            );
            let key_only_bq = scalar_on(
                &conn,
                &format!(
                    "SELECT count(*) FROM right_db.main.{r} b WHERE NOT EXISTS \
                     (SELECT 1 FROM left_db.main.{r} d WHERE d.{key_col} = b.{key_col})"
                ),
            );
            if key_only_duck != 0 || key_only_bq != 0 {
                return Err(format!(
                    "row-key-set mismatch on `{key_col}`: {key_only_duck} duckdb-only, \
                     {key_only_bq} bigquery-only — a divergence bound never licenses a \
                     missing or extra row"
                ));
            }
            for col in *exact_columns {
                let n = scalar_on(
                    &conn,
                    &format!(
                        "SELECT count(*) FROM left_db.main.{r} d \
                         JOIN right_db.main.{r} b USING ({key_col}) \
                         WHERE d.{col} IS DISTINCT FROM b.{col}"
                    ),
                );
                if n != 0 {
                    return Err(format!(
                        "{n} row(s) differ on `{col}`, which this bound does not license \
                         to diverge"
                    ));
                }
            }
            // `behind_side` names the target never allowed to *lead*, so a
            // violation is a row where that target's value is strictly ahead.
            let (behind, other, predicate) = match behind_side {
                Side::Right => (labels.right, labels.left, "b.{col} > d.{col}"),
                Side::Left => (labels.left, labels.right, "d.{col} > b.{col}"),
            };
            for col in *monotone_columns {
                let predicate = predicate.replace("{col}", col);
                let n = scalar_on(
                    &conn,
                    &format!(
                        "SELECT count(*) FROM left_db.main.{r} d \
                         JOIN right_db.main.{r} b USING ({key_col}) \
                         WHERE {predicate}"
                    ),
                );
                if n != 0 {
                    return Err(format!(
                        "{n} row(s) have the {behind} leg's `{col}` ahead of the {other} \
                         leg's — expected {behind} to never lead on a monotone column"
                    ));
                }
            }
            Ok(())
        }
        DivergenceBound::ArrivalLag {
            event_time_column,
            behind_side,
        } => {
            let r = entry.relation;
            let (behind, leading, behind_name, leading_name) = match behind_side {
                Side::Left => ("left_db", "right_db", labels.left, labels.right),
                Side::Right => ("right_db", "left_db", labels.right, labels.left),
            };
            // (1) The behind side holds nothing the leading side lacks.
            let behind_only_sql = format!(
                "SELECT * FROM {behind}.main.{r} EXCEPT ALL SELECT * FROM {leading}.main.{r}"
            );
            let behind_only =
                scalar_on(&conn, &format!("SELECT count(*) FROM ({behind_only_sql})"));
            if behind_only != 0 {
                return Err(format!(
                    "{behind_only} row(s) exist only on the {behind_name} leg, which this \
                     bound licenses to be strictly behind — an arrival lag never adds rows \
                     to the lagging side"
                ));
            }
            // (2) The rows only the leading side holds are the not-yet-arrived
            // tail, never a row lost from a day the behind side already has.
            let leading_only_sql = format!(
                "SELECT * FROM {leading}.main.{r} EXCEPT ALL SELECT * FROM {behind}.main.{r}"
            );
            let inside_loaded_range = scalar_on(
                &conn,
                &format!(
                    "SELECT count(*) FROM ({leading_only_sql}) x \
                     WHERE CAST(x.{event_time_column} AS DATE) < \
                       (SELECT max(CAST({event_time_column} AS DATE)) \
                        FROM {behind}.main.{r})"
                ),
            );
            if inside_loaded_range != 0 {
                return Err(format!(
                    "{inside_loaded_range} row(s) exist only on the {leading_name} leg with \
                     an `{event_time_column}` day the {behind_name} leg has already loaded \
                     — that is a lost row, not an arrival lag"
                ));
            }
            Ok(())
        }
        DivergenceBound::UnorderedColumnDivergence {
            key_col,
            exact_columns,
            tolerant_columns,
        } => {
            let r = entry.relation;
            let key_only_left = scalar_on(
                &conn,
                &format!(
                    "SELECT count(*) FROM left_db.main.{r} d WHERE NOT EXISTS \
                     (SELECT 1 FROM right_db.main.{r} b WHERE b.{key_col} = d.{key_col})"
                ),
            );
            let key_only_right = scalar_on(
                &conn,
                &format!(
                    "SELECT count(*) FROM right_db.main.{r} b WHERE NOT EXISTS \
                     (SELECT 1 FROM left_db.main.{r} d WHERE d.{key_col} = b.{key_col})"
                ),
            );
            if key_only_left != 0 || key_only_right != 0 {
                return Err(format!(
                    "row-key-set mismatch on `{key_col}`: {key_only_left} {}-only, \
                     {key_only_right} {}-only — a divergence bound never licenses a missing \
                     or extra row",
                    labels.left, labels.right
                ));
            }
            for col in *exact_columns {
                let n = scalar_on(
                    &conn,
                    &format!(
                        "SELECT count(*) FROM left_db.main.{r} d \
                         JOIN right_db.main.{r} b USING ({key_col}) \
                         WHERE d.{col} IS DISTINCT FROM b.{col}"
                    ),
                );
                if n != 0 {
                    return Err(format!(
                        "{n} row(s) differ on `{col}`, which this bound does not license \
                         to diverge — only {tolerant_columns:?} may differ"
                    ));
                }
            }
            // `tolerant_columns` are checked for nothing beyond membership in
            // the two rows already matched above: this bound licenses them to
            // differ arbitrarily, with no direction.
            Ok(())
        }
    }
}

/// The registry-consulting sweep: for every compared relation, either a
/// registered bound holds or the multiset difference is zero in both
/// directions.
///
/// The registry is a **parameter**, not a module constant, so each suite's
/// fail-closed control can drive this real code path over an **empty**
/// registry no matter what that suite's registry happens to hold today —
/// adding or removing an entry can never quietly retire the control.
pub fn check_agreement_against(
    left_db: &Path,
    right_db: &Path,
    window_label: &str,
    registry: &[RegisteredDivergence],
    labels: SideLabels,
) -> Result<Vec<RelationDiff>, String> {
    let diffs = compare_databases(left_db, right_db, labels)
        .map_err(|e| format!("window {window_label}: coverage failure: {e}"))?;
    for diff in &diffs {
        if let Some(entry) = registry.iter().find(|e| e.relation == diff.relation) {
            if let Err(msg) = check_bound(left_db, right_db, entry, labels) {
                return Err(format!(
                    "window {window_label}: registered divergence bound violated for `{}` \
                     (registered reason: {}): {msg}",
                    entry.relation, entry.reason
                ));
            }
        } else if diff.left_only != 0 || diff.right_only != 0 {
            return Err(format!(
                "window {window_label}: unregistered divergence in `{}` \
                 ({}_only={}, {}_only={})\n{}-only sample: {:?}\n\
                 {}-only sample: {:?}",
                diff.relation,
                labels.left,
                diff.left_only,
                labels.right,
                diff.right_only,
                labels.left,
                diff.left_only_sample,
                labels.right,
                diff.right_only_sample
            ));
        }
    }
    Ok(diffs)
}

// ---------------------------------------------------------------------------
// The parity manifest shape, shared by every sweep
// ---------------------------------------------------------------------------

/// One compared checkpoint: the DuckDB leg's database file at that window,
/// and the directory of per-relation NDJSON the other leg exported at the
/// same window. Produced by `scripts/bq-dogfood-parity.sh manifest` or
/// `scripts/dbx-dogfood-parity.sh manifest` — both sweeps deserialise the
/// same shape, so a manifest is not restated per target.
#[derive(serde::Deserialize)]
pub struct Checkpoint {
    pub label: String,
    pub window: i64,
    pub day: String,
    pub duck_db_path: PathBuf,
    pub ndjson_dir: PathBuf,
}

#[derive(serde::Deserialize)]
pub struct ParityManifest {
    pub checkpoints: Vec<Checkpoint>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The landing seam carries no BigQuery-specific branch: a synthetic
    /// NDJSON directory in the *declared export encoding* (a TIMESTAMP column
    /// as epoch seconds, everything else as text) lands byte-identically to
    /// its source table, whatever exporter produced it.
    #[test]
    fn landing_seam_is_target_agnostic() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let types_db = tmp.path().join("types.duckdb");
        let conn = duckdb::Connection::open(&types_db).unwrap_or_else(|e| panic!("open: {e}"));
        conn.execute_batch(
            "CREATE TABLE main.some_relation (id BIGINT, name VARCHAR, seen_at TIMESTAMP); \
             INSERT INTO main.some_relation VALUES (1, 'a', TIMESTAMP '2026-08-05 01:02:03');",
        )
        .expect("create source table");
        drop(conn);

        let ndjson_dir = tmp.path().join("export");
        std::fs::create_dir_all(&ndjson_dir).expect("mkdir");
        std::fs::write(
            ndjson_dir.join("some_relation.ndjson"),
            r#"{"id": 1, "name": "a", "seen_at": 1785891723.0}"#,
        )
        .expect("write ndjson");

        let out_db = tmp.path().join("landed.duckdb");
        load_exported_snapshot(&types_db, &ndjson_dir, &out_db);

        let landed_conn = attached_conn(&types_db, &out_db);
        let diffs = compare_databases(
            &types_db,
            &out_db,
            SideLabels {
                left: "types",
                right: "landed",
            },
        )
        .expect("relation sets must match");
        drop(landed_conn);
        let d = diffs
            .iter()
            .find(|d| d.relation == "some_relation")
            .expect("relation compared");
        assert_eq!(
            (d.left_only, d.right_only),
            (0, 0),
            "an exported row in the declared encoding must land byte-identical to its source"
        );
    }

    /// Both sweeps deserialise the same [`ParityManifest`] shape from this
    /// module — a compile-level guarantee plus one round-trip assertion.
    #[test]
    fn parity_manifest_shape_is_shared() {
        let json = r#"{"checkpoints": [
            {"label": "w01", "window": 1, "day": "2026-08-05",
             "duck_db_path": "/tmp/w01.duckdb", "ndjson_dir": "/tmp/w01"}
        ]}"#;
        let manifest: ParityManifest =
            serde_json::from_str(json).expect("parse a manifest in the shared shape");
        assert_eq!(manifest.checkpoints.len(), 1);
        assert_eq!(manifest.checkpoints[0].label, "w01");
        assert_eq!(manifest.checkpoints[0].window, 1);
    }
}

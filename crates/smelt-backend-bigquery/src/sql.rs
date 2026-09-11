//! Pure SQL generation functions for the BigQuery backend.
//!
//! All SQL strings sent to BigQuery are built here, making them independently
//! testable without a Python runtime or a live warehouse.

use smelt_backend::{partition_literal, PartitionRange};

/// Build a fully qualified, backtick-quoted table name: `` `project.dataset.table` ``.
///
/// GoogleSQL quotes identifiers with backticks, and a project id routinely
/// contains hyphens (`smelt-bq-test-20260816`), which are otherwise parsed as
/// subtraction — so the quoting is required, not cosmetic.
pub fn qualified_name(project: &str, dataset: &str, name: &str) -> String {
    format!("`{}.{}.{}`", project, dataset, name)
}

/// DROP TABLE IF EXISTS
pub fn drop_table(table_name: &str) -> String {
    format!("DROP TABLE IF EXISTS {}", table_name)
}

/// DROP VIEW IF EXISTS
pub fn drop_view(view_name: &str) -> String {
    format!("DROP VIEW IF EXISTS {}", view_name)
}

/// CREATE OR REPLACE TABLE ... AS SELECT
///
/// BigQuery supports `CREATE OR REPLACE TABLE` natively, so no
/// DROP-then-CREATE emulation is needed (unlike Spark).
pub fn create_table_as(table_name: &str, query: &str) -> String {
    format!("CREATE OR REPLACE TABLE {} AS {}", table_name, query)
}

/// CREATE OR REPLACE VIEW ... AS SELECT
pub fn create_view_as(view_name: &str, query: &str) -> String {
    format!("CREATE OR REPLACE VIEW {} AS {}", view_name, query)
}

/// CREATE OR REPLACE MATERIALIZED VIEW ... AS SELECT — the `refresh:
/// materialized_view` delegation target (`docs/specs/materialized_view.md`).
///
/// **Measured**, not guessed — `scripts/bigquery-probe-mv.sh`
/// (`docs/research/20260816-bigquery-backend.md` §"Materialized views"):
///
/// - `OR REPLACE` genuinely *replaces*: re-running the identical definition
///   is accepted, and swapping the aggregation itself (`SUM` → `COUNT(*)`)
///   changes the value the view serves. So one idempotent statement covers
///   both a plain re-run and a definition change — no drop-then-create
///   emulation, unlike `create_table_as`'s reason for using `CREATE OR
///   REPLACE TABLE` on other grounds. A plain `CREATE MATERIALIZED VIEW`
///   over an existing view fails `Already Exists`, which is why `OR
///   REPLACE` is the form emitted here.
/// - No `OPTIONS(...)` clause is emitted. `enable_refresh`,
///   `refresh_interval_minutes` and `max_staleness` are all accepted, but
///   refresh is **on by default**, so omitting the clause already gets the
///   engine-owned freshness this mode exists for. Emitting a knob here
///   would pre-empt the per-engine physical-strategy modifier that
///   `docs/specs/materialized_view.md` §Known Divergences deliberately
///   defers to a later mode.
pub fn create_materialized_view_as(view_name: &str, query: &str) -> String {
    format!(
        "CREATE OR REPLACE MATERIALIZED VIEW {} AS {}",
        view_name, query
    )
}

/// DROP MATERIALIZED VIEW IF EXISTS
///
/// **Measured** (`scripts/bigquery-probe-mv.sh`,
/// `docs/research/20260816-bigquery-backend.md` §"Materialized views"):
/// dropping the base *table* out from under a live materialized view is
/// accepted, so teardown needs no ordering — but `DROP TABLE IF EXISTS` and
/// `DROP VIEW IF EXISTS` both *fail* against a materialized view (`Cannot
/// drop ... which has type MATERIALIZED_VIEW. A table was expected.`);
/// `IF EXISTS` does not rescue a wrong-type object, because the object does
/// exist. That failure is new with this feature — no materialized view
/// could exist before it — so cleaning one up now genuinely needs this
/// statement: `BigQueryBackend::execute_model` issues it before delegating
/// to the ordinary table/view drop-and-create path, and
/// `create_materialized_view_as` implicitly needs no such call itself
/// (`CREATE OR REPLACE MATERIALIZED VIEW` handles that case per the "OR
/// REPLACE" note above).
pub fn drop_materialized_view(view_name: &str) -> String {
    format!("DROP MATERIALIZED VIEW IF EXISTS {}", view_name)
}

/// SELECT * FROM table LIMIT n
pub fn select_preview(table_name: &str, limit: usize) -> String {
    format!("SELECT * FROM {} LIMIT {}", table_name, limit)
}

/// INSERT INTO table SELECT ...
pub fn insert_into(table_name: &str, query: &str) -> String {
    format!("INSERT INTO {} {}", table_name, query)
}

/// DELETE over a half-open partition range `[start, end)`, rendered through
/// the single-owner axis renderer (quoted on the calendar axis, bare on the
/// integer axis).
pub fn delete_partitions_range(
    table_name: &str,
    partition: &PartitionRange,
) -> Result<String, String> {
    let start_lit = partition_literal(partition.axis, &partition.start)?;
    let end_lit = partition_literal(partition.axis, &partition.end)?;
    Ok(format!(
        "DELETE FROM {} WHERE {} >= {} AND {} < {}",
        table_name, partition.column, start_lit, partition.column, end_lit
    ))
}

/// Truncate SQL for log output.
pub fn truncate_sql(sql: &str) -> String {
    const MAX: usize = 200;
    if sql.len() <= MAX {
        sql.to_string()
    } else {
        format!("{}...", &sql[..MAX])
    }
}

/// Whether a BigQuery DDL failure means "the name exists, but as the wrong
/// kind of object" — the shape that a defensive drop-or-replace must treat
/// as *absent for its target kind*, not as a real failure.
///
/// Three call sites rely on this, all catalog-type collisions that only
/// exist because `refresh: materialized_view` lets a table, a view, and a
/// materialized view now share one logical model name across runs:
///
/// - `drop_view_if_exists` / `drop_table_if_exists`: the default
///   `Backend::execute_model` (`crates/smelt-backend/src/lib.rs`)
///   unconditionally issues both "in case the materialization type
///   changed". On DuckDB and Spark, `DROP VIEW IF EXISTS` against an
///   existing TABLE (or the reverse) is a no-op, honouring the `IF EXISTS`
///   contract; BigQuery instead raises a hard `400`. Measured live
///   2026-08-18 (`docs/specs/multi_backend.md` §Known Divergences):
///   `400 Cannot drop <project>:<dataset>.recipe_additive_agg which has
///   type TABLE. A view was expected.`
/// - `BigQueryBackend::create_materialized_view_as`'s defensive
///   drop-table/drop-view-first step (forward flip *into*
///   `materialized_view`): `CREATE OR REPLACE MATERIALIZED VIEW` is refused
///   outright when a TABLE already holds the name (measured via
///   `scripts/bigquery-probe-mv.sh`,
///   `docs/research/20260816-bigquery-backend.md` §"Materialized views":
///   `... is not allowed for this operation because it is currently a
///   TABLE.`), so the emitter drops first — but if the *existing* object is
///   itself already a materialized view, that defensive drop hits the next
///   bullet's failure instead, which must also be tolerated here (`CREATE
///   OR REPLACE MATERIALIZED VIEW` handles the actual replacement).
/// - `BigQueryBackend::execute_model`'s defensive
///   drop-materialized-view-first step (reverse flip *out of*
///   `materialized_view`): `DROP TABLE IF EXISTS` / `DROP VIEW IF EXISTS`
///   against an existing materialized view both fail — measured:
///   `Cannot drop ... which has type MATERIALIZED_VIEW. A table was
///   expected.` `IF EXISTS` does not rescue a wrong-type object, because
///   the object does exist.
///
/// This is an allow-list of verified (or, where noted, directly symmetric
/// but not yet independently observed) shapes, not a deny-list — the same
/// discipline `classify_bq_error` in `smelt-maintenance-testkit` uses for
/// the quota-refusal shape (`docs/specs/multi_backend.md` §"Measured
/// against the live warehouse"), and for the identical reason: a classifier
/// built as "not a 400 I recognise ⇒ treat as absent" would swallow
/// unrelated failures (bad SQL, missing dataset, permission errors) instead
/// of failing loud on them.
pub fn is_wrong_type_drop_failure(error_message: &str) -> bool {
    const WRONG_KIND_SHAPES: &[&str] = &[
        // Measured live 2026-08-18 — DROP VIEW IF EXISTS against a TABLE.
        "which has type TABLE. A view was expected",
        // Symmetric reverse (DROP TABLE IF EXISTS against a VIEW); not
        // independently observed, included defensively.
        "which has type VIEW. A table was expected",
        // Measured via scripts/bigquery-probe-mv.sh — DROP TABLE IF EXISTS
        // (and, symmetrically, DROP VIEW IF EXISTS) against a materialized
        // view.
        "which has type MATERIALIZED_VIEW. A table was expected",
        "which has type MATERIALIZED_VIEW. A view was expected",
        // Symmetric reverse (DROP MATERIALIZED VIEW IF EXISTS against an
        // ordinary table/view); not independently observed, included
        // defensively for `execute_model`'s unconditional defensive drop.
        "which has type TABLE. A materialized view was expected",
        "which has type VIEW. A materialized view was expected",
        // Measured via scripts/bigquery-probe-mv.sh — CREATE OR REPLACE
        // MATERIALIZED VIEW refused because the name is currently a TABLE.
        "is not allowed for this operation because it is currently a TABLE",
        // Symmetric reverse (currently a VIEW); not independently observed,
        // included defensively.
        "is not allowed for this operation because it is currently a VIEW",
    ];
    WRONG_KIND_SHAPES
        .iter()
        .any(|shape| error_message.contains(shape))
}

/// The ordered list of statements (each one query job) that realises
/// `Backend::execute_write_with_bookkeeping` on BigQuery.
///
/// The seam's contract is that `pre_write_sqls` and `write_group` share one
/// backend transaction while `ensure_sqls` run first and **outside** it. That
/// maps onto GoogleSQL as: one job per `ensure_sqls` entry, then one
/// multi-statement script holding the transaction.
///
/// Three GoogleSQL facts shape the script, each of which would otherwise be a
/// silent correctness hole:
///
/// - **DDL stays out of the transaction.** `ensure_sqls` is idempotent
///   `CREATE TABLE IF NOT EXISTS` DDL; the trait already documents keeping it
///   outside for exactly this reason, and BigQuery is the backend that makes
///   the precedent load-bearing.
/// - **Rollback is explicit.** BigQuery does not roll a script's transaction
///   back on its own when a statement fails mid-script, so the transaction is
///   wrapped in `BEGIN … EXCEPTION WHEN ERROR THEN ROLLBACK TRANSACTION;
///   RAISE …; END` — the documented GoogleSQL shape. The `RAISE` re-surfaces
///   the original error message, so a failure still reaches the caller as a
///   failure rather than being swallowed by the handler.
/// - **Statements are `;`-terminated.** A script is a statement list; the
///   emitters produce unterminated SQL, so this function terminates each one
///   (and tolerates an already-terminated statement rather than emitting `;;`).
///
/// Where there is **no** bookkeeping to bind (`pre_write_sqls` empty), no
/// transaction is opened: the write statements run as ordinary jobs, exactly
/// as the trait's default would. The transaction exists to make a bookkeeping
/// record and its write atomic, and with no record there is nothing to bind.
///
/// **And where the write itself is a `CREATE`, the transaction cannot hold
/// it.** BigQuery does not permit DDL creating or dropping permanent entities
/// inside a multi-statement transaction (the same documented fact
/// `BackendCapabilities::supports_transactional_ddl: false` records, and the
/// reason the additive fold refuses its first step outright). A maintained
/// model's *first* run writes `CREATE TABLE … AS` rather than a merge, so a
/// re-run-tolerant cell's ledger record would otherwise be bound into a script
/// the engine rejects. That case takes [`BookkeepingAtomicity::
/// NonAtomicCreatingWrite`]: no transaction, the write first and the
/// bookkeeping after, each its own job — and the caller **reports** the lost
/// atomicity rather than absorbing it (`docs/specs/state.md` §"The degradation
/// contract").
///
/// Two things make that ordering the right one rather than a coin flip:
///
/// - **Record-before-write is vacuous here.** The contract's ordering exists
///   because a record reads the target's *pre-write* state to compute the
///   changed-row set. A write that creates the target has no pre-write state
///   to read — the record's own query would reference a table that does not
///   exist yet — so nothing is lost by running it after.
/// - **The surviving exposure is the harmless direction.** A crash between the
///   two leaves the table created and the window unrecorded, so a re-run
///   redoes the window. The reverse (a bookkeeping record claiming a window
///   whose write never happened) is the direction that can mislead a later
///   run, and this ordering makes it impossible.
///
/// Only a leading `CREATE` is treated this way, because that is the only DDL
/// the maintenance driver ever puts in a write group (`create_group` is
/// `emit_create_table_as`). Any other DDL reaching a write group would still
/// be bound into the transaction and rejected by the engine — loudly, and
/// noted here rather than silently pre-empted, since inventing a degradation
/// for a shape nothing emits would be untested behaviour.
pub fn write_with_bookkeeping_plan(
    ensure_sqls: &[String],
    pre_write_sqls: &[String],
    write_sqls: &[String],
) -> BookkeepingPlan {
    let mut statements: Vec<String> = ensure_sqls.to_vec();
    if pre_write_sqls.is_empty() {
        statements.extend(write_sqls.iter().cloned());
        return BookkeepingPlan {
            statements,
            atomicity: BookkeepingAtomicity::NothingToBind,
        };
    }
    if write_sqls.iter().any(|s| creates_a_permanent_entity(s)) {
        statements.extend(write_sqls.iter().cloned());
        statements.extend(pre_write_sqls.iter().cloned());
        return BookkeepingPlan {
            statements,
            atomicity: BookkeepingAtomicity::NonAtomicCreatingWrite,
        };
    }
    let body = pre_write_sqls
        .iter()
        .chain(write_sqls.iter())
        .map(|s| format!("{};", s.trim().trim_end_matches(';').trim_end()))
        .collect::<Vec<_>>()
        .join("\n");
    statements.push(format!(
        "BEGIN\nBEGIN TRANSACTION;\n{}\nCOMMIT TRANSACTION;\nEXCEPTION WHEN ERROR THEN\n\
         ROLLBACK TRANSACTION;\nRAISE USING MESSAGE = @@error.message;\nEND;",
        body
    ));
    BookkeepingPlan {
        statements,
        atomicity: BookkeepingAtomicity::Transactional,
    }
}

/// The statements [`write_with_bookkeeping_plan`] produces, and how much
/// atomicity they actually buy. The second half is data, not a log line, so
/// the caller can report a degradation instead of silently absorbing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookkeepingPlan {
    /// The ordered statements, each one query job.
    pub statements: Vec<String>,
    /// What the plan guarantees about the bookkeeping/write pair.
    pub atomicity: BookkeepingAtomicity,
}

/// How much the statement plan binds the bookkeeping record to its write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookkeepingAtomicity {
    /// The record and the write share one multi-statement transaction — the
    /// seam's full contract.
    Transactional,
    /// There was no bookkeeping to bind; the write ran as ordinary jobs. Not
    /// a degradation — nothing was promised.
    NothingToBind,
    /// **Degraded.** The write group creates a permanent entity, which
    /// GoogleSQL forbids inside a transaction, so the write and the
    /// bookkeeping ran as separate jobs (write first). The caller must report
    /// this; the surviving exposure is a created table whose window is
    /// unrecorded, which costs a redundant re-run and never correctness.
    NonAtomicCreatingWrite,
}

/// Does this statement create a permanent entity — the one DDL shape a write
/// group produced by the maintenance driver can contain?
///
/// `CREATE TEMP`/`CREATE TEMPORARY` are excluded: BigQuery does permit those
/// inside a transaction, and they are never a maintained model's target.
fn creates_a_permanent_entity(sql: &str) -> bool {
    let head = sql.trim_start().to_ascii_uppercase();
    if !head.starts_with("CREATE") {
        return false;
    }
    let rest = head["CREATE".len()..].trim_start();
    let rest = rest
        .strip_prefix("OR REPLACE")
        .map(str::trim_start)
        .unwrap_or(rest);
    !(rest.starts_with("TEMP ") || rest.starts_with("TEMPORARY "))
}

/// The marker [`fold_ledger_delta_script`] raises when the fold's ledger
/// record turns out to be a repeat, and the *only* string
/// [`is_already_reflected`] matches on.
///
/// It is emitted and recognised in this one module on purpose: a recogniser
/// living apart from its emitter is the failure this seam could least afford —
/// drift would turn "refuse the repeat" into "fail the run with an
/// unclassified error", or worse, leave a repeat unrecognised. The pairing is
/// asserted by `sentinel_the_script_emits_is_the_sentinel_the_matcher_knows`.
///
/// Shaped so it cannot collide with user data reaching an error message:
/// screaming snake case, a `SMELT_` namespace prefix, and a token
/// (`ALREADY_REFLECTED`) that appears in no GoogleSQL keyword, no table name
/// and no smelt-emitted SQL.
pub const ALREADY_REFLECTED_SENTINEL: &str = "SMELT_LEDGER_ALREADY_REFLECTED";

/// The GoogleSQL script realising `Backend::fold_ledger_delta`'s
/// never-fold-twice contract (`docs/specs/incremental_models.md` §Constraints
/// "Never fold a delta already reflected in the state") on BigQuery.
///
/// **Why this exists at all.** On DuckDB the guarantee *is* a storage
/// constraint: the ledger's `PRIMARY KEY` is enforced, a repeat insert
/// violates it, and the violation aborts the transaction before the fold runs.
/// GoogleSQL's `PRIMARY KEY` is `NOT ENFORCED` — it documents row identity and
/// refuses nothing — so the same two statements would silently double-count an
/// additive fold. Here the refusal is re-expressed as an *effect* test:
/// `record_sql` (`smelt_state::ledger::ledger_fold_record_sql`, which on this
/// dialect is a `MERGE … WHEN NOT MATCHED THEN INSERT`) modifies one row the
/// first time and zero rows on a repeat, and `@@row_count = 0` aborts the
/// script before `action_sql` is ever reached.
///
/// ```text
/// BEGIN
/// BEGIN TRANSACTION;
/// <record_sql>;
/// IF @@row_count = 0 THEN
/// RAISE USING MESSAGE = '<sentinel>: …';
/// END IF;
/// <action_sql>;
/// COMMIT TRANSACTION;
/// EXCEPTION WHEN ERROR THEN
/// ROLLBACK TRANSACTION;
/// RAISE USING MESSAGE = @@error.message;
/// END;
/// ```
///
/// **The soundness argument, and exactly what it rests on.** Two concurrent
/// runs must not both see "absent" and both fold — the check-then-act race
/// that makes `Backend::fold_ledger_delta`'s documented best-effort default
/// (`exists` → `insert` → `action`, as three separate jobs) unacceptable as
/// BigQuery's realisation. BigQuery's multi-statement transactions are
/// documented to "guarantee ACID properties and support snapshot isolation",
/// and — the load-bearing sentence — "If a transaction mutates (updates or
/// deletes) rows in a table, then other transactions or DML statements that
/// mutate rows in the same table cannot run concurrently. Conflicting
/// transactions are cancelled." (BigQuery docs, "Multi-statement
/// transactions"). Both folds mutate `_smelt_ledger`, the same table, so they
/// cannot commit concurrently: one wins and the other is cancelled by the
/// engine. The winner's fold applies once; the loser is either cancelled (a
/// loud failure, never a silent second fold) or, on a later re-run, reads the
/// committed row and refuses here. The property this rests on is therefore
/// *conflict detection between mutating transactions on one table*, which is
/// stronger than the snapshot isolation it is usually stated alongside — the
/// refusal does not need to reason about read skew at all.
///
/// Two things offline evidence cannot settle, both inherited by phase 16 of
/// `docs/outcomes/20260906-bigquery-correctness`: that a live repeat really
/// surfaces this sentinel through the adapter's error text unmangled, and that
/// the engine's cancellation of a conflicting transaction is observed as a
/// failure rather than as a retry that quietly succeeds.
///
/// **Rollback is explicit.** BigQuery does not unwind a script's transaction
/// on its own when a statement fails mid-script, so the body is wrapped in
/// `BEGIN … EXCEPTION WHEN ERROR THEN ROLLBACK TRANSACTION; RAISE USING
/// MESSAGE = @@error.message; END` — the same shape
/// [`write_with_bookkeeping_plan`] uses. A `RAISE` inside the `BEGIN` section
/// is caught by that handler, which rolls the transaction back and re-raises
/// carrying the sentinel, so the refusal still reaches the caller as an error.
///
/// **No DDL may appear in `action_sql`.** BigQuery does not permit DDL that
/// creates or drops permanent entities inside a transaction, so a first-run
/// `CREATE TABLE … AS` action cannot be folded atomically here. That is
/// refused *upstream*, at the driver, keyed on
/// `BackendCapabilities::supports_transactional_ddl` — never discovered inside
/// this script.
pub fn fold_ledger_delta_script(record_sql: &str, action_sql: &str) -> String {
    let terminate = |s: &str| format!("{};", s.trim().trim_end_matches(';').trim_end());
    format!(
        "BEGIN\nBEGIN TRANSACTION;\n{}\nIF @@row_count = 0 THEN\n\
         RAISE USING MESSAGE = '{}: this delta is already recorded in the reconciliation \
         ledger; the fold was not applied';\nEND IF;\n{}\nCOMMIT TRANSACTION;\n\
         EXCEPTION WHEN ERROR THEN\nROLLBACK TRANSACTION;\n\
         RAISE USING MESSAGE = @@error.message;\nEND;",
        terminate(record_sql),
        ALREADY_REFLECTED_SENTINEL,
        terminate(action_sql),
    )
}

/// Does this BigQuery error message carry [`fold_ledger_delta_script`]'s
/// already-reflected sentinel?
///
/// The counterpart of `smelt_backend_duckdb`'s `is_constraint_violation`: it
/// answers "is this failure the ledger refusing a repeat, rather than a
/// genuine execution failure?", and it is the one place that answer is decided
/// on this backend. Deliberately a `contains` rather than an equality test —
/// the adapter wraps the raised message in BigQuery's own job-error envelope
/// (`400 Query error: …`), so the sentinel arrives embedded, not alone.
pub fn is_already_reflected(message: &str) -> bool {
    message.contains(ALREADY_REFLECTED_SENTINEL)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hyphenated project id must survive quoting — unquoted it parses as
    /// subtraction and the statement fails.
    #[test]
    fn qualified_name_backticks_hyphenated_project() {
        assert_eq!(
            qualified_name("smelt-bq-test-20260816", "smelt_test", "orders"),
            "`smelt-bq-test-20260816.smelt_test.orders`"
        );
    }

    #[test]
    fn create_table_as_uses_native_or_replace() {
        assert_eq!(
            create_table_as("`p.d.t`", "SELECT 1"),
            "CREATE OR REPLACE TABLE `p.d.t` AS SELECT 1"
        );
    }

    #[test]
    fn create_view_as_uses_or_replace() {
        assert_eq!(
            create_view_as("`p.d.v`", "SELECT 1"),
            "CREATE OR REPLACE VIEW `p.d.v` AS SELECT 1"
        );
    }

    /// Pins the measured `CREATE OR REPLACE MATERIALIZED VIEW` form
    /// (`docs/specs/materialized_view.md`, `scripts/bigquery-probe-mv.sh`).
    #[test]
    fn create_materialized_view_as_uses_or_replace_no_options() {
        assert_eq!(
            create_materialized_view_as("`p.d.mv`", "SELECT 1"),
            "CREATE OR REPLACE MATERIALIZED VIEW `p.d.mv` AS SELECT 1"
        );
    }

    /// Pins the `DROP MATERIALIZED VIEW IF EXISTS` form used by
    /// `BigQueryBackend::execute_model`'s reverse-flip cleanup.
    #[test]
    fn drop_materialized_view_emits_measured_form() {
        assert_eq!(
            drop_materialized_view("`p.d.mv`"),
            "DROP MATERIALIZED VIEW IF EXISTS `p.d.mv`"
        );
    }

    /// The exact error text observed live 2026-08-18 against a real
    /// warehouse (`docs/specs/multi_backend.md` §Known Divergences) must
    /// classify as "wrong object type" — this is the case that was
    /// crashing `column_add_between_runs_recovers_equivalence_on_bigquery`
    /// and `full_refresh_interleave_resets_state_correctly_on_bigquery`.
    #[test]
    fn wrong_type_matches_live_drop_view_against_table_error() {
        let msg = "400 Cannot drop project:dataset.recipe_additive_agg which has type TABLE. \
                    A view was expected.";
        assert!(is_wrong_type_drop_failure(msg));
    }

    /// The symmetric case: `DROP TABLE IF EXISTS` against an existing VIEW.
    /// Not yet observed live, but the same BigQuery error family with the
    /// object kinds swapped — included defensively.
    #[test]
    fn wrong_type_matches_drop_table_against_view_error() {
        let msg = "400 Cannot drop project:dataset.some_view which has type VIEW. \
                    A table was expected.";
        assert!(is_wrong_type_drop_failure(msg));
    }

    /// Measured via `scripts/bigquery-probe-mv.sh`
    /// (`docs/research/20260816-bigquery-backend.md` §"Materialized
    /// views"): `DROP TABLE IF EXISTS` against an existing materialized
    /// view.
    #[test]
    fn wrong_type_matches_drop_table_against_materialized_view_error() {
        let msg = "400 Cannot drop project:dataset.mv_model which has type \
                    MATERIALIZED_VIEW. A table was expected.";
        assert!(is_wrong_type_drop_failure(msg));
    }

    /// Symmetric to the above: `DROP VIEW IF EXISTS` against an existing
    /// materialized view. Not independently observed, included
    /// defensively.
    #[test]
    fn wrong_type_matches_drop_view_against_materialized_view_error() {
        let msg = "400 Cannot drop project:dataset.mv_model which has type \
                    MATERIALIZED_VIEW. A view was expected.";
        assert!(is_wrong_type_drop_failure(msg));
    }

    /// Measured via `scripts/bigquery-probe-mv.sh`: `CREATE OR REPLACE
    /// MATERIALIZED VIEW` refused because the name currently holds a TABLE.
    #[test]
    fn wrong_type_matches_create_materialized_view_over_table_error() {
        let msg = "400 CREATE OR REPLACE MATERIALIZED VIEW is not allowed for this operation \
                    because it is currently a TABLE.";
        assert!(is_wrong_type_drop_failure(msg));
    }

    /// An unrelated 400 (bad SQL) must NOT be classified as "wrong object
    /// type" — swallowing this would violate fail-loud discipline
    /// (CLAUDE.md §"Fail-loud discipline").
    #[test]
    fn wrong_type_does_not_match_unrelated_bad_request() {
        let msg = "400 Syntax error: Unexpected keyword FORM at [1:15]";
        assert!(!is_wrong_type_drop_failure(msg));
    }

    /// A 403 permission-denied error must NOT be classified as "wrong
    /// object type" either — a different failure family entirely.
    #[test]
    fn wrong_type_does_not_match_permission_denied() {
        let msg = "403 Access Denied: Dataset project:dataset: User does not have \
                    bigquery.tables.delete permission";
        assert!(!is_wrong_type_drop_failure(msg));
    }

    /// A quota-refusal error (a different classified shape, owned by
    /// `smelt-maintenance-testkit::classify_bq_error`) must NOT match here
    /// either — the two classifiers are disjoint by design.
    #[test]
    fn wrong_type_does_not_match_quota_refusal() {
        let msg = "400 Exceeded rate limits: too many table update operations for this \
                    table. exceeded quota for table update operations";
        assert!(!is_wrong_type_drop_failure(msg));
    }

    /// A generic "not found" error must NOT match either — the classifier
    /// is specifically about a *wrong-type* collision, not any DDL failure.
    #[test]
    fn wrong_type_does_not_match_not_found() {
        let msg = "404 Not found: Table project:dataset.does_not_exist";
        assert!(!is_wrong_type_drop_failure(msg));
    }

    /// The seam's contract, asserted against the recorded statement plan
    /// rather than a live warehouse: `ensure_sqls` run first, each as its own
    /// job and **outside** the transaction (BigQuery does not accept the
    /// idempotent DDL inside one), and the bookkeeping record and the write
    /// share exactly one transaction.
    #[test]
    fn bookkeeping_keeps_ensure_ddl_outside_one_transaction_with_the_write() {
        let plan = write_with_bookkeeping_plan(
            &["CREATE TABLE IF NOT EXISTS `ds._smelt_ledger` (a STRING)".to_string()],
            &[
                "MERGE `ds._smelt_ledger` T USING (SELECT 'm' AS a) S ON T.a = S.a WHEN NOT \
               MATCHED THEN INSERT (a) VALUES (S.a)"
                    .to_string(),
            ],
            &[
                "MERGE `ds.t` USING (SELECT 1) ON FALSE WHEN NOT MATCHED THEN INSERT ROW"
                    .to_string(),
            ],
        );
        let plan = plan.statements;
        assert_eq!(plan.len(), 2, "{plan:#?}");
        assert_eq!(
            plan[0],
            "CREATE TABLE IF NOT EXISTS `ds._smelt_ledger` (a STRING)"
        );
        assert!(!plan[0].contains("TRANSACTION"), "{}", plan[0]);

        let script = &plan[1];
        assert_eq!(script.matches("BEGIN TRANSACTION;").count(), 1, "{script}");
        assert_eq!(script.matches("COMMIT TRANSACTION;").count(), 1, "{script}");
        assert!(script.contains("_smelt_ledger` T USING"), "{script}");
        assert!(script.contains("MERGE `ds.t`"), "{script}");
        assert!(
            !script.contains("CREATE TABLE IF NOT EXISTS"),
            "the idempotent DDL must not be inside the transaction: {script}"
        );
        // The record is before the write — it reads pre-write target state.
        let record_at = script.find("_smelt_ledger` T USING").expect("record");
        let write_at = script.find("MERGE `ds.t`").expect("write");
        assert!(record_at < write_at, "{script}");
    }

    /// A failure mid-script must roll the transaction back and still surface
    /// as a failure — the handler re-raises rather than swallowing the error.
    #[test]
    fn bookkeeping_rolls_back_explicitly_and_re_raises() {
        let plan = write_with_bookkeeping_plan(
            &[],
            &["INSERT INTO x VALUES (1)".to_string()],
            &["INSERT INTO y VALUES (2)".to_string()],
        );
        assert_eq!(plan.atomicity, BookkeepingAtomicity::Transactional);
        let script = &plan.statements[0];
        assert!(script.starts_with("BEGIN\nBEGIN TRANSACTION;"), "{script}");
        assert!(script.contains("EXCEPTION WHEN ERROR THEN"), "{script}");
        assert!(script.contains("ROLLBACK TRANSACTION;"), "{script}");
        assert!(
            script.contains("RAISE USING MESSAGE = @@error.message;"),
            "{script}"
        );
        assert!(script.ends_with("END;"), "{script}");
    }

    /// Every statement in the script is `;`-terminated exactly once — a
    /// script is a statement list, and `;;` is a syntax error.
    #[test]
    fn bookkeeping_terminates_each_statement_once() {
        let plan = write_with_bookkeeping_plan(
            &[],
            &["INSERT INTO x VALUES (1);".to_string()],
            &["INSERT INTO y VALUES (2)".to_string()],
        );
        let plan = plan.statements;
        assert!(!plan[0].contains(";;"), "{}", plan[0]);
        assert!(
            plan[0].contains("INSERT INTO x VALUES (1);\n"),
            "{}",
            plan[0]
        );
        assert!(
            plan[0].contains("INSERT INTO y VALUES (2);\n"),
            "{}",
            plan[0]
        );
    }

    /// With nothing to bind, no transaction is opened — the plan is the
    /// trait default's own sequence.
    #[test]
    fn bookkeeping_opens_no_transaction_when_there_is_no_record() {
        let plan = write_with_bookkeeping_plan(
            &["CREATE TABLE IF NOT EXISTS `ds.t` (a STRING)".to_string()],
            &[],
            &["INSERT INTO `ds.t` VALUES ('a')".to_string()],
        );
        assert_eq!(plan.atomicity, BookkeepingAtomicity::NothingToBind);
        assert_eq!(
            plan.statements,
            vec![
                "CREATE TABLE IF NOT EXISTS `ds.t` (a STRING)".to_string(),
                "INSERT INTO `ds.t` VALUES ('a')".to_string(),
            ]
        );
    }

    /// A first run's write group is a `CREATE TABLE … AS`, and GoogleSQL does
    /// not permit DDL on a permanent entity inside a transaction — so binding
    /// the bookkeeping record to it would produce a script the engine rejects.
    /// The plan degrades instead: no transaction, write first, record after,
    /// and the lost atomicity is *reported* rather than absorbed.
    #[test]
    fn a_creating_write_group_is_not_bound_into_a_transaction() {
        let plan = write_with_bookkeeping_plan(
            &["CREATE TABLE IF NOT EXISTS `ds._smelt_ledger` (a STRING)".to_string()],
            &[
                "MERGE `ds._smelt_ledger` T USING (SELECT 'm' AS a) S ON T.a = S.a WHEN NOT \
               MATCHED THEN INSERT (a) VALUES (S.a)"
                    .to_string(),
            ],
            &["CREATE OR REPLACE TABLE `ds.t` AS SELECT 1 AS a".to_string()],
        );
        assert_eq!(
            plan.atomicity,
            BookkeepingAtomicity::NonAtomicCreatingWrite,
            "the degradation must be reported, not silent: {plan:#?}"
        );
        assert!(
            plan.statements.iter().all(|s| !s.contains("TRANSACTION")),
            "GoogleSQL rejects permanent-entity DDL inside a transaction: {:#?}",
            plan.statements
        );
        assert_eq!(plan.statements.len(), 3, "{:#?}", plan.statements);
        assert!(plan.statements[0].starts_with("CREATE TABLE IF NOT EXISTS `ds._smelt_ledger`"));
        assert!(
            plan.statements[1].starts_with("CREATE OR REPLACE TABLE `ds.t`"),
            "the write runs first — a created table with an unrecorded window costs a re-run; \
             the reverse could mislead a later run: {:#?}",
            plan.statements
        );
        assert!(plan.statements[2].starts_with("MERGE `ds._smelt_ledger`"));
    }

    /// Non-vacuity for the case above: an ordinary DML write group with the
    /// same bookkeeping still gets the full transaction.
    #[test]
    fn a_dml_write_group_still_shares_one_transaction_with_its_record() {
        let plan = write_with_bookkeeping_plan(
            &[],
            &[
                "MERGE `ds._smelt_ledger` T USING (SELECT 'm' AS a) S ON T.a = S.a WHEN NOT \
               MATCHED THEN INSERT (a) VALUES (S.a)"
                    .to_string(),
            ],
            &[
                "MERGE `ds.t` USING (SELECT 1) ON FALSE WHEN NOT MATCHED THEN INSERT ROW"
                    .to_string(),
            ],
        );
        assert_eq!(plan.atomicity, BookkeepingAtomicity::Transactional);
        assert_eq!(plan.statements.len(), 1);
        assert!(plan.statements[0].contains("BEGIN TRANSACTION;"));
    }

    /// A temporary table is not a permanent entity, and BigQuery does allow
    /// it inside a transaction — so it must not trip the degradation.
    #[test]
    fn a_temp_create_is_not_treated_as_permanent_ddl() {
        assert!(creates_a_permanent_entity(
            "CREATE OR REPLACE TABLE `ds.t` AS SELECT 1"
        ));
        assert!(creates_a_permanent_entity("create table `ds.t` (a STRING)"));
        assert!(!creates_a_permanent_entity(
            "CREATE TEMP TABLE t AS SELECT 1"
        ));
        assert!(!creates_a_permanent_entity(
            "CREATE OR REPLACE TEMPORARY TABLE t AS SELECT 1"
        ));
        assert!(!creates_a_permanent_entity(
            "MERGE `ds.t` USING (SELECT 1) ON FALSE WHEN NOT MATCHED THEN INSERT ROW"
        ));
    }

    // ── never-fold-twice (`fold_ledger_delta_script`) ────────────────────

    /// The emitter and the recogniser must never drift apart: the string the
    /// script raises is the string [`is_already_reflected`] matches, asserted
    /// in one place so a rename of either half fails here rather than silently
    /// downgrading a refusal into an unclassified execution failure.
    #[test]
    fn sentinel_the_script_emits_is_the_sentinel_the_matcher_knows() {
        let script = fold_ledger_delta_script(
            "MERGE `ds._smelt_ledger` T USING (SELECT 'm' AS model_name) S ON T.model_name = \
             S.model_name WHEN NOT MATCHED THEN INSERT (model_name) VALUES (S.model_name)",
            "MERGE INTO `ds.t` USING (SELECT 1)",
        );
        assert!(
            script.contains(ALREADY_REFLECTED_SENTINEL),
            "the script must raise the sentinel: {script}"
        );
        // The shape the adapter actually hands back: BigQuery's job-error
        // envelope wrapped around the raised message.
        let live_shaped_error = format!(
            "400 Query error: {}: this delta is already recorded in the reconciliation ledger; \
             the fold was not applied at [4:1]",
            ALREADY_REFLECTED_SENTINEL
        );
        assert!(is_already_reflected(&live_shaped_error));
        assert!(!is_already_reflected(
            "404 Not found: Table project:dataset.orders"
        ));
    }

    /// The atomicity shape, asserted against the pure builder rather than a
    /// warehouse: the record, the zero-row abort and the action all sit inside
    /// exactly one `BEGIN TRANSACTION … COMMIT TRANSACTION`, in that order,
    /// and the abort is positioned so the action cannot be reached on a
    /// repeat.
    #[test]
    fn record_abort_and_action_share_exactly_one_transaction_in_that_order() {
        let script = fold_ledger_delta_script(
            "MERGE `ds._smelt_ledger` T USING (SELECT 'm' AS model_name) S ON T.model_name = \
             S.model_name WHEN NOT MATCHED THEN INSERT (model_name) VALUES (S.model_name)",
            "MERGE INTO `ds.t` USING (SELECT 1)",
        );
        assert_eq!(script.matches("BEGIN TRANSACTION;").count(), 1, "{script}");
        assert_eq!(script.matches("COMMIT TRANSACTION;").count(), 1, "{script}");
        assert_eq!(
            script.matches("ROLLBACK TRANSACTION;").count(),
            1,
            "{script}"
        );

        let begin = script.find("BEGIN TRANSACTION;").unwrap();
        let record = script.find("_smelt_ledger").unwrap();
        let abort = script.find("IF @@row_count = 0 THEN").unwrap();
        let action = script.find("MERGE INTO `ds.t`").unwrap();
        let commit = script.find("COMMIT TRANSACTION;").unwrap();
        assert!(
            begin < record && record < abort && abort < action && action < commit,
            "record → abort → action must all fall inside the one transaction: {script}"
        );
        assert!(
            script.contains("EXCEPTION WHEN ERROR THEN\nROLLBACK TRANSACTION;"),
            "BigQuery does not unwind a script's transaction on its own: {script}"
        );
    }

    /// Statements arrive unterminated from the emitters and must be
    /// `;`-terminated exactly once — an already-terminated statement must not
    /// become `;;`, which is a script syntax error.
    #[test]
    fn fold_script_terminates_each_statement_exactly_once() {
        let script = fold_ledger_delta_script("MERGE `ds._smelt_ledger` X;", "MERGE INTO `ds.t` Y");
        assert!(!script.contains(";;"), "{script}");
        assert!(script.contains("MERGE `ds._smelt_ledger` X;\n"), "{script}");
        assert!(script.contains("MERGE INTO `ds.t` Y;\n"), "{script}");
    }
}

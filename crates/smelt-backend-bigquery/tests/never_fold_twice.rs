//! **The headline: an additive fold never applies twice on BigQuery.**
//!
//! On DuckDB this guarantee is a storage constraint — the reconciliation
//! ledger's `PRIMARY KEY` is enforced, a repeat insert violates it, and the
//! violation aborts the transaction before the fold action runs. GoogleSQL's
//! `PRIMARY KEY` is `NOT ENFORCED`; it raises nothing. So on BigQuery the
//! refusal is re-expressed as an *effect* test inside the same multi-statement
//! transaction: the ledger record is a `MERGE … WHEN NOT MATCHED`, which
//! modifies one row the first time and zero on a repeat, and `@@row_count = 0`
//! raises before `action_sql` is reached
//! (`smelt_backend_bigquery::sql::fold_ledger_delta_script`).
//!
//! This file proves that offline, with no warehouse, by driving the **real**
//! builders — `smelt_state::ledger::ledger_fold_record_sql` for the record and
//! `sql::fold_ledger_delta_script` for the script — through a small GoogleSQL
//! script executor that models exactly the three engine behaviours the refusal
//! depends on, and nothing else:
//!
//! 1. `MERGE … WHEN NOT MATCHED THEN INSERT` inserts iff the key is absent,
//!    and sets `@@row_count` to the number of rows it modified;
//! 2. `IF @@row_count = 0 THEN RAISE USING MESSAGE = '…'` stops the script
//!    with that message;
//! 3. the `EXCEPTION WHEN ERROR THEN ROLLBACK TRANSACTION; RAISE …` handler
//!    discards everything the aborted transaction did and re-raises.
//!
//! The executor reads the emitted script text rather than being told what the
//! script contains, so a change to the script's shape shows up here as a
//! failure rather than as a test that quietly stops testing anything. What it
//! cannot prove — that a live BigQuery agrees about `@@row_count` after a
//! `MERGE`, and that the sentinel survives the adapter's error envelope — is
//! phase 16's, and is stated in `fold_ledger_delta_script`'s doc comment.

use smelt_backend_bigquery::sql;
use smelt_dialect::SqlDialect;
use smelt_state::ledger;

const SCHEMA: &str = "smelt_dogfood";
const MODEL: &str = "gold.events_rollup";
const GROUP: &str = "{*}";
const INPUT: &str = "smelt.raw_events";

/// The action an additive fold folds — a merge, never DDL. A `CREATE TABLE …
/// AS` first-run action is refused upstream by the driver, because BigQuery
/// cannot hold permanent-entity DDL inside a transaction.
const ACTION_SQL: &str = "MERGE INTO `smelt_dogfood.events_rollup` T USING (SELECT 1) S \
                          ON T.k = S.k WHEN MATCHED THEN UPDATE SET T.n = T.n + S.n";

/// A fake warehouse: the ledger's four-column key set, plus the statements it
/// was asked to run.
#[derive(Default)]
struct FakeWarehouse {
    ledger_rows: Vec<String>,
    statement_log: Vec<String>,
}

/// What running one script did.
#[derive(Debug)]
enum ScriptOutcome {
    Committed,
    /// The script raised; carries the message an adapter would surface.
    Raised(String),
}

impl FakeWarehouse {
    /// Execute one GoogleSQL script of the exact shape
    /// [`sql::fold_ledger_delta_script`] emits, modelling only the three
    /// behaviours the refusal depends on.
    fn run_script(&mut self, script: &str) -> ScriptOutcome {
        assert!(
            script.contains("BEGIN TRANSACTION;") && script.contains("COMMIT TRANSACTION;"),
            "the script must open and close exactly one transaction: {script}"
        );

        // Everything the transaction did, held back until COMMIT — the
        // EXCEPTION handler's ROLLBACK discards it.
        let mut pending_rows = self.ledger_rows.clone();
        let mut pending_log = Vec::new();
        let mut row_count: usize = 0;

        let body = script
            .split_once("BEGIN TRANSACTION;\n")
            .expect("transaction body")
            .1;
        let body = body
            .split_once("COMMIT TRANSACTION;")
            .expect("transaction body ends at COMMIT")
            .0;

        let mut lines = body.lines().peekable();
        while let Some(line) = lines.next() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if line.contains("_smelt_ledger") {
                // The ledger record. Its key is the four literals the builder
                // put in the statement; read them back out of the text rather
                // than assuming them.
                let key = ledger_key_of(line);
                pending_log.push(line.to_string());
                if line.starts_with("MERGE ") {
                    // `MERGE … WHEN NOT MATCHED THEN INSERT`: inserts iff
                    // absent, and reports how many rows it modified.
                    if pending_rows.contains(&key) {
                        row_count = 0;
                    } else {
                        pending_rows.push(key);
                        row_count = 1;
                    }
                } else {
                    // A plain `INSERT`. GoogleSQL's `PRIMARY KEY` is
                    // `NOT ENFORCED`, so a duplicate key raises **nothing** and
                    // the row simply lands a second time — this is the engine
                    // behaviour that makes DuckDB's spelling unusable here, and
                    // modelling it honestly is what makes the headline test
                    // fail if the BigQuery fold record ever becomes an INSERT.
                    assert!(
                        line.starts_with("INSERT INTO"),
                        "unknown ledger DML: {line}"
                    );
                    pending_rows.push(key);
                    row_count = 1;
                }
            } else if line == "IF @@row_count = 0 THEN" {
                let raise = lines
                    .next()
                    .expect("RAISE follows the IF")
                    .trim()
                    .to_string();
                let end_if = lines.next().expect("END IF follows the RAISE").trim();
                assert_eq!(end_if, "END IF;", "unexpected IF body: {raise}");
                if row_count == 0 {
                    // The EXCEPTION handler catches it: ROLLBACK, then
                    // re-raise with @@error.message.
                    return ScriptOutcome::Raised(raise_message(&raise));
                }
            } else {
                // The fold action.
                pending_log.push(line.trim_end_matches(';').to_string());
            }
        }

        self.ledger_rows = pending_rows;
        self.statement_log.extend(pending_log);
        ScriptOutcome::Committed
    }
}

/// The four key literals a ledger record names, in order — read out of the
/// emitted statement rather than restated here, so the fake warehouse keys on
/// what the builder actually wrote. Handles both realisable spellings: the
/// `MERGE`'s one-row `SELECT` source and a plain `INSERT … VALUES`.
fn ledger_key_of(ledger_sql: &str) -> String {
    let source = if let Some((_, rest)) = ledger_sql.split_once("USING (SELECT ") {
        rest.split_once(") S ON ")
            .expect("source ends before the ON clause")
            .0
    } else {
        ledger_sql
            .split_once(") VALUES (")
            .expect("INSERT names its values")
            .1
            .trim_end_matches(')')
    };
    source
        .split(", ")
        .take(4)
        .map(|literal| {
            // `'value'` in an INSERT, `'value' AS column` in the MERGE source.
            literal.split_once(" AS ").map_or(literal, |(v, _)| v)
        })
        .collect::<Vec<_>>()
        .join("|")
}

/// The text a `RAISE USING MESSAGE = '…'` raises.
fn raise_message(raise_stmt: &str) -> String {
    raise_stmt
        .split_once("USING MESSAGE = '")
        .expect("RAISE carries a message")
        .1
        .trim_end_matches(';')
        .trim_end_matches('\'')
        .to_string()
}

/// Build the script the BigQuery backend would run for one fold, from the
/// production builders.
fn fold_script(delta_id: &str) -> String {
    let record_sql = ledger::ledger_fold_record_sql(
        SqlDialect::BigQuery,
        SCHEMA,
        MODEL,
        GROUP,
        INPUT,
        delta_id,
        delta_id,
        "2026-01-02",
    )
    .expect("BigQuery realises the reconciliation ledger");
    sql::fold_ledger_delta_script(&record_sql, ACTION_SQL)
}

/// **The row's named guarantee.** Fold the same
/// `(model, group, input, delta_id)` twice. The first fold applies. The second
/// is refused with the already-reflected sentinel, and — the half that makes
/// it a correctness claim rather than an error-message claim — the fold action
/// does not appear in the warehouse's statement log a second time.
#[test]
fn folding_the_same_delta_twice_refuses_and_never_applies_the_action_again() {
    let mut warehouse = FakeWarehouse::default();

    match warehouse.run_script(&fold_script("2026-01-01")) {
        ScriptOutcome::Committed => {}
        other => panic!("the first fold must commit, got {other:?}"),
    }
    let after_first = warehouse.statement_log.len();
    assert_eq!(
        warehouse
            .statement_log
            .iter()
            .filter(|s| s.contains("events_rollup` T USING"))
            .count(),
        1,
        "the first fold applies exactly once: {:?}",
        warehouse.statement_log
    );

    let outcome = warehouse.run_script(&fold_script("2026-01-01"));
    let ScriptOutcome::Raised(message) = outcome else {
        panic!("a repeat fold must be refused, got {outcome:?}");
    };
    assert!(
        sql::is_already_reflected(&message),
        "the refusal must be recognisable as already-reflected: {message}"
    );
    assert_eq!(
        warehouse.statement_log.len(),
        after_first,
        "the repeat fold must apply NOTHING — not the ledger record, not the action: {:?}",
        warehouse.statement_log
    );
    assert_eq!(
        warehouse
            .statement_log
            .iter()
            .filter(|s| s.contains("events_rollup` T USING"))
            .count(),
        1,
        "the additive fold must never be applied twice: {:?}",
        warehouse.statement_log
    );
}

/// Non-vacuity: the refusal is keyed on the delta identity, not on "a fold
/// already happened". A *different* delta folds normally.
#[test]
fn a_different_delta_folds_normally() {
    let mut warehouse = FakeWarehouse::default();
    for delta_id in ["2026-01-01", "2026-01-02", "2026-01-03"] {
        match warehouse.run_script(&fold_script(delta_id)) {
            ScriptOutcome::Committed => {}
            other => panic!("delta {delta_id} must fold, got {other:?}"),
        }
    }
    assert_eq!(warehouse.ledger_rows.len(), 3);
    assert_eq!(
        warehouse
            .statement_log
            .iter()
            .filter(|s| s.contains("events_rollup` T USING"))
            .count(),
        3
    );
}

/// The refusal must not depend on the process's own memory of what it folded:
/// a *fresh* run against a ledger that already holds the row refuses just the
/// same. This is the case the check-then-act default gets wrong under
/// concurrency and the case a re-run actually hits.
#[test]
fn a_fresh_run_against_an_already_recorded_delta_refuses() {
    let mut first = FakeWarehouse::default();
    first.run_script(&fold_script("2026-01-01"));

    // A new run, same warehouse state, no memory of the first.
    let mut second = FakeWarehouse {
        ledger_rows: first.ledger_rows.clone(),
        statement_log: Vec::new(),
    };
    let outcome = second.run_script(&fold_script("2026-01-01"));
    let ScriptOutcome::Raised(message) = outcome else {
        panic!("a re-run of a recorded delta must be refused, got {outcome:?}");
    };
    assert!(sql::is_already_reflected(&message), "{message}");
    assert!(
        second.statement_log.is_empty(),
        "nothing may be applied: {:?}",
        second.statement_log
    );
}

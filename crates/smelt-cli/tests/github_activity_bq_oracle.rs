#![cfg(feature = "duckdb")]
//! The equivalence invariant, checked on BigQuery: after each incremental
//! window, does each model's incrementally-maintained state equal a **full
//! refresh over the inputs seen so far**?
//! (`docs/specs/incremental_models.md` §"The equivalence invariant";
//! `docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md` criterion 7.)
//!
//! # What this suite is and is not
//!
//! It is **not** a second dual-target parity sweep. `github_activity_dual_target.rs`
//! asks "do two engines agree with each other"; this asks "does one engine's
//! incremental state equal its own full refresh". Both sides here run on
//! BigQuery. A non-zero diff in a compared cell is a **violation of the
//! equivalence invariant** on a real pipeline, and the bar for registering
//! rather than fixing it is correspondingly higher (see
//! [`EQUIVALENCE_DIVERGENCE_REGISTRY`]).
//!
//! # Where the oracle is a valid oracle
//!
//! The invariant is `incremental_state(S) == full_refresh(inputs ∈ S)`, so the
//! oracle must refresh over **the inputs seen so far** — and on BigQuery the
//! source tables statically hold all thirty days from before the first window.
//! A model whose full refresh bounds its own scan to the requested window
//! therefore has a valid oracle at every checkpoint; a model whose full refresh
//! reads the whole source does not, at an intermediate one, because its inputs
//! are then a strict superset of what the incremental leg had seen. Those are
//! named in [`UNBOUNDED_REFRESH_RELATIONS`], exempted only at intermediate
//! checkpoints, and only with the mechanism *measured* rather than asserted.
//!
//! **At the final window nothing is exempt**, because there the inputs seen so
//! far are the whole source — which makes window 30 the complete statement of
//! the invariant over the fixture's entire thirty-window run sequence, across
//! all fourteen relations.
//!
//! The DuckDB half of criterion 7 is **not** restated here. It is
//! `github_activity_oracle::every_window_matches_the_full_refresh_oracle`,
//! which replays all thirty windows of the committed fixture and compares every
//! materialised relation against a full-refresh oracle — over the same
//! population BigQuery runs on, since phase 17 widened the BigQuery source to
//! exactly that fixture. There is deliberately no second DuckDB oracle.
//!
//! # How a comparison is made
//!
//! Both sides are BigQuery relations, exported to typed NDJSON by
//! `scripts/bq-dogfood-parity.sh` and landed into DuckDB by the **same**
//! primitive the dual-target sweep uses (`bq_parity_support`), typed from the
//! same DuckDB reference database, then differenced whole-row with `EXCEPT ALL`
//! in both directions. One comparator, two claims.
//!
//! # Coverage, stated rather than implied
//!
//! - **Fourteen of sixteen models.** `silver.actor_sessions` and
//!   `marts.daily_active_contributors` are excluded from every leg — a
//!   compile-time `UnsupportedOnBackend` refusal on GoogleSQL's INTERVAL
//!   `RANGE` frame, not a value divergence. Criterion 7's BigQuery half does
//!   not cover them.
//! - **Seven of thirty windows.** The checkpoints are the set phase 13 measured
//!   and declared (1, 2, 3, 5, 10, 20, 30), so a phase-13 claim and a phase-14
//!   claim at the same window are about the same state.
//! - **Bookkeeping is excluded by decision, not by absence.** `_smelt_ledger`,
//!   `_smelt_observed_delta` and the two `__tombstones` tables now exist on
//!   BigQuery. They record *how* a run happened, and an incremental run's
//!   bookkeeping legitimately differs from a `--full-refresh` run's, so they
//!   are excluded exactly as the DuckDB oracle already excludes them.

use std::collections::BTreeSet;
use std::path::PathBuf;

#[path = "bq_parity_support/mod.rs"]
mod bq_parity_support;
use bq_parity_support::{
    check_agreement_against, compare_databases, load_bigquery_snapshot, repo_root,
    RegisteredDivergence, RelationDiff, SideLabels, EXCLUDED_MODELS,
};

/// The two sides of *this* sweep: BigQuery's incrementally-maintained state,
/// and BigQuery's own full refresh over the inputs seen so far.
const SIDES: SideLabels = SideLabels {
    left: "incremental",
    right: "oracle",
};

/// The committed project's third target, which the oracle leg runs on.
const ORACLE_TARGET: &str = "bigquery_oracle";
/// The scratch dataset that target writes into — created without a default
/// table expiration and dropped when the phase ends. Scaffolding, not history.
const ORACLE_SCHEMA: &str = "smelt_dogfood_oracle";
/// The dataset holding the pipeline's incrementally-maintained state and, in
/// `github_events` / `github_events_arrival`, the **shared** source tables both
/// legs read.
const SHARED_DATASET: &str = "smelt_dogfood";

fn example_dir() -> PathBuf {
    repo_root().join("examples/github_activity")
}

fn example_config() -> smelt_core::config::Config {
    let path = example_dir().join("smelt.yml");
    serde_yaml::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}")),
    )
    .unwrap_or_else(|e| panic!("parse {path:?}: {e}"))
}

// ---------------------------------------------------------------------------
// The anti-vacuity gate on the oracle target's source resolution
// ---------------------------------------------------------------------------

/// **The gate this phase exists to hold.** The oracle leg must read the *same
/// physical source tables* as the incremental leg, not a copy and emphatically
/// not an empty table.
///
/// Without a `bigquery_oracle:` entry in each source's target-aware `name:`
/// map, `SourceInfo::db_name_for_target` falls back to the default mapping
/// `<target_schema>.<address_segments.join("_")>` — which on the oracle target
/// is `smelt_dogfood_oracle.sources_raw_github_events`, a table nothing ever
/// creates. The oracle would then refresh over zero rows and the whole sweep
/// would pass vacuously: incremental state compared against an oracle built
/// from nothing. That is the failure mode this phase is here to rule out, so it
/// is asserted through the real resolver rather than by eyeballing the YAML.
#[test]
fn the_oracle_target_resolves_sources_to_the_shared_tables() {
    let config = example_config();
    let infos = smelt_core::discover_source_infos(&example_dir(), &config.paths);

    for (segments, expected) in [
        (
            ["sources", "raw", "github_events"],
            "smelt_dogfood.github_events",
        ),
        (
            ["sources", "raw", "github_events_arrival"],
            "smelt_dogfood.github_events_arrival",
        ),
    ] {
        let info = infos
            .iter()
            .find(|s| s.address_segments == segments)
            .unwrap_or_else(|| panic!("no source declared at {segments:?}"));

        let resolved = info.db_name_for_target(ORACLE_TARGET, ORACLE_SCHEMA);
        assert_eq!(
            resolved,
            expected,
            "the `{ORACLE_TARGET}` target must resolve `{}` to the shared source table — \
             otherwise the full-refresh oracle reads an empty table and the equivalence \
             sweep passes vacuously",
            segments.join(".")
        );
        assert!(
            resolved.starts_with(&format!("{SHARED_DATASET}.")),
            "the oracle leg must read from `{SHARED_DATASET}`, not from its own scratch \
             dataset: got `{resolved}`"
        );

        let default_mapping = format!("{ORACLE_SCHEMA}.{}", segments.join("_"));
        assert_ne!(
            resolved, default_mapping,
            "the oracle target fell back to the default source mapping — this is exactly \
             the vacuous pass the `bigquery_oracle:` entry exists to prevent"
        );
    }
}

/// Adding a third target must not move the no-`--target` default. `target: dev`
/// is pinned in `smelt.yml` precisely because the fallback when it is unset is
/// the alphabetically-first target name (`smelt-runtime/src/profile.rs`), and
/// `bigquery` and now `bigquery_oracle` both sort ahead of `dev`.
#[test]
fn adding_the_oracle_target_does_not_move_the_default() {
    let config = example_config();

    assert!(
        config.targets.contains_key(ORACLE_TARGET),
        "the committed project must declare the `{ORACLE_TARGET}` target"
    );

    // The same resolution `smelt-runtime/src/profile.rs` performs: an explicit
    // `target:` wins, otherwise the alphabetically-first target name.
    let alphabetically_first = {
        let mut names: Vec<&String> = config.targets.keys().collect();
        names.sort();
        names
            .first()
            .map(|s| s.to_string())
            .expect("the project declares at least one target")
    };
    let resolved = config
        .target
        .clone()
        .unwrap_or_else(|| alphabetically_first.clone());

    assert_eq!(
        resolved, "dev",
        "the no-`--target` default must stay `dev`; the oracle target is only ever \
         selected with `--target {ORACLE_TARGET}`"
    );
    assert_ne!(
        alphabetically_first, "dev",
        "the `target: dev` pin has stopped being load-bearing — if `dev` ever sorts first \
         on its own, this test no longer proves the pin holds the default in place"
    );
}

// ---------------------------------------------------------------------------
// The divergence registry for this sweep
// ---------------------------------------------------------------------------

/// **Empty, and the bar for adding to it is high.** An entry here would not be
/// an engine-compatibility note: it would license a model's incrementally
/// maintained state to differ from its own full refresh, which is the promise
/// `docs/specs/incremental_models.md` §"The equivalence invariant" makes. An
/// entry must name the maintenance technique, the model, and the mechanism by
/// which the incremental plan legitimately lags — the shape
/// [`bq_parity_support::DivergenceBound::MonotoneDivergence`] encodes. Anything
/// that cannot be stated in those terms is a defect for
/// `20260906-bigquery-correctness`, recorded as a finding, never registered
/// away.
///
/// An empty registry plus a vacuous sweep would be indistinguishable from
/// success, so the registry-consulting path is driven over a real mismatch by
/// [`an_unregistered_equivalence_violation_fails`] and
/// [`the_equivalence_sweep_fails_closed_on_an_empty_registry`], and the
/// committed report is checked for unregistered divergence by
/// [`equivalence_registry_entries_are_all_live`].
const EQUIVALENCE_DIVERGENCE_REGISTRY: &[RegisteredDivergence] = &[];

// ---------------------------------------------------------------------------
// Where the oracle is well-defined, and where it is not
// ---------------------------------------------------------------------------

/// A relation whose **full refresh reads the whole source**, not the requested
/// event-time window.
///
/// This is not a divergence and is emphatically not a licence for the
/// incremental side to be wrong — it is a statement about what the *oracle*
/// computes, and it narrows where the oracle is a valid oracle at all.
///
/// The invariant is `incremental_state(S) == full_refresh(inputs ∈ S)`. On
/// BigQuery the source tables hold all thirty days from before the first
/// window — the arrival order phase 13 isolated — so a full refresh that does
/// not bound its own scan reads rows the incremental leg had not yet seen at
/// window *k*. Its `inputs` are then *not* "the inputs seen so far", and the
/// invariant's antecedent simply does not hold: comparing against it would
/// measure arrival order, exactly the mistake phase 13 controlled for.
///
/// **At the final window there is nothing to narrow**: the inputs seen so far
/// *are* the whole source, so every relation here is compared normally at
/// window 30 — which is the complete statement of the invariant over the whole
/// thirty-window run sequence.
///
/// The mechanism is **checked, not asserted**, by the entry's own
/// [`ExemptionProof`]. If a proof ever failed, "the oracle read beyond the
/// window" would be false and the exemption would fail the sweep rather than
/// quietly widening into a licence for a real defect. Every exempted row in
/// the committed report carries its proof and its verdict, and the list is
/// checked against the report by
/// [`the_unbounded_refresh_set_is_exactly_what_the_report_shows`], so a
/// relation cannot be exempted without evidence.
struct UnboundedRefresh {
    relation: &'static str,
    reason: &'static str,
    proof: ExemptionProof,
}

/// How an exemption is proved at an intermediate checkpoint.
enum ExemptionProof {
    /// The relation's oracle at checkpoint *k* is **byte-identical to its
    /// oracle at the final window** — direct evidence that the refresh read
    /// the whole source rather than the requested window. Used where the
    /// refresh is unbounded outright.
    OracleIsTheFinalState,
    /// The relation's own refresh *is* window-bounded — its row set is
    /// correct — but it carries columns enriched from an unbounded-refresh
    /// upstream, so the oracle stamps those columns with values from beyond
    /// the window. The proof is correspondingly tight: the two sides must
    /// agree **exactly** once these columns are projected away. A lost row, a
    /// duplicated row, or any other column differing fails it, so this cannot
    /// hide a real defect behind an enrichment column.
    InheritedColumns {
        from: &'static str,
        columns: &'static [&'static str],
    },
}

impl ExemptionProof {
    fn name(&self) -> &'static str {
        match self {
            ExemptionProof::OracleIsTheFinalState => "oracle_is_the_final_state",
            ExemptionProof::InheritedColumns { .. } => "inherited_columns_only",
        }
    }
}

const UNBOUNDED_REFRESH_RELATIONS: &[UnboundedRefresh] = &[
    UnboundedRefresh {
        relation: "silver_repo_naming",
        proof: ExemptionProof::OracleIsTheFinalState,
        reason: "succession cell over `raw.github_events`: a full refresh rebuilds the whole \
                 naming history, because a succession row's `valid_to`/`is_current` is a \
                 function of every later event for the key, not of the requested window",
    },
    UnboundedRefresh {
        relation: "silver_actor_naming",
        proof: ExemptionProof::OracleIsTheFinalState,
        reason: "the same succession shape on `actor_id` over the arrival-partitioned twin \
                 source",
    },
    UnboundedRefresh {
        relation: "gold_repo_dim",
        proof: ExemptionProof::OracleIsTheFinalState,
        reason: "keyed dimension with `maintenance.scan_bounds.per_source.silver.repo_naming.\
                 allow_full_scan: true` — it reads its upstream in full by declaration, so a \
                 full refresh inherits that upstream's whole-history rebuild",
    },
    UnboundedRefresh {
        relation: "marts_naming_history",
        proof: ExemptionProof::OracleIsTheFinalState,
        reason: "derives renames from `silver.repo_naming` pairwise, so it inherits the \
                 succession model's whole-history rebuild",
    },
    UnboundedRefresh {
        relation: "gold_events_enriched",
        proof: ExemptionProof::InheritedColumns {
            from: "gold_repo_dim",
            columns: &["current_repo_name"],
        },
        reason: "its own row set is window-bounded and matches exactly; the enrichment \
                 `LEFT JOIN` against `gold.repo_dim` is what carries beyond the window, so \
                 the oracle stamps each event with the repo's name as of day 30 while the \
                 incremental state carries the name in force when the window ran. Every \
                 other column, and the row set itself, agree exactly",
    },
    UnboundedRefresh {
        relation: "marts_repo_leaderboard",
        proof: ExemptionProof::InheritedColumns {
            from: "gold_repo_dim",
            columns: &["current_repo_name"],
        },
        reason: "same inheritance one level further down: it groups by \
                 `gold.repo_dim.current_repo_name`, so it carries the same beyond-the-window \
                 name the oracle's dimension holds. Row set and every aggregate agree exactly",
    },
];

/// The multiset difference in both directions over every column **except**
/// `ignored` — the check an [`ExemptionProof::InheritedColumns`] entry rests
/// on. Columns are read from the landed left-hand database's own catalogue, so
/// a column disappearing cannot silently shrink what is compared.
fn diff_ignoring_columns(
    left_db: &std::path::Path,
    right_db: &std::path::Path,
    relation: &str,
    ignored: &[&str],
) -> (i64, i64) {
    let conn = bq_parity_support::attached_conn(left_db, right_db);
    let mut stmt = conn
        .prepare(
            "SELECT column_name FROM information_schema.columns \
             WHERE table_catalog = 'left_db' AND table_schema = 'main' AND table_name = ? \
             ORDER BY ordinal_position",
        )
        .expect("prepare column discovery");
    let columns: Vec<String> = stmt
        .query_map([relation], |row| row.get::<_, String>(0))
        .expect("query column discovery")
        .collect::<Result<_, _>>()
        .expect("collect columns");
    for name in ignored {
        assert!(
            columns.iter().any(|c| c == name),
            "`{relation}` has no column `{name}` to ignore — the exemption names a column \
             that no longer exists, which would make the proof vacuous"
        );
    }
    let projection: Vec<&str> = columns
        .iter()
        .map(String::as_str)
        .filter(|c| !ignored.contains(c))
        .collect();
    assert!(
        !projection.is_empty(),
        "`{relation}`: ignoring {ignored:?} leaves nothing to compare"
    );
    let p = projection.join(", ");
    let left_only = format!(
        "SELECT {p} FROM left_db.main.{relation} EXCEPT ALL SELECT {p} FROM right_db.main.{relation}"
    );
    let right_only = format!(
        "SELECT {p} FROM right_db.main.{relation} EXCEPT ALL SELECT {p} FROM left_db.main.{relation}"
    );
    (
        bq_parity_support::scalar_on(&conn, &format!("SELECT count(*) FROM ({left_only})")),
        bq_parity_support::scalar_on(&conn, &format!("SELECT count(*) FROM ({right_only})")),
    )
}

fn unbounded_refresh(relation: &str) -> Option<&'static UnboundedRefresh> {
    UNBOUNDED_REFRESH_RELATIONS
        .iter()
        .find(|u| u.relation == relation)
}

/// The registry-consulting sweep over this suite's own registry and labels.
fn check_equivalence(
    incr_db: &std::path::Path,
    oracle_db: &std::path::Path,
    window_label: &str,
) -> Result<Vec<RelationDiff>, String> {
    check_agreement_against(
        incr_db,
        oracle_db,
        window_label,
        EQUIVALENCE_DIVERGENCE_REGISTRY,
        SIDES,
    )
}

// ---------------------------------------------------------------------------
// Offline negative controls — synthetic pairs, no cloud, no credential
// ---------------------------------------------------------------------------

fn synth_db(path: &std::path::Path, sql: &str) {
    let conn = duckdb::Connection::open(path).unwrap_or_else(|e| panic!("open {path:?}: {e}"));
    conn.execute_batch(sql)
        .unwrap_or_else(|e| panic!("exec failed: {e}\nSQL:\n{sql}"));
}

/// A synthetic pair whose incremental side carries one row the oracle does not,
/// and one differing value — the two shapes an equivalence violation takes.
fn violating_pair(tmp: &std::path::Path) -> (PathBuf, PathBuf) {
    let incr = tmp.join("incr.duckdb");
    let oracle = tmp.join("oracle.duckdb");
    synth_db(
        &incr,
        "CREATE TABLE main.gold_repo_dim AS SELECT * FROM (VALUES \
           (1, 'a/one', 10), (2, 'a/two_stale', 20)) \
           AS t(repo_id, current_repo_name, event_count);",
    );
    synth_db(
        &oracle,
        "CREATE TABLE main.gold_repo_dim AS SELECT * FROM (VALUES \
           (1, 'a/one', 10), (2, 'a/two', 20)) \
           AS t(repo_id, current_repo_name, event_count);",
    );
    (incr, oracle)
}

/// A real difference between incremental state and its oracle fails the sweep,
/// naming the relation and both counts under *this* suite's side labels.
#[test]
fn an_unregistered_equivalence_violation_fails() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (incr, oracle) = violating_pair(tmp.path());

    let err = check_equivalence(&incr, &oracle, "synthetic")
        .expect_err("an unregistered equivalence violation must fail the sweep");
    assert!(
        err.contains("gold_repo_dim"),
        "expected the relation to be named: {err}"
    );
    assert!(
        err.contains("incremental_only=1") && err.contains("oracle_only=1"),
        "expected both counts to be named: {err}"
    );
}

/// The sweep fails closed on an empty registry. The control drives the real
/// registry-consulting path with the registry passed explicitly, so it keeps
/// holding whatever [`EQUIVALENCE_DIVERGENCE_REGISTRY`] comes to contain.
#[test]
fn the_equivalence_sweep_fails_closed_on_an_empty_registry() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (incr, oracle) = violating_pair(tmp.path());

    let err = check_agreement_against(&incr, &oracle, "synthetic", &[], SIDES)
        .expect_err("an empty registry must not make the sweep vacuous");
    assert!(
        err.contains("unregistered divergence"),
        "expected an unregistered-divergence failure: {err}"
    );
}

/// Coverage totality: a relation on only one side is a failure naming the side,
/// never a quietly smaller comparison. On this sweep that would mean a model
/// the incremental leg materialised and the oracle did not — exactly the shape
/// a silently-empty oracle would take.
#[test]
fn a_relation_missing_from_the_oracle_is_a_coverage_failure() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let incr = tmp.path().join("incr.duckdb");
    let oracle = tmp.path().join("oracle.duckdb");
    synth_db(
        &incr,
        "CREATE TABLE main.bronze_events AS SELECT 1 AS id; \
         CREATE TABLE main.marts_star_growth AS SELECT 1 AS id;",
    );
    synth_db(
        &oracle,
        "CREATE TABLE main.bronze_events AS SELECT 1 AS id;",
    );

    let err = compare_databases(&incr, &oracle, SIDES)
        .expect_err("a relation on only one side must fail");
    assert!(
        err.contains("marts_star_growth") && err.contains("missing from oracle"),
        "expected the relation and the missing side to be named: {err}"
    );
}

// ---------------------------------------------------------------------------
// The committed report, and the gates over it
// ---------------------------------------------------------------------------

/// The measured result of the live sweep, committed so the claims above have
/// something to be checked against per-PR. Written by
/// [`bigquery_incremental_matches_its_oracle_at_every_window`]; read here.
const EQUIVALENCE_REPORT_PATH: &str =
    "docs/outcomes/20260906-bigquery-dogfood-spine/phases/14-equivalence.json";

fn equivalence_report() -> serde_json::Value {
    let path = repo_root().join(EQUIVALENCE_REPORT_PATH);
    serde_json::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}")),
    )
    .unwrap_or_else(|e| panic!("parse {path:?}: {e}"))
}

fn report_checkpoints(report: &serde_json::Value) -> Vec<&serde_json::Value> {
    report["checkpoints"]
        .as_array()
        .expect("the report declares checkpoints")
        .iter()
        .collect()
}

/// The report is total: the same relation set at every compared checkpoint, no
/// blank cell, the final window present, and neither excluded model listed. A
/// report that covered twelve relations at one checkpoint and fourteen at
/// another would make every claim below vacuous for the two it dropped.
#[test]
fn the_equivalence_report_covers_every_model_at_every_checkpoint() {
    let report = equivalence_report();
    let checkpoints = report_checkpoints(&report);
    assert!(
        !checkpoints.is_empty(),
        "the committed equivalence report declares no checkpoint"
    );

    let relations_at = |cp: &serde_json::Value| -> BTreeSet<String> {
        cp["relations"]
            .as_array()
            .expect("checkpoint relations")
            .iter()
            .map(|r| r["relation"].as_str().expect("relation name").to_string())
            .collect()
    };

    let first = relations_at(checkpoints[0]);
    assert_eq!(
        first.len(),
        14,
        "the BigQuery half of criterion 7 covers exactly the fourteen models GoogleSQL \
         compiles; got {first:?}"
    );
    for cp in &checkpoints {
        let label = cp["label"].as_str().expect("checkpoint label");
        assert_eq!(
            relations_at(cp),
            first,
            "checkpoint {label} compares a different relation set than the first — a \
             ratchet over a shifting relation set is vacuous for whatever it drops"
        );
        for rel in cp["relations"].as_array().expect("relations") {
            assert!(
                matches!(
                    rel["scope"].as_str(),
                    Some("compared") | Some("oracle_unbounded")
                ),
                "checkpoint {label}, relation {}: no scope",
                rel["relation"]
            );
            for cell in ["incr_rows", "oracle_rows", "incr_only", "oracle_only"] {
                assert!(
                    rel[cell].is_i64(),
                    "checkpoint {label}, relation {}: `{cell}` is missing",
                    rel["relation"]
                );
            }
        }
    }

    for excluded in EXCLUDED_MODELS {
        let table = excluded.replace('.', "_");
        assert!(
            !first.contains(&table),
            "`{excluded}` is compile-refused on BigQuery and must not appear in the report"
        );
    }

    let windows: Vec<i64> = checkpoints
        .iter()
        .map(|c| c["window"].as_i64().expect("window"))
        .collect();
    assert!(
        windows.contains(&30),
        "the final window must be compared; got {windows:?}"
    );
}

/// **Criterion 7's BigQuery half, gated rather than left to prose.** At every
/// compared checkpoint, every compared relation's incrementally-maintained
/// state equals the full refresh over the inputs seen so far — zero rows in
/// both directions of a whole-row multiset difference.
#[test]
fn the_committed_equivalence_report_shows_no_violation() {
    let report = equivalence_report();
    let mut offenders = Vec::new();
    for cp in report_checkpoints(&report) {
        let label = cp["label"].as_str().expect("label");
        for rel in cp["relations"].as_array().expect("relations") {
            if rel["scope"].as_str() != Some("compared") {
                continue;
            }
            let incr_only = rel["incr_only"].as_i64().expect("incr_only");
            let oracle_only = rel["oracle_only"].as_i64().expect("oracle_only");
            if incr_only != 0 || oracle_only != 0 {
                offenders.push(format!(
                    "{label}/{}: incremental_only={incr_only}, oracle_only={oracle_only}",
                    rel["relation"]
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the committed report shows the equivalence invariant violated on BigQuery — each \
         of these is a finding for `20260906-bigquery-correctness`, not something to \
         register away: {offenders:?}"
    );
}

/// The anti-vacuity gate, restated over the committed evidence: the oracle
/// actually materialised rows. An oracle that silently refreshed over an empty
/// source would agree with nothing and disagree with nothing at zero rows, so
/// every compared relation must be non-empty on the oracle side at the final
/// window, and `bronze_events` — the whole-source passthrough — must carry the
/// full source population.
#[test]
fn the_committed_report_proves_the_oracle_read_the_shared_source() {
    let report = equivalence_report();
    let checkpoints = report_checkpoints(&report);
    let final_cp = checkpoints
        .iter()
        .find(|c| c["window"].as_i64() == Some(30))
        .expect("the final window is in the report");

    for rel in final_cp["relations"].as_array().expect("relations") {
        let rows = rel["oracle_rows"].as_i64().expect("oracle_rows");
        assert!(
            rows > 0,
            "relation {} has {rows} oracle rows at the final window — an oracle over an \
             empty source compares vacuously",
            rel["relation"]
        );
    }

    let bronze = final_cp["relations"]
        .as_array()
        .expect("relations")
        .iter()
        .find(|r| r["relation"].as_str() == Some("bronze_events"))
        .expect("`bronze_events` is compared");
    assert_eq!(
        bronze["oracle_rows"].as_i64(),
        Some(65583),
        "`bronze.events` is a whole-source passthrough, so the oracle's copy must carry \
         the shared source's full 65,583 rows — a smaller number means the oracle read \
         something other than `smelt_dogfood.github_events`"
    );
}

/// The two-sided liveness ratchet over the committed report: an entry naming a
/// relation the report shows agreeing is stale and must be deleted; a relation
/// the report shows diverging with no entry is an unregistered violation.
#[test]
fn equivalence_registry_entries_are_all_live() {
    let report = equivalence_report();
    let mut measured: BTreeSet<String> = BTreeSet::new();
    for cp in report_checkpoints(&report) {
        for rel in cp["relations"].as_array().expect("relations") {
            if rel["scope"].as_str() != Some("compared") {
                continue;
            }
            if rel["incr_only"].as_i64() != Some(0) || rel["oracle_only"].as_i64() != Some(0) {
                measured.insert(rel["relation"].as_str().expect("relation name").to_string());
            }
        }
    }
    let registered: BTreeSet<String> = EQUIVALENCE_DIVERGENCE_REGISTRY
        .iter()
        .map(|e| e.relation.to_string())
        .collect();

    let stale: Vec<&String> = registered.difference(&measured).collect();
    assert!(
        stale.is_empty(),
        "these registry entries name relations the committed report shows agreeing — \
         delete them: {stale:?}"
    );
    let unregistered: Vec<&String> = measured.difference(&registered).collect();
    assert!(
        unregistered.is_empty(),
        "the committed report shows these relations violating the equivalence invariant \
         with no registry entry: {unregistered:?}"
    );
}

/// **The final window is compared in full.** Nothing is exempt there: the
/// inputs seen so far are the whole source, so the oracle is exactly the
/// invariant's full refresh over them, for all fourteen relations. This is the
/// complete statement of `incremental_state(S) == full_refresh(inputs ∈ S)`
/// over the fixture's whole thirty-window run sequence, and it is the claim
/// criterion 7's BigQuery half actually rests on.
#[test]
fn the_final_window_compares_every_relation_with_nothing_exempt() {
    let report = equivalence_report();
    let checkpoints = report_checkpoints(&report);
    let final_cp = checkpoints
        .iter()
        .find(|c| c["window"].as_i64() == Some(30))
        .expect("the final window is in the report");
    let relations = final_cp["relations"].as_array().expect("relations");
    assert_eq!(
        relations.len(),
        14,
        "the final window must compare all fourteen"
    );
    for rel in relations {
        assert_eq!(
            rel["scope"].as_str(),
            Some("compared"),
            "`{}` is exempt at the final window — nothing may be, since the inputs seen so \
             far are the whole source there",
            rel["relation"]
        );
    }
}

/// The two-sided ratchet over [`UNBOUNDED_REFRESH_RELATIONS`]. A relation
/// listed there that the report does not show behaving that way is an
/// unjustified exemption and must be deleted; a relation the report exempts
/// with no entry cannot happen (the sweep derives the scope from the list), so
/// this checks the direction that can: every exempted row carries the proof
/// that its oracle equals the final window's.
#[test]
fn the_unbounded_refresh_set_is_exactly_what_the_report_shows() {
    let report = equivalence_report();
    let listed: BTreeSet<String> = UNBOUNDED_REFRESH_RELATIONS
        .iter()
        .map(|u| u.relation.to_string())
        .collect();
    assert!(
        !listed.is_empty(),
        "an empty exemption list with a scope-aware gate would be a trap for whoever adds \
         the first entry — if the list is genuinely empty, delete the mechanism instead"
    );

    let mut exempted: BTreeSet<String> = BTreeSet::new();
    for cp in report_checkpoints(&report) {
        let label = cp["label"].as_str().expect("label");
        for rel in cp["relations"].as_array().expect("relations") {
            if rel["scope"].as_str() != Some("oracle_unbounded") {
                continue;
            }
            let name = rel["relation"].as_str().expect("relation").to_string();
            assert_eq!(
                rel["proof_holds"].as_bool(),
                Some(true),
                "checkpoint {label}: `{name}` is exempted, but the report records its \
                 exemption proof as not holding — an exemption whose mechanism fails is a \
                 divergence"
            );
            assert!(
                rel["exemption_proof"].is_string(),
                "checkpoint {label}: `{name}` is exempted with no named proof"
            );
            exempted.insert(name);
        }
    }
    assert_eq!(
        exempted, listed,
        "the exemption list and what the committed report actually exempts have drifted \
         apart — an entry with no evidence is an unjustified exemption"
    );

    for u in UNBOUNDED_REFRESH_RELATIONS {
        assert!(
            !u.reason.trim().is_empty(),
            "`{}` is exempted with no stated mechanism",
            u.relation
        );
    }
}

// ---------------------------------------------------------------------------
// The live sweep
// ---------------------------------------------------------------------------

/// One compared checkpoint. `types_db_path` is the DuckDB leg's database at
/// that window — the **type reference** for landing both BigQuery sides, not a
/// comparison side: typing both sides identically is what makes the two landed
/// snapshots byte-comparable. Produced by
/// `scripts/bq-dogfood-parity.sh oracle-manifest`.
#[derive(serde::Deserialize)]
struct Checkpoint {
    label: String,
    window: i64,
    day: String,
    types_db_path: PathBuf,
    incr_ndjson_dir: PathBuf,
    oracle_ndjson_dir: PathBuf,
}

#[derive(serde::Deserialize)]
struct EquivalenceManifest {
    checkpoints: Vec<Checkpoint>,
}

/// The whole sweep over the live snapshots, and the writer of the committed
/// equivalence report.
///
/// Gating: with `SMELT_BQ_DOGFOOD_LIVE=1` set this **fails** rather than
/// skipping when the snapshots are absent — a live gate that goes green because
/// nothing was there is worse than no gate. With it unset the test skips,
/// because the snapshots are hundreds of megabytes of exported rows that no
/// per-PR run produces.
#[test]
fn bigquery_incremental_matches_its_oracle_at_every_window() {
    if std::env::var("SMELT_BQ_DOGFOOD_LIVE").as_deref() != Ok("1") {
        eprintln!(
            "SMELT_BQ_DOGFOOD_LIVE is not 1 — skipping the live equivalence sweep. \
             Produce the snapshots with scripts/bq-dogfood-parity.sh."
        );
        return;
    }
    let manifest_path = std::env::var("EQUIVALENCE_MANIFEST")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_root().join("target/phase14/equivalence-manifest.json"));
    let manifest: EquivalenceManifest = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path).unwrap_or_else(|e| {
            panic!("SMELT_BQ_DOGFOOD_LIVE=1 but no manifest at {manifest_path:?}: {e}")
        }),
    )
    .unwrap_or_else(|e| panic!("parse {manifest_path:?}: {e}"));
    assert!(
        !manifest.checkpoints.is_empty(),
        "{manifest_path:?} declares no checkpoint — the sweep would pass vacuously"
    );

    let scratch = tempfile::TempDir::new().expect("tempdir");
    let mut checkpoints_json = Vec::new();
    let mut failures = Vec::new();

    // The final window's oracle is landed first and kept: at an intermediate
    // checkpoint the unbounded-refresh relations are proved against it (see
    // [`UnboundedRefresh`]), which is what makes their exemption a measured
    // claim rather than an assumption.
    let final_window = manifest
        .checkpoints
        .iter()
        .map(|c| c.window)
        .max()
        .expect("at least one checkpoint");
    let final_cp = manifest
        .checkpoints
        .iter()
        .find(|c| c.window == final_window)
        .expect("the final checkpoint");
    let final_oracle = scratch.path().join("final-oracle.duckdb");
    load_bigquery_snapshot(
        &final_cp.types_db_path,
        &final_cp.oracle_ndjson_dir,
        &final_oracle,
    );

    for cp in &manifest.checkpoints {
        for (what, path) in [
            ("type reference", &cp.types_db_path),
            ("incremental snapshot", &cp.incr_ndjson_dir),
            ("oracle snapshot", &cp.oracle_ndjson_dir),
        ] {
            assert!(
                path.exists(),
                "checkpoint {}: no {what} at {path:?}",
                cp.label
            );
        }

        let incr = scratch.path().join(format!("{}-incr.duckdb", cp.label));
        let oracle = scratch.path().join(format!("{}-oracle.duckdb", cp.label));
        load_bigquery_snapshot(&cp.types_db_path, &cp.incr_ndjson_dir, &incr);
        load_bigquery_snapshot(&cp.types_db_path, &cp.oracle_ndjson_dir, &oracle);

        // Every relation's raw numbers, recorded whatever the verdict, so the
        // report names what differed rather than merely that something did.
        let diffs = compare_databases(&incr, &oracle, SIDES)
            .unwrap_or_else(|e| panic!("checkpoint {}: {e}", cp.label));

        // Anti-vacuity, checked at the moment of measurement rather than only
        // over the written report: an oracle that read an empty source would
        // agree with nothing at zero rows.
        for d in &diffs {
            assert!(
                d.right_rows > 0,
                "checkpoint {}: relation `{}` has zero oracle rows — the full refresh \
                 read an empty source, which would make this comparison vacuous",
                cp.label,
                d.relation
            );
        }

        let is_final = cp.window == final_window;

        // The unbounded-refresh relations, at an intermediate checkpoint only:
        // prove the mechanism — this checkpoint's oracle is byte-identical to
        // the final window's, i.e. the refresh read the whole source rather
        // than the requested window. A relation that failed this would not be
        // exempt; it would be a divergence.
        let mut proof_holds: std::collections::BTreeMap<String, bool> = Default::default();
        if !is_final {
            for u in UNBOUNDED_REFRESH_RELATIONS {
                let (held, detail) = match &u.proof {
                    ExemptionProof::OracleIsTheFinalState => {
                        let conn = bq_parity_support::attached_conn(&oracle, &final_oracle);
                        let d = bq_parity_support::relation_diff(&conn, u.relation);
                        (
                            d.left_only == 0 && d.right_only == 0,
                            format!(
                                "this checkpoint's oracle is not the final window's                                  ({} rows only here, {} only there)",
                                d.left_only, d.right_only
                            ),
                        )
                    }
                    ExemptionProof::InheritedColumns { from, columns } => {
                        let (a, b) = diff_ignoring_columns(&incr, &oracle, u.relation, columns);
                        (
                            a == 0 && b == 0,
                            format!(
                                "with {columns:?} (inherited from `{from}`) projected away the                                  two sides still differ: {a} rows only on the incremental                                  side, {b} only on the oracle's"
                            ),
                        )
                    }
                };
                proof_holds.insert(u.relation.to_string(), held);
                if !held {
                    failures.push(format!(
                        "checkpoint {}: `{}` is exempted from comparison ({}), but its \
                         exemption proof `{}` does not hold — {detail}. An exemption whose \
                         mechanism fails is a divergence, not a scope note",
                        cp.label,
                        u.relation,
                        u.reason,
                        u.proof.name()
                    ));
                }
            }
            // Compare only where the oracle is a valid oracle. The exempt
            // relations are dropped from BOTH landed sides, so relation-set
            // totality still holds and the registry-consulting path below runs
            // over a set it can actually judge.
            for db in [&incr, &oracle] {
                let conn =
                    duckdb::Connection::open(db).unwrap_or_else(|e| panic!("open {db:?}: {e}"));
                for u in UNBOUNDED_REFRESH_RELATIONS {
                    conn.execute_batch(&format!("DROP TABLE IF EXISTS main.{};", u.relation))
                        .unwrap_or_else(|e| panic!("drop {}: {e}", u.relation));
                }
            }
        }

        // The gated verdict, through the same registry-consulting path the
        // offline negative controls drive.
        if let Err(msg) = check_equivalence(&incr, &oracle, &cp.label) {
            failures.push(msg);
        }

        checkpoints_json.push(serde_json::json!({
            "label": cp.label,
            "window": cp.window,
            "day": cp.day,
            "relations": diffs.iter().map(|d| {
                let exempt = !is_final && unbounded_refresh(&d.relation).is_some();
                serde_json::json!({
                    "relation": d.relation,
                    "scope": if exempt { "oracle_unbounded" } else { "compared" },
                    "exemption_proof": unbounded_refresh(&d.relation)
                        .filter(|_| !is_final)
                        .map(|u| u.proof.name()),
                    "proof_holds": proof_holds.get(&d.relation),
                    "incr_rows": d.left_rows,
                    "oracle_rows": d.right_rows,
                    "incr_only": d.left_only,
                    "oracle_only": d.right_only,
                })
            }).collect::<Vec<_>>(),
        }));
    }

    let report = serde_json::json!({
        "claim": "on BigQuery, each model's incrementally-maintained state equals a full \
                  refresh over the inputs seen so far",
        "target": ORACLE_TARGET,
        "oracle_dataset": ORACLE_SCHEMA,
        "shared_source_dataset": SHARED_DATASET,
        "schedule": {
            "start_date": "2026-08-05",
            "days": 30,
            "checkpoints": manifest.checkpoints.iter().map(|c| c.window).collect::<Vec<_>>(),
        },
        "excluded_models": EXCLUDED_MODELS,
        "checkpoints": checkpoints_json,
    });
    let out = std::env::var("EQUIVALENCE_REPORT_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_root().join(EQUIVALENCE_REPORT_PATH));
    std::fs::write(
        &out,
        serde_json::to_string_pretty(&report).expect("serialise the equivalence report") + "\n",
    )
    .unwrap_or_else(|e| panic!("write {out:?}: {e}"));
    eprintln!("wrote {out:?}");

    assert!(
        failures.is_empty(),
        "BigQuery's incremental state does not equal its own full refresh:\n{}",
        failures.join("\n\n")
    );
}

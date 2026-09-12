#![cfg(feature = "duckdb")]
//! The equivalence invariant, checked on Databricks: after each incremental
//! window, does each model's incrementally-maintained state equal a **full
//! refresh over the inputs seen so far**?
//! (`docs/specs/incremental_models.md` §"The equivalence invariant";
//! `docs/outcomes/20260912-databricks-dogfood-spine/outcome.md` criterion 8.)
//!
//! # What this suite is and is not
//!
//! It is **not** a second dual-target parity sweep. `github_activity_dual_target.rs`
//! asks "do two engines agree with each other"; this asks "does one engine's
//! incremental state equal its own full refresh". Both sides here run on
//! Databricks. A non-zero diff in a compared cell is a **violation of the
//! equivalence invariant** on a real pipeline, and the bar for registering
//! rather than fixing it is correspondingly higher than the dual-target
//! sweep's own registry (see [`EQUIVALENCE_DIVERGENCE_REGISTRY`]).
//!
//! # Where the oracle is a valid oracle — measured, not exempted
//!
//! The invariant is `incremental_state(S) == full_refresh(inputs ∈ S)`, so the
//! oracle must refresh over **the inputs seen so far**. On BigQuery the source
//! tables held all thirty fixture days before the first window, so
//! `github_activity_bq_oracle.rs` has to *exempt* a handful of whole-source-
//! rebuild relations at intermediate checkpoints. On Databricks the loader
//! lands one fixture day at a time (`docs/outcomes/
//! 20260912-databricks-dogfood-spine/phases/03-plan.md`), so the source holds
//! **only** the inputs seen so far at every checkpoint by construction — a
//! full refresh over the source *is* a full refresh over "the inputs seen so
//! far". That premise is not assumed; it is measured per checkpoint by
//! [`parity_support::assert_source_covers_window`], which fails the sweep the
//! moment a checkpoint's source has landed more (or fewer) days than its
//! window number. No relation is exempted from comparison — see
//! [`the_databricks_oracle_exempts_no_relation`].
//!
//! # How a comparison is made
//!
//! Both sides are Databricks relations, exported to typed NDJSON by
//! `scripts/dbx_dogfood_export.py` and landed into DuckDB by the **same**
//! primitive the dual-target sweep and the BigQuery oracle sweep use
//! (`parity_support`), typed from the same DuckDB reference database, then
//! differenced whole-row with `EXCEPT ALL` in both directions. One
//! comparator, three claims.
//!
//! # Coverage, stated rather than implied
//!
//! - **All sixteen models.** Phases 6b-6f closed every construct
//!   Spark/Databricks refused (`parity_support::DATABRICKS_EXCLUDED_MODELS`
//!   is empty), so nothing is excluded from this sweep the way
//!   `silver.actor_sessions` and `marts.daily_active_contributors` are
//!   excluded from the BigQuery leg.
//! - **Three windows.** `2026-08-13`, `2026-08-14`, `2026-08-15` — windows 9,
//!   10 and 11, landing three more fixture days on top of the eight phases
//!   5-7b already loaded (`scripts/dbx-dogfood-oracle.sh`'s default
//!   checkpoints, asserted against this file by
//!   [`the_oracle_driver_declares_the_checkpoint_schedule_the_sweep_expects`]).
//! - **Bookkeeping is excluded by decision, not by absence.**
//!   `_smelt_ledger`, `_smelt_observed_delta` and the two `__tombstones`
//!   tables record *how* a run happened, and an incremental run's bookkeeping
//!   legitimately differs from a `--full-refresh` run's, so they are excluded
//!   exactly as the BigQuery and DuckDB oracles already exclude them.

use std::collections::BTreeSet;
use std::path::PathBuf;

#[path = "parity_support/mod.rs"]
mod parity_support;
use parity_support::{
    check_agreement_against, compare_databases, load_exported_snapshot, repo_root, synth_db,
    violating_pair, DivergenceBound, EquivalenceManifest, RegisteredDivergence, RelationDiff,
    SideLabels, DATABRICKS_EXCLUDED_MODELS,
};

/// The two sides of *this* sweep: Databricks' incrementally-maintained state,
/// and Databricks' own full refresh over the inputs seen so far.
const SIDES: SideLabels = SideLabels {
    left: "incremental",
    right: "oracle",
};

/// The committed project's fourth target, which the oracle leg runs on.
const ORACLE_TARGET: &str = "databricks_oracle";
/// The scratch schema that target writes into.
const ORACLE_SCHEMA: &str = "smelt_dogfood_oracle";
/// The schema holding the pipeline's incrementally-maintained state and, in
/// `github_events` / `github_events_arrival`, the **shared** source tables
/// both legs read.
const SHARED_SCHEMA: &str = "smelt_dogfood";

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
/// Without a `databricks_oracle:` entry in each source's target-aware `name:`
/// map, `SourceInfo::db_name_for_target` falls back to the default mapping
/// `<target_schema>.<address_segments.join("_")>` — which on the oracle
/// target is `smelt_dogfood_oracle.sources_raw_github_events`, a table
/// nothing ever creates. The oracle would then refresh over zero rows and the
/// whole sweep would pass vacuously: incremental state compared against an
/// oracle built from nothing. That is the failure mode this phase is here to
/// rule out, so it is asserted through the real resolver rather than by
/// eyeballing the YAML.
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
            resolved.starts_with(&format!("{SHARED_SCHEMA}.")),
            "the oracle leg must read from `{SHARED_SCHEMA}`, not from its own scratch \
             schema: got `{resolved}`"
        );

        let default_mapping = format!("{ORACLE_SCHEMA}.{}", segments.join("_"));
        assert_ne!(
            resolved, default_mapping,
            "the oracle target fell back to the default source mapping — this is exactly \
             the vacuous pass the `databricks_oracle:` entry exists to prevent"
        );
    }
}

/// The oracle target must write to its own scratch schema, never the state it
/// judges.
#[test]
fn the_oracle_target_writes_only_to_the_oracle_schema() {
    let config = example_config();
    let target = config
        .targets
        .get(ORACLE_TARGET)
        .unwrap_or_else(|| panic!("the committed project must declare `{ORACLE_TARGET}`"));

    assert_eq!(
        target.schema, ORACLE_SCHEMA,
        "`{ORACLE_TARGET}` must write to its own scratch schema, not `{SHARED_SCHEMA}` — \
         otherwise the oracle could overwrite the state it is judging"
    );
    assert_eq!(
        target.catalog.as_deref(),
        Some("workspace"),
        "`{ORACLE_TARGET}` must share the same Unity Catalog catalog as `databricks`"
    );
}

/// Adding a fourth target must not move the no-`--target` default. `target:
/// dev` is pinned in `smelt.yml` precisely because the fallback when it is
/// unset is the alphabetically-first target name
/// (`smelt-runtime/src/profile.rs`), and `bigquery`, `bigquery_oracle`,
/// `databricks` and now `databricks_oracle` all sort ahead of `dev`.
#[test]
fn adding_the_oracle_target_does_not_move_the_default() {
    let config = example_config();

    assert!(
        config.targets.contains_key(ORACLE_TARGET),
        "the committed project must declare the `{ORACLE_TARGET}` target"
    );

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
/// [`parity_support::DivergenceBound::MonotoneDivergence`] encodes. Anything
/// that cannot be stated in those terms is a defect for a follow-on
/// `databricks-correctness` outcome, recorded as a finding, never registered
/// away.
///
/// An empty registry plus a vacuous sweep would be indistinguishable from
/// success, so the registry-consulting path is driven over a real mismatch by
/// [`an_unregistered_equivalence_violation_fails`] and
/// [`the_equivalence_sweep_fails_closed_on_an_empty_registry`], and the
/// committed report is checked for unregistered divergence once phase 9b
/// commits one.
const EQUIVALENCE_DIVERGENCE_REGISTRY: &[RegisteredDivergence] = &[RegisteredDivergence {
    relation: "gold_events_enriched",
    // Root cause, not a shrug — the identical bound and root cause
    // `github_activity_dual_target.rs::DBX_DIVERGENCE_REGISTRY` already
    // registers for the same relation. `gold.events_enriched`'s
    // `current_repo_name` is a value-enrichment join against
    // `gold.repo_dim` whose designed healing semantics
    // (`crates/smelt-runtime/src/execute/enrichment_heal.rs`) run a
    // `ColumnScopedMerge` cell once per run over the model's UNWINDOWED
    // output. Phase 7b downgraded this cell's Databricks route from
    // `PerGroupRecompute` (which the key-addressed driver can never
    // dispatch for an `EnrichmentKeyed` cell) straight to `DeleteInsert`,
    // because Spark/Delta has no `MergeLedger`
    // (`docs/outcomes/20260912-databricks-dogfood-spine/phases/07b-plan.md`).
    // `DeleteInsert` is window-scoped, so on Databricks `current_repo_name`
    // freezes at whatever `gold.repo_dim` held on the day a row was FIRST
    // written and is never retroactively healed by a later rename — this
    // affects the **full-refresh oracle exactly as much as the incremental
    // leg**, because the oracle target runs the same plan, so the committed
    // report shows the identical incr_only == oracle_only count at every
    // checkpoint (9/10/16 at w09/w10/w11) rather than a one-sided
    // divergence. No column other than `current_repo_name` diverges and no
    // row is missing or extra, so the bound is unordered rather than
    // monotone.
    reason: "gold.repo_dim enrichment freezes at write time on Databricks: phase 7b downgraded \
             the EnrichmentKeyed cell to window-scoped DeleteInsert (Spark/Delta has no \
             MergeLedger), which sacrifices the unwindowed run-level heal DuckDB's \
             ColumnScopedMerge cell performs. The same downgrade applies to both the \
             incremental leg and this suite's own full-refresh oracle, so a pre-rename \
             current_repo_name persists identically on both sides until databricks-correctness \
             realises the fingerprint sidecar on Delta.",
    bound: DivergenceBound::UnorderedColumnDivergence {
        key_col: "id",
        exact_columns: &[
            "type",
            "actor_id",
            "actor_login",
            "repo_id",
            "repo_name",
            "org_id",
            "public",
            "created_at",
            "event_date",
        ],
        tolerant_columns: &["current_repo_name"],
    },
}];

/// The relations whose full refresh reads beyond the requested window on this
/// target. Empty — see the module doc's "Where the oracle is a valid oracle"
/// section: the source lands one day at a time, so a full refresh over it
/// never reads beyond the inputs seen so far, and no relation needs an
/// exemption from comparison the way BigQuery's whole-source-rebuild
/// relations do.
const DBX_UNBOUNDED_REFRESH_RELATIONS: &[&str] = &[];

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

/// The per-checkpoint source-coverage assertion over a whole manifest — the
/// measurement that stands in for BigQuery's unbounded-refresh exemption
/// list on this target (see the module doc comment).
fn check_source_coverage(manifest: &EquivalenceManifest) -> Result<(), String> {
    for cp in &manifest.checkpoints {
        parity_support::assert_source_covers_window(&cp.label, cp.source_days_loaded, cp.window)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Offline negative controls — synthetic pairs, no cloud, no credential
// ---------------------------------------------------------------------------

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

/// Coverage totality: a relation on only one side is a failure naming the
/// side, never a quietly smaller comparison. On this sweep that would mean a
/// model the incremental leg materialised and the oracle did not — exactly
/// the shape a silently-empty oracle would take.
#[test]
fn a_relation_missing_from_the_oracle_side_is_a_coverage_failure() {
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

/// The exemption list is empty and the per-checkpoint source-coverage
/// assertion this target uses in its place exists and passes when the source
/// has landed exactly the inputs the window claims.
#[test]
fn the_databricks_oracle_exempts_no_relation() {
    assert!(
        DBX_UNBOUNDED_REFRESH_RELATIONS.is_empty(),
        "the Databricks leg exempts no relation from comparison — see the module doc's \
         \"Where the oracle is a valid oracle\" section"
    );
    assert!(
        DATABRICKS_EXCLUDED_MODELS.is_empty(),
        "phases 6b-6f closed every construct Spark/Databricks refused; this sweep covers \
         the whole sixteen-model set"
    );
    parity_support::assert_source_covers_window("w09", 9, 9)
        .expect("a checkpoint whose source has landed exactly its window number must pass");
}

/// Over a synthetic manifest, a checkpoint recording more source days than
/// its window number fails — the assertion that makes "the source holds only
/// the inputs seen so far" measured rather than assumed.
#[test]
fn a_checkpoint_whose_source_ran_ahead_of_its_window_fails() {
    let manifest: EquivalenceManifest = serde_json::from_str(
        r#"{"checkpoints": [
            {"label": "w09", "window": 9, "day": "2026-08-13",
             "types_db_path": "/tmp/w09.duckdb", "incr_ndjson_dir": "/tmp/incr",
             "oracle_ndjson_dir": "/tmp/oracle", "source_days_loaded": 11}
        ]}"#,
    )
    .expect("parse a synthetic manifest");

    let err = check_source_coverage(&manifest)
        .expect_err("a source that ran ahead of its window must fail the sweep");
    assert!(
        err.contains("w09"),
        "expected the checkpoint to be named: {err}"
    );
    assert!(
        err.contains("11") && err.contains('9'),
        "expected both the source day count and the window number to be named: {err}"
    );
}

/// The default `START_DATE`/checkpoint list parsed out of
/// `scripts/dbx-dogfood-oracle.sh` matches the constants this file's own live
/// sweep compares against, so script and suite cannot drift.
#[test]
fn the_oracle_driver_declares_the_checkpoint_schedule_the_sweep_expects() {
    let script = std::fs::read_to_string(repo_root().join("scripts/dbx-dogfood-oracle.sh"))
        .expect("read scripts/dbx-dogfood-oracle.sh");

    assert!(
        script.contains(r#"START_DATE="${ORACLE_START_DATE:-2026-08-05}""#),
        "the script's default START_DATE has drifted from what this suite expects: {script}"
    );
    assert!(
        script.contains(r#"CHECKPOINTS="${ORACLE_CHECKPOINTS:-9,10,11}""#),
        "the script's default checkpoint schedule has drifted from windows 9, 10, 11"
    );
}

// ---------------------------------------------------------------------------
// The committed report, and the gates over it
// ---------------------------------------------------------------------------

/// The measured result of the live sweep, committed so the claims above have
/// something to be checked against per-PR. Written by
/// [`databricks_incremental_matches_its_oracle_at_every_window`]; read here.
const EQUIVALENCE_REPORT_PATH: &str =
    "docs/outcomes/20260912-databricks-dogfood-spine/phases/09b-equivalence.json";

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

/// The report is total: the same sixteen-relation set at every compared
/// checkpoint, no blank cell.
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
        16,
        "the Databricks half of criterion 8 covers all sixteen models — DATABRICKS_EXCLUDED_MODELS \
         is empty; got {first:?}"
    );
    for cp in &checkpoints {
        let label = cp["label"].as_str().expect("checkpoint label");
        assert_eq!(
            relations_at(cp),
            first,
            "checkpoint {label} compares a different relation set than the first — a ratchet \
             over a shifting relation set is vacuous for whatever it drops"
        );
        for rel in cp["relations"].as_array().expect("relations") {
            for cell in ["incr_rows", "oracle_rows", "incr_only", "oracle_only"] {
                assert!(
                    rel[cell].is_i64(),
                    "checkpoint {label}, relation {}: `{cell}` is missing",
                    rel["relation"]
                );
            }
        }
    }
}

/// Relations the committed report shows diverging at *any* compared
/// checkpoint. Mirrors `github_activity_dual_target.rs::divergent_relations`
/// under this suite's own `incr_only`/`oracle_only` field names.
fn divergent_relations(report: &serde_json::Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for cp in report_checkpoints(report) {
        for rel in cp["relations"].as_array().expect("relations array") {
            let incr_only = rel["incr_only"].as_i64().expect("incr_only");
            let oracle_only = rel["oracle_only"].as_i64().expect("oracle_only");
            if incr_only != 0 || oracle_only != 0 {
                out.insert(rel["relation"].as_str().expect("relation name").to_string());
            }
        }
    }
    out
}

/// **Criterion 8, closed.** At every compared checkpoint, every compared
/// relation's incrementally-maintained state equals the full refresh over the
/// inputs seen so far — zero rows in both directions of a whole-row multiset
/// difference — unless the relation carries a registered
/// [`EQUIVALENCE_DIVERGENCE_REGISTRY`] entry, in which case the divergence is
/// licensed rather than a violation.
#[test]
fn the_committed_equivalence_report_shows_no_violation() {
    let report = equivalence_report();
    let registered: BTreeSet<String> = EQUIVALENCE_DIVERGENCE_REGISTRY
        .iter()
        .map(|e| e.relation.to_string())
        .collect();
    let mut offenders = Vec::new();
    for cp in report_checkpoints(&report) {
        let label = cp["label"].as_str().expect("label");
        for rel in cp["relations"].as_array().expect("relations") {
            let relation = rel["relation"].as_str().expect("relation name");
            if registered.contains(relation) {
                continue;
            }
            let incr_only = rel["incr_only"].as_i64().expect("incr_only");
            let oracle_only = rel["oracle_only"].as_i64().expect("oracle_only");
            if incr_only != 0 || oracle_only != 0 {
                offenders.push(format!(
                    "{label}/{relation}: incremental_only={incr_only}, oracle_only={oracle_only}"
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the committed report shows the equivalence invariant violated on Databricks with no \
         registered licence — each of these is a finding for a follow-on \
         databricks-correctness outcome, not something to register away silently: {offenders:?}"
    );
}

/// The two-sided liveness ratchet. An entry naming a relation the committed
/// report shows agreeing at every checkpoint is stale and must be deleted; a
/// relation the report shows diverging with no registry entry is an
/// unregistered divergence — caught here rather than only by
/// [`the_committed_equivalence_report_shows_no_violation`] so a healed
/// divergence forces the entry out instead of just going quiet. Mirrors
/// `github_activity_dual_target.rs::registry_entries_are_all_live`.
#[test]
fn equivalence_registry_entries_are_all_live() {
    let report = equivalence_report();
    let measured = divergent_relations(&report);
    let registered: BTreeSet<String> = EQUIVALENCE_DIVERGENCE_REGISTRY
        .iter()
        .map(|e| e.relation.to_string())
        .collect();

    let stale: Vec<&String> = registered.difference(&measured).collect();
    assert!(
        stale.is_empty(),
        "EQUIVALENCE_DIVERGENCE_REGISTRY entries name relations the committed equivalence \
         report shows agreeing at every checkpoint — delete them: {stale:?}"
    );
    let unregistered: Vec<&String> = measured.difference(&registered).collect();
    assert!(
        unregistered.is_empty(),
        "the committed equivalence report shows these relations diverging with no registry \
         entry — register each with a root-caused reason and a checkable bound: \
         {unregistered:?}"
    );
}

// ---------------------------------------------------------------------------
// The live sweep
// ---------------------------------------------------------------------------

/// The whole sweep over the live snapshots, and the writer of the committed
/// equivalence report.
///
/// Gating: with `SMELT_DBX_DOGFOOD_LIVE=1` set this **fails** rather than
/// skipping when the manifest is absent — a live gate that goes green because
/// nothing was there is worse than no gate. With it unset the test skips,
/// because the snapshots are exported rows that no per-PR run produces.
#[test]
fn databricks_incremental_matches_its_oracle_at_every_window() {
    if std::env::var("SMELT_DBX_DOGFOOD_LIVE").as_deref() != Ok("1") {
        eprintln!(
            "SMELT_DBX_DOGFOOD_LIVE is not 1 — skipping the live equivalence sweep. \
             Produce the snapshots with scripts/dbx-dogfood-oracle.sh."
        );
        return;
    }
    let manifest_path = std::env::var("EQUIVALENCE_MANIFEST")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_root().join("target/phase9/equivalence-manifest.json"));
    let manifest: EquivalenceManifest = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path).unwrap_or_else(|e| {
            panic!("SMELT_DBX_DOGFOOD_LIVE=1 but no manifest at {manifest_path:?}: {e}")
        }),
    )
    .unwrap_or_else(|e| panic!("parse {manifest_path:?}: {e}"));
    assert!(
        !manifest.checkpoints.is_empty(),
        "{manifest_path:?} declares no checkpoint — the sweep would pass vacuously"
    );

    // The measurement that makes every checkpoint's oracle a valid oracle for
    // "the inputs seen so far": fail loudly and immediately if the source
    // ever ran ahead of (or behind) the window it is supposed to bound.
    check_source_coverage(&manifest).unwrap_or_else(|e| panic!("{e}"));

    let scratch = tempfile::TempDir::new().expect("tempdir");
    let mut checkpoints_json = Vec::new();
    let mut failures = Vec::new();

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
        load_exported_snapshot(&cp.types_db_path, &cp.incr_ndjson_dir, &incr);
        load_exported_snapshot(&cp.types_db_path, &cp.oracle_ndjson_dir, &oracle);

        // Every relation's raw numbers, recorded whatever the verdict, so the
        // report names what differed rather than merely that something did.
        let diffs = compare_databases(&incr, &oracle, SIDES)
            .unwrap_or_else(|e| panic!("checkpoint {}: {e}", cp.label));

        // Anti-vacuity, checked at the moment of measurement rather than
        // only over the written report: an oracle that read an empty source
        // would agree with nothing at zero rows.
        for d in &diffs {
            assert!(
                d.right_rows > 0,
                "checkpoint {}: relation `{}` has zero oracle rows — the full refresh read \
                 an empty source, which would make this comparison vacuous",
                cp.label,
                d.relation
            );
        }

        if let Err(msg) = check_equivalence(&incr, &oracle, &cp.label) {
            failures.push(msg);
        }

        checkpoints_json.push(serde_json::json!({
            "label": cp.label,
            "window": cp.window,
            "day": cp.day,
            "source_days_loaded": cp.source_days_loaded,
            "relations": diffs.iter().map(|d| {
                serde_json::json!({
                    "relation": d.relation,
                    "scope": "compared",
                    "incr_rows": d.left_rows,
                    "oracle_rows": d.right_rows,
                    "incr_only": d.left_only,
                    "oracle_only": d.right_only,
                })
            }).collect::<Vec<_>>(),
        }));
    }

    let report = serde_json::json!({
        "claim": "on Databricks, each model's incrementally-maintained state equals a full \
                  refresh over the inputs seen so far",
        "target": ORACLE_TARGET,
        "oracle_schema": ORACLE_SCHEMA,
        "shared_source_schema": SHARED_SCHEMA,
        "schedule": {
            "start_date": "2026-08-05",
            "checkpoints": manifest.checkpoints.iter().map(|c| c.window).collect::<Vec<_>>(),
        },
        "excluded_models": DATABRICKS_EXCLUDED_MODELS,
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
        "Databricks' incremental state does not equal its own full refresh:\n{}",
        failures.join("\n\n")
    );
}

// ---------------------------------------------------------------------------
// Anti-vacuity and coverage gates over the committed report
// ---------------------------------------------------------------------------

/// Anti-vacuity over the committed evidence: every checkpoint records
/// `source_days_loaded == window` (the premise that makes this target's oracle
/// valid — see the module doc's "Where the oracle is a valid oracle" section)
/// and a non-zero row count on both sides for every relation, so an
/// empty-vs-empty comparison cannot read as success. Mirrors
/// `github_activity_bq_oracle.rs`'s gate of the same name.
#[test]
fn the_committed_report_proves_the_oracle_read_the_shared_source() {
    let report = equivalence_report();
    for cp in report_checkpoints(&report) {
        let label = cp["label"].as_str().expect("label");
        let window = cp["window"].as_i64().expect("window");
        let source_days_loaded = cp["source_days_loaded"]
            .as_i64()
            .expect("source_days_loaded");
        assert_eq!(
            source_days_loaded, window,
            "checkpoint {label}: source_days_loaded ({source_days_loaded}) must equal the \
             window number ({window}) — otherwise the oracle is not a valid full refresh over \
             exactly the inputs seen so far"
        );
        for rel in cp["relations"].as_array().expect("relations") {
            let incr_rows = rel["incr_rows"].as_i64().expect("incr_rows");
            let oracle_rows = rel["oracle_rows"].as_i64().expect("oracle_rows");
            assert!(
                incr_rows > 0 && oracle_rows > 0,
                "checkpoint {label}, relation {}: incr_rows={incr_rows}, oracle_rows={oracle_rows} \
                 — a zero on either side would make this comparison vacuous",
                rel["relation"]
            );
        }
    }
}

/// **The final window is compared in full.** Nothing is exempt there: all
/// sixteen models are checked with `DBX_UNBOUNDED_REFRESH_RELATIONS` empty —
/// the complete statement of `incremental_state(S) == full_refresh(inputs ∈
/// S)` over this target's three-window live run.
#[test]
fn the_final_window_compares_every_relation_with_nothing_exempt() {
    assert!(
        DBX_UNBOUNDED_REFRESH_RELATIONS.is_empty(),
        "this target exempts no relation from comparison at any window"
    );
    let report = equivalence_report();
    let checkpoints = report_checkpoints(&report);
    let final_cp = checkpoints
        .iter()
        .find(|c| c["window"].as_i64() == Some(11))
        .expect("the final window (11) is in the report");
    let relations = final_cp["relations"].as_array().expect("relations");
    assert_eq!(
        relations.len(),
        16,
        "the final window must compare all sixteen models"
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

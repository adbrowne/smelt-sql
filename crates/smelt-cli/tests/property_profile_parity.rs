//! Standing gate: `smelt explain <model> --json`'s maintenance-plan report
//! must render exactly the values the shared property profile
//! (`smelt_logical::analysis::profile::PropertyProfile`,
//! `docs/specs/property_diff.md` §"The property profile") derives, for
//! `properties`, `cell_verdicts`-vs-`cells`, `refusals`, and `probes`
//! (`docs/specs/property_diff.md` §Constraints item 4, "Report/profile
//! parity"). This is the phase's own standing gate:
//! `cargo test -p smelt-cli --test property_profile_parity`.
//!
//! **Coverage note on `examples/retail_analytics`.** That workspace
//! declares no `grain:` and no incremental models — every one of its models
//! hits `smelt explain <model>`'s "no maintenance plan" early return, so
//! there is no per-model `--json` maintenance report to compare against for
//! *any* model there (`crates/smelt-cli/src/commands/explain.rs`'s
//! `explain_maintenance_plan` returns before the `--json` branch when
//! `smelt_db::maintenance_plan_report` is `None`). Its report-vs-profile
//! comparison loop is therefore correctly empty for that workspace — the
//! gate's non-vacuity assertions below are all scoped to
//! `examples/timeseries`, which does have maintained models. What
//! `examples/retail_analytics` DOES exercise is the `PropertySet` half of
//! the profile in isolation: this test also asserts `PropertySet::derive`
//! succeeds for every one of its discovered models (the half of the profile
//! that exists independent of a maintenance plan), so the workspace still
//! earns its place in this gate rather than being dead weight.
//!
//! **What this gate is, and is not, an oracle for.** `smelt-cli`'s own
//! `build_maintenance_plan_json` (`commands/explain.rs`) now *reads*
//! `CellVerdict`/`ProfileRefusal`/`ProfileProbe` off the same
//! `PropertyProfile` this test compares against, rather than recomputing
//! its own encodings — the correct consequence of single ownership
//! (`CLAUDE.md` §"Maintenance-plan purity"). That means this gate compares
//! JSON derived from the profile against the profile itself for those
//! fields, and is therefore a real tripwire against a parallel, drifting
//! derivation reappearing in the report path — but it is NOT an
//! independent oracle for whether the profile's own values are *correct*.
//! Value-level correctness for the underlying maintenance plan is the
//! standing-gate suite's job (`maintenance_conformance`,
//! `maintenance_diagnostics`, and friends), not this file's.

use std::path::Path;
use std::process::Command;

use smelt_cli::{discover_python_models, init_db, Config, ModelDiscovery};
use smelt_core::ModelFile;
use smelt_logical::analysis::profile::{PropertyProfile, PropertySet};

/// Discover every genuine **model** in `project_dir`, mirroring
/// `explain_maintenance.rs::build_report_for`'s own discovery prefix.
/// `ModelDiscovery::discover_models` bundles bare-SELECT models together
/// with `smelt.define` functions, `smelt.test` tests, and `smelt.check`
/// checks (`smelt_core::discovery::ModelDiscovery::discover_models`'s own
/// doc comment) — filtered here to `EntityKind::Model` only via the same
/// `smelt_core::resolver::classify` the discovery walk itself uses, since
/// this gate's model-diagnostics/property-set derivation is only meaningful
/// for an actual model (a `smelt.test` file's SQL is not a model's own
/// property vector).
fn discover_all(project_dir: &Path) -> (Config, Vec<ModelFile>) {
    let config = Config::load(project_dir).expect("load smelt.yml");
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let mut models = discovery.discover_models().expect("discover models");
    let python_files = discovery
        .discover_python_files()
        .expect("scan python files");
    if !python_files.is_empty() {
        let python_models = discover_python_models(
            &python_files,
            &models,
            &config,
            project_dir,
            config.python.as_deref(),
        )
        .expect("discover python models");
        models.extend(python_models);
    }
    models.retain(|m| {
        matches!(m.kind, smelt_core::ModelKind::Python { .. })
            || matches!(
                smelt_core::resolver::classify(&m.path, Some(&m.content), &[]),
                Some(smelt_core::resolver::EntityKind::Model)
            )
    });
    (config, models)
}

/// Build every model's [`smelt_logical::analysis::profile::PropertyProfile`]
/// in `project_dir` via the shared whole-workspace builder
/// (`smelt_runtime::profile::profiles_for_workspace`,
/// `docs/outcomes/20260905-property-diff/phases/04-plan.md` D9) — the
/// profile side of the comparison. This is the regression proof that lifting
/// the per-model pipeline (dependency graph, compiler registry, ephemeral
/// resolver, probe plan, `build_model_diagnostics`) out of this test file
/// and into `smelt-runtime` preserved behaviour: this gate is byte-exact
/// against the real `smelt explain --json` binary, unchanged from before the
/// lift.
fn profiles_for(project_dir: &Path) -> std::collections::BTreeMap<String, PropertyProfile> {
    let loaded = smelt_core::workspace::load_workspace(project_dir);
    smelt_runtime::profile::profiles_for_workspace(&loaded)
        .unwrap_or_else(|e| panic!("profiles_for_workspace({}): {e}", project_dir.display()))
        .profiles
}

/// Independent ground truth for a model's refusal count, computed directly
/// via the raw Salsa query — deliberately NOT going through
/// `profiles_for`/`profiles_for_workspace`'s own pipeline, the same
/// independence `count_models_with_maintenance_plan` below already
/// exercises for plan presence. Without this, comparing a profile's own
/// `refusals.len()` against itself is a tautology (fix round 1, P4).
fn refusal_counts_by_model(project_dir: &Path) -> std::collections::BTreeMap<String, usize> {
    let (_, models) = discover_all(project_dir);
    let db = init_db(project_dir, &models);
    let ws = smelt_db::Workspace::try_get(&db).expect("workspace not initialized");
    models
        .iter()
        .filter_map(|m| {
            let file = db.source_file(&m.path).expect("model file registered");
            let result = smelt_db::maintenance_plan_report(&db, ws, file)?;
            Some((m.canonical_path(), result.plan.refusals.len()))
        })
        .collect()
}

/// Spawn the real `smelt` binary's `explain <model> --json` and parse its
/// stdout — the report side of the comparison, exercising the actual
/// wiring rather than calling `build_maintenance_plan_json` directly.
fn spawn_explain_json(project_dir: &Path, model_name: &str) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg(model_name)
        .arg("--json")
        .arg("--project-dir")
        .arg(project_dir)
        .output()
        .expect("spawn smelt explain --json");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "smelt explain {model_name} --json failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!("smelt explain {model_name} --json did not print JSON: {e}\nstdout={stdout}")
    })
}

struct WorkspaceCheck {
    /// Models with a maintenance plan whose report/profile were compared.
    checked: usize,
    /// Models discovered with a maintenance plan at all (the denominator
    /// `checked` must equal — `covers_every_example_model`).
    discovered_with_plan: usize,
    total_cell_verdicts: usize,
    total_refusals_underlying: usize,
    total_refusals_profile: usize,
    total_probes: usize,
}

/// Run the report/profile comparison over every discovered model in
/// `project_dir`, asserting byte-identical JSON encodings for `properties`,
/// each cell's scalar fields + `contract_point`, `refusals`, and `probes`.
fn compare_workspace(project_dir: &Path) -> WorkspaceCheck {
    let (_, models) = discover_all(project_dir);
    let profiles = profiles_for(project_dir);
    let refusal_counts = refusal_counts_by_model(project_dir);
    let mut checked = 0usize;
    let mut total_cell_verdicts = 0usize;
    let mut total_refusals_underlying = 0usize;
    let mut total_refusals_profile = 0usize;
    let mut total_probes = 0usize;

    for model in &models {
        let canonical = model.canonical_path();
        let Some(profile) = profiles.get(&canonical) else {
            continue;
        };
        checked += 1;
        // Independent ground truth (P4): read from a raw
        // `maintenance_plan_report` call, never from `profile.refusals`
        // itself — the latter would make the non-vacuity assertion below a
        // tautology.
        let refusals_ground_truth = *refusal_counts.get(&canonical).unwrap_or(&0);
        total_refusals_underlying += refusals_ground_truth;

        let report = spawn_explain_json(project_dir, &canonical);

        // `properties` — byte-identical string encodings.
        let profile_properties =
            serde_json::to_value(&profile.properties).expect("serialize PropertySet");
        assert_eq!(
            profile_properties, report["properties"],
            "model '{canonical}': report `properties` diverges from the profile's own encoding"
        );

        // `cell_verdicts` vs the report's `cells` scalar fields +
        // `contract_point` (report cells carry extra fields —
        // `statements`/`technique_previews`/`no_statements_reason` — that
        // are renderings, not profile data, so only the shared scalar
        // fields are compared).
        let cells = report["cells"].as_array().cloned().unwrap_or_default();
        assert_eq!(
            profile.cell_verdicts.len(),
            cells.len(),
            "model '{canonical}': cell_verdicts count diverges from report cells count"
        );
        total_cell_verdicts += profile.cell_verdicts.len();
        for (verdict, cell_json) in profile.cell_verdicts.iter().zip(cells.iter()) {
            assert_eq!(
                cell_json["group"],
                serde_json::json!(verdict.group),
                "{canonical}"
            );
            assert_eq!(
                cell_json["trigger"],
                serde_json::json!(verdict.trigger),
                "{canonical}"
            );
            assert_eq!(
                cell_json["corner"],
                serde_json::json!(verdict.corner),
                "{canonical}"
            );
            assert_eq!(
                cell_json["technique"],
                serde_json::json!(verdict.technique),
                "{canonical}"
            );
            assert_eq!(
                cell_json["row_identity"],
                serde_json::json!(format!("{:?}", verdict.row_identity.identity)),
                "{canonical}"
            );
            assert_eq!(
                cell_json
                    .get("row_identity_proven_mismatch")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
                serde_json::to_value(&verdict.row_identity.proven_mismatch).unwrap(),
                "{canonical}"
            );
            assert_eq!(
                cell_json["contract_point"],
                serde_json::to_value(&verdict.contract_point).unwrap(),
                "model '{canonical}': cell '{}' contract_point diverges",
                verdict.group
            );
        }

        // `refusals` — byte-identical.
        let profile_refusals = serde_json::to_value(&profile.refusals).expect("serialize refusals");
        assert_eq!(
            profile_refusals, report["refusals"],
            "model '{canonical}': report `refusals` diverges from the profile's own encoding"
        );
        total_refusals_profile += profile.refusals.len();

        // `probes` — compare the shared fact/probe/cell fields (the report
        // additionally carries `cadence`/`cost`, presentation-only extras
        // the profile's `ProfileProbe` deliberately omits).
        let report_probes = report["probes"].as_array().cloned().unwrap_or_default();
        assert_eq!(
            profile.probes.len(),
            report_probes.len(),
            "model '{canonical}': probes count diverges from the report's own probes array"
        );
        total_probes += profile.probes.len();
        for (probe, probe_json) in profile.probes.iter().zip(report_probes.iter()) {
            assert_eq!(
                probe_json["fact"],
                serde_json::json!(probe.fact),
                "{canonical}"
            );
            assert_eq!(
                probe_json["probe"],
                serde_json::json!(probe.probe),
                "{canonical}"
            );
            assert_eq!(
                probe_json["cell"],
                serde_json::json!(probe.cell),
                "{canonical}"
            );
        }
    }

    let discovered_with_plan = count_models_with_maintenance_plan(project_dir);

    WorkspaceCheck {
        checked,
        discovered_with_plan,
        total_cell_verdicts,
        total_refusals_underlying,
        total_refusals_profile,
        total_probes,
    }
}

/// Count discovered models whose `smelt_db::maintenance_plan_report` is
/// `Some` — computed independently of `compare_workspace`'s own loop above
/// (fix round 1, F3). The loop's `checked` counter increments exactly when
/// `build_diagnostics_for` returns `Some`; comparing it against a count
/// derived from that very same conditional was a tautology, not a coverage
/// check. This asks `maintenance_plan_report` directly for every discovered
/// model, without going through `build_diagnostics_for`'s much larger
/// pipeline (dependency graph, compiler registry, ephemeral resolver,
/// probe plan, `build_model_diagnostics`), so a bug that silently drops a
/// model somewhere later in that pipeline (a swallowed `Err`, a stray
/// filter) still shows up as `checked != discovered_with_plan`.
fn count_models_with_maintenance_plan(project_dir: &Path) -> usize {
    let (_, models) = discover_all(project_dir);
    let db = init_db(project_dir, &models);
    let ws = smelt_db::Workspace::try_get(&db).expect("workspace not initialized");
    models
        .iter()
        .filter(|m| {
            let file = db.source_file(&m.path).expect("model file registered");
            smelt_db::maintenance_plan_report(&db, ws, file).is_some()
        })
        .count()
}

fn timeseries_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists")
}

fn retail_analytics_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/retail_analytics")
        .canonicalize()
        .expect("examples/retail_analytics exists")
}

/// The gate: `smelt explain <model> --json`'s maintenance report and the
/// shared `PropertyProfile` agree, field by field, for every maintained
/// model in `examples/timeseries`. Also asserts NON-VACUITY (ruling R3):
/// at least one model yields a non-empty `cell_verdicts` set, at least one
/// cell (hence at least one `contract_point`), and — only if the
/// underlying plan actually refused something — a non-empty `refusals`
/// set (never asserted unconditionally, since a clean example project may
/// legitimately refuse nothing).
#[test]
fn report_json_matches_profile_encoding() {
    let ts = compare_workspace(&timeseries_dir());
    assert!(
        ts.checked > 0,
        "examples/timeseries must have at least one maintained model to compare"
    );
    assert!(
        ts.total_cell_verdicts > 0,
        "non-vacuity: examples/timeseries must yield at least one non-empty cell_verdicts set \
         (hence at least one contract_point)"
    );
    // Refusals: only assert non-empty when the underlying data actually
    // refused something (computed from the profile's own refusals, since
    // that count is exactly what a real refusal would populate).
    if ts.total_refusals_underlying > 0 {
        assert!(
            ts.total_refusals_profile > 0,
            "a model refused admission but the profile carried no refusals for it"
        );
    }

    // examples/retail_analytics has no maintenance plans at all — its
    // report-comparison loop is correctly empty (see module doc comment).
    let retail = compare_workspace(&retail_analytics_dir());
    assert_eq!(
        retail.checked, 0,
        "examples/retail_analytics declares no incremental models; a maintained model here \
         would mean the fixture assumption behind this gate's coverage note is stale"
    );

    // The `PropertySet` half of the profile, exercised in isolation for
    // `examples/retail_analytics` since no report exists to compare against
    // (see module doc comment).
    let (_, retail_models) = discover_all(&retail_analytics_dir());
    assert!(
        !retail_models.is_empty(),
        "fixture sanity: retail_analytics has models"
    );
    for model in &retail_models {
        let sql = smelt_parser::strip_frontmatter(&model.content).to_string();
        let declared_unique_key: Vec<String> = model
            .metadata
            .as_deref()
            .and_then(|m| m.unique_key.clone())
            .unwrap_or_default();
        let bound_ctx = smelt_logical::analysis::source_bounds::BoundContext::new();
        PropertySet::derive(
            &model.canonical_path(),
            &sql,
            &declared_unique_key,
            &bound_ctx,
        )
        .unwrap_or_else(|e| {
            panic!(
                "PropertySet::derive must succeed for retail_analytics model '{}': {e}",
                model.canonical_path()
            )
        });
    }
}

/// The set of models this gate actually compared equals the set of
/// discovered models that have a maintenance plan at all — a silent
/// skip (an `Err` swallowed by `.ok()`, a filter bug) would otherwise pass
/// unnoticed.
#[test]
fn covers_every_example_model() {
    let ts = compare_workspace(&timeseries_dir());
    assert_eq!(
        ts.checked, ts.discovered_with_plan,
        "examples/timeseries: the gate skipped a maintained model"
    );
    let retail = compare_workspace(&retail_analytics_dir());
    assert_eq!(
        retail.checked, retail.discovered_with_plan,
        "examples/retail_analytics: the gate skipped a maintained model"
    );
}

/// At least one model in `examples/timeseries` is actually maintained
/// (non-empty `cell_verdicts`) — otherwise the whole gate would pass
/// vacuously over an all-`full`-refresh project.
#[test]
fn covers_at_least_one_maintained_model() {
    let ts = compare_workspace(&timeseries_dir());
    assert!(
        ts.total_cell_verdicts > 0,
        "examples/timeseries must contain at least one maintained model with ≥1 cell"
    );
    // `examples/timeseries` does declare probe-backed facts (ruling R3
    // asked this to be checked, not assumed) — this is real coverage, not
    // a vacuous pass.
    assert!(
        ts.total_probes > 0,
        "examples/timeseries declared no probe-backed facts — this gate's probe-parity \
         assertions would be vacuous; add one or note the coverage gap explicitly"
    );
}

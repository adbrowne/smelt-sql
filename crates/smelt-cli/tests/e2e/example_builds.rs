#![cfg(feature = "duckdb")]
//! Build-and-execute gate for every example workspace.
//!
//! Where `example_diagnostics.rs` checks the *analysis* layer (the LSP /
//! `file_diagnostics`), this gate checks the *build* layer: it runs
//! `smelt build` — which compiles **and** executes the models on DuckDB — for
//! every workspace under `examples/` and asserts the outcome the workspace's
//! category demands.
//!
//! This is the structural harness for the diagnostic-parity effort
//! (`docs/plans/20260531-diagnostic-parity.md`). The two layers should agree:
//! a workspace the analyzer accepts must build, and a workspace the analyzer
//! rejects must not. Later phases of that plan close the remaining gaps; the
//! `KNOWN_UNBUILDABLE` allow-list below records the gaps that exist today, each
//! with the exact observed reason, and shrinks to empty as the phases land.
//!
//! Categories (decided per workspace at runtime):
//!   1. **Allow-listed unbuildable** (`KNOWN_UNBUILDABLE`) — `smelt build` is
//!      expected to FAIL. The failure reason is logged (no silent skip) and the
//!      test asserts the build does indeed fail, so a future fix that makes it
//!      build forces the entry's removal from the allow-list.
//!   2. **Broken fixture** (name contains `broken`) — an intentionally invalid
//!      workspace whose defect must be caught *somewhere*: either the analyzer
//!      rejects it with an `Error`-severity diagnostic (via the same
//!      diagnostics API `example_diagnostics.rs` uses), or `smelt build` itself
//!      fails (some defects — the incremental safety classifier, generator-body
//!      validators — surface at build time, not through `file_diagnostics`). A
//!      small named set (`BROKEN_BUILDS_CLEAN`) intentionally builds clean
//!      because the "broken" scenario is one the framework is expected to
//!      suppress; those assert `smelt build` succeeds instead.
//!   3. **Clean** (everything else) — `smelt build` must compile and execute
//!      every model successfully (exit 0).

use smelt_cli::{init_db, Config, ModelDiscovery};
use smelt_db::{DiagnosticAcc, DiagnosticSeverity, Workspace};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Workspaces that do not build today, each with the exact observed reason.
///
/// These are SKIPPED from the clean-build assertion but their failure is LOGGED
/// (run with `--nocapture` to see it) and the test still asserts the build
/// fails, so the entry cannot silently become stale: when a later phase makes
/// the workspace build, this test turns red and the entry must be removed.
///
/// Later phases of `docs/plans/20260531-diagnostic-parity.md` remove entries
/// here; the close-out phase empties the list (only intentional `*broken*`
/// markers remain, and those are handled by the `broken` category, not here).
const KNOWN_UNBUILDABLE: &[(&str, &str)] = &[
    // ── Meta-language codegen gaps (closed by P5–P7) ──────────────────────
    // These workspaces are analysis-clean but the in-model meta-language
    // constructs are not yet evaluated at build, so the analyzer-accepted ref
    // is emitted verbatim and either fails dependency resolution or compiles to
    // SQL referencing a relation that was never produced.
    (
        // In-model list spread now expands at build (P5/BUG-006a, verified by
        // `crates/smelt-cli/tests/meta_lists_e2e.rs`): the emitted SQL is
        // `SELECT id, 1, 2, 3 FROM raw.users`. The remaining blocker is no
        // longer a codegen gap — `raw.users` is an unseeded source in the
        // standalone build env (same category as timeseries/web_analytics).
        // (Latent, separate from spread: the `list_literal` model's bare
        // integer-literal columns would also hit the `apply_type_casts`
        // unaliased-literal defect were the source seeded; reproducible with no
        // spread at all, so tracked apart from diagnostic parity.)
        "meta_lists",
        "Error: Failed to execute model: list_literal \
         (Catalog Error: Table \"raw.users\" does not exist — unseeded source in \
         the standalone build env; in-model list spread itself now expands at \
         build, see meta_lists_e2e.rs)",
    ),
    // `meta_hofs` and `meta_polish` were here (HOFs / pipe / config.var /
    // ternary / reducers not evaluated at build). Closed by P6: the build-path
    // meta evaluator (`smelt-runtime::meta_eval`) lowers them to plain SQL, and
    // their models reference no sources, so they build + execute clean.
    (
        // `smelt.columns_of` reflection now lowers at build (P7a/BUG-006
        // columns_of, verified by `crates/smelt-cli/tests/meta_columns_e2e.rs`):
        // `orders_safe` materialises its numeric columns into plain select items.
        // The remaining blocker is no longer a codegen gap — the `orders` model
        // reads the unseeded `raw.orders` source in the standalone build env
        // (same category as timeseries/web_analytics), and fails before
        // `orders_safe` is reached.
        "meta_columns",
        "Error: Failed to execute model: orders \
         (Catalog Error: Table \"raw.orders\" does not exist — unseeded source in \
         the standalone build env; smelt.columns_of reflection itself now lowers \
         at build, see meta_columns_e2e.rs)",
    ),
    (
        // Wide reflection (`smelt.models.with_tag` / `smelt.sources.with_tag`)
        // now lowers and executes at build (P7b). The remaining blocker is
        // unrelated to diagnostic parity: the cohort models read the unseeded
        // `raw.events` source (the standalone build env has no datagen), so the
        // build fails on the first cohort model. De-listing is a seeding
        // decision for P8, not a codegen gap. The BUG-006 wide-reflection
        // regression target is the hermetic `meta_workspace_e2e.rs`.
        "meta_workspace",
        "Error: Execution failed for 'main.cohort_c': Catalog Error: Table with \
         name \"raw.events\" does not exist because schema \"raw\" does not exist \
         (unseeded source in standalone build env; wide reflection itself lowers \
         correctly — covered by meta_workspace_e2e.rs)",
    ),
    // ── Function codegen gaps ──────────────────────────────────────────────
    (
        // Nested `smelt.define` (P3/BUG-013) and block PASSING fragments
        // (P4/BUG-018) now expand and execute correctly. The remaining blocker
        // is unrelated to this plan: `uses_generics` lowers to SQL DuckDB
        // rejects with `Parser Error: NameListToString NOT IMPLEMENTED` (a
        // generics/registry codegen gap, not a diagnostic-parity defect), plus
        // unknown `provenance`/`deterministic` frontmatter fields surface as
        // warnings. De-listing waits on the separate generics-codegen fix.
        "functions_demo",
        "Error: Failed to execute model: uses_generics \
         (Parser Error: NameListToString NOT IMPLEMENTED — generics codegen gap, \
         separate from diagnostic-parity; PASSING/nested-define already fixed)",
    ),
    // ── Workspaces whose execution needs external source data not present in
    //    the test environment (a `raw.*` schema / generated tables). These are
    //    not codegen gaps; they cannot execute standalone. They remain on the
    //    allow-list because the harness builds from a clean copy with no
    //    pre-populated source tables. (Not owned by a specific phase; the
    //    close-out phase decides whether to seed them or keep them listed.) ──
    (
        "ecommerce",
        "Error: Catalog Error: Table with name \"raw.events\" does not exist \
         (source schema `raw` is not seeded in the standalone build env)",
    ),
    (
        "retail_analytics",
        "Error: Catalog Error: Table with name \"raw.customers\" does not exist \
         (source schema `raw` requires setup_sources.sql / datagen; not seeded)",
    ),
    (
        "timeseries",
        "Error: Failed to execute model: daily_events \
         (upstream source tables not seeded in the standalone build env)",
    ),
    (
        "web_analytics",
        "Error: Failed to execute model: bronze.raw_events \
         (upstream source tables not seeded in the standalone build env)",
    ),
    (
        "test_workspace",
        "Error: Failed to execute model: raw_events \
         (upstream source tables not seeded in the standalone build env)",
    ),
    (
        "incremental_idempotency",
        "Error: Failed to execute model: daily_events \
         (upstream source tables not seeded in the standalone build env)",
    ),
    (
        "scoping_probe",
        "Error: Failed to execute model: uses_shadow \
         (upstream source tables not seeded in the standalone build env)",
    ),
    (
        "smelt_shop_min",
        "Error: Failed to execute model: stg_orders \
         (upstream source tables not seeded in the standalone build env)",
    ),
    (
        "staging_from_sources",
        "Error: Failed to execute model: staging.staging.products \
         (upstream source tables not seeded in the standalone build env)",
    ),
    (
        "architecture_domain_layout",
        "Catalog Error: Table with name sources_raw_invoices does not exist \
         (W1 domain-layout fixture: demonstrates billing/finance path prefixes; \
         stg_invoices references smelt.sources.raw_invoices which is not seeded \
         in the standalone build env)",
    ),
    (
        "seed_source_type_join",
        "Catalog Error: Table with name sources_raw_orders does not exist \
         (D5 probe fixture: seeds+sources type-alias coverage; the seed loads \
         but the model joins an external source that is not seeded in the \
         standalone build env; LSP + source-diagnostics coverage verified)",
    ),
    (
        "cumulative_classifier_gate",
        "Error: Failed to execute cumulative model: edges_bad_aggregator \
         (probe fixture: intentionally exercises a cumulative aggregator gate; \
         not seeded / not a clean build target)",
    ),
    (
        "fn_incremental_ts",
        "Catalog Error: Table with name sources_events does not exist \
         (D1 probe fixture: function expansion inside incremental model works correctly; \
         source table `main.sources_events` not seeded in standalone build env; \
         verified e2e via fn_incremental_ts_e2e.rs)",
    ),
    (
        "incremental_nondeterministic_columns",
        "Catalog Error: Table with name sources_events does not exist \
         (batched.nondeterministic_columns fixture: the non-determinism flow/taint check \
         admits the model correctly; source table `main.sources_events` not seeded in \
         standalone build env; verified e2e via incremental_nondeterministic_columns_e2e.rs)",
    ),
    (
        "incremental_declared_monotonic",
        "Catalog Error: Table with name sources_fact does not exist \
         (DC1 fixture: `timeseries.assert_monotonic` correctly widens the opaque-function \
         join driving-fact trace and admits the model; source table `main.sources_fact` \
         not seeded in the standalone build env — covered by the unit tests in \
         crates/smelt-logical/src/rules/incremental.rs)",
    ),
    (
        "horizon_ceiling_comfortable",
        "Catalog Error: Table with name sources_raw_events does not exist \
         (DC4 fixture: `horizon_ceiling` warning-only declaration — the derived 2-hour \
         reach is comfortably inside the declared 30-day ceiling, so no warning fires; \
         source table `main.sources_raw_events` not seeded in the standalone build env — \
         covered by crates/smelt-runtime/tests/horizon_ceiling_warning.rs)",
    ),
    (
        "horizon_ceiling_tight",
        "Catalog Error: Table with name sources_raw_events does not exist \
         (DC4 fixture: `horizon_ceiling` warning-only declaration — the derived 2-hour \
         reach exceeds the declared 1-hour ceiling, licensing a compile-time warning that \
         never narrows the clamp; source table `main.sources_raw_events` not seeded in the \
         standalone build env — covered by crates/smelt-runtime/tests/horizon_ceiling_warning.rs)",
    ),
    (
        "source_mutation_profile_declared",
        "Catalog Error: Table with name sources_raw_events does not exist \
         (DC5 fixture: source-side `mutation_profile` + `source_lateness` declaration; \
         source table `main.sources_raw_events` not seeded in the standalone build env — \
         covered by crates/smelt-core/tests/source_yaml.rs and \
         crates/smelt-logical/src/analysis/input_delta.rs unit tests)",
    ),
    (
        "fn_struct_spread_cte",
        "Catalog Error: Table with name sources_gps_readings does not exist \
         (D2 probe fixture: struct spread inside CTE body schema propagation; \
         source table not seeded in standalone build env; \
         verified via type_command_function_returns::struct_spread_inside_cte_body_propagates_field_types)",
    ),
    (
        "demo_workspace",
        "Error: Dependency validation failed \
         (probe/demo fixture; not a clean standalone build target)",
    ),
    (
        "combined_generators",
        "Error: Failed to run combined discovery loop: smelt Python SDK not found \
         (D-24 bidirectional fixture; contains a Python @model that requires the \
         smelt Python SDK adjacent to the project root; verified e2e via \
         crates/smelt-cli/tests/combined_generators_e2e.rs)",
    ),
    // NOTE: `huge` and `multi_engine` are NOT here — they are in `NEVER_BUILD`
    // below (the gate does not even attempt to build them; see that doc).
];

/// Workspaces the gate does NOT attempt to build at all — running `smelt build`
/// on them is pure cost with no signal. Unlike `KNOWN_UNBUILDABLE` (which runs
/// the build and asserts it still fails, so a future fix forces de-listing),
/// these are logged and skipped *without spawning a build*.
///
/// - `huge` is an auto-generated 2000-model stress workspace; compiling and
///   executing it — even just to a known failure — dominates this gate's
///   wall-clock and (in the autonomy loop) token cost.
/// - `multi_engine` has no standalone target; its Spark target needs a Docker
///   engine the CI/test environment does not provide.
///
/// We never intend to make either build, so the "forces de-listing when it
/// starts building" guarantee that justifies running the build for
/// `KNOWN_UNBUILDABLE` does not apply to them.
const NEVER_BUILD: &[(&str, &str)] = &[
    (
        "huge",
        "auto-generated 2000-model stress workspace; building it is pure cost",
    ),
    (
        "multi_engine",
        "no standalone target (Spark target needs a Docker engine)",
    ),
];

/// Broken fixtures whose intentional scenario nonetheless builds clean — the
/// "broken" in the name refers to a condition the framework is expected to
/// *handle*, not reject. For these the broken-category check asserts `smelt
/// build` SUCCEEDS rather than asserting a rejection.
///
/// Empty today: with the diagnostic-parity gate in place, any fixture whose
/// analyzer reports an `Error` is refused at build time, so a "broken" fixture
/// cannot both flag red in the editor and build clean.
/// `per_cohort_union_broken_emission_body_collision_suppression` used to live
/// here — its emission-body name collision *suppresses* the discarded body's
/// analysis (no spurious `UndeclaredColumn`), but the surviving
/// `ModelDefDuplicateName` is an `Error` (expansion.md structural file check),
/// so the parity gate now correctly rejects the build. It is handled by the
/// `broken` category (analyzer `Error`) instead.
const BROKEN_BUILDS_CLEAN: &[&str] = &[];

/// Max workspaces whose full failure detail is dumped in the assert message; the
/// rest are summarized by name + count. Bounds the assert size when a regression
/// breaks several workspaces at once.
const MAX_DETAILED_FAILURES: usize = 8;

/// Keep only the last `n` lines of build output. A failing `smelt build` can emit
/// a large stdout+stderr; the tail carries the actual error without flooding the
/// assert message (and, in the autonomy loop, the model's context window).
fn tail_output(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    if lines.len() <= n {
        return s.trim_end().to_string();
    }
    let omitted = lines.len() - n;
    format!(
        "… ({omitted} earlier line(s) omitted; showing last {n}) …\n{}",
        lines[lines.len() - n..].join("\n")
    )
}

/// Optional per-phase scoping. When `SMELT_EXAMPLE_BUILDS_ONLY` is set to a
/// comma-separated list of workspace names, the gate runs only those workspaces
/// — so a single plan phase can build just the workspace(s) it touches instead
/// of the whole example set (clean copies + DuckDB execution per workspace is
/// the gate's dominant cost). With the var unset, every workspace runs: that is
/// the close-out / CI configuration that proves the whole set still builds.
fn only_filter() -> Option<std::collections::HashSet<String>> {
    match std::env::var("SMELT_EXAMPLE_BUILDS_ONLY") {
        Ok(v) if !v.trim().is_empty() => Some(
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        ),
        _ => None,
    }
}

fn examples_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
}

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

/// Every directory under `examples/` that contains a `smelt.yml`, sorted.
fn example_workspaces() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(examples_root())
        .expect("examples/ directory is readable")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("smelt.yml").is_file())
        .collect();
    dirs.sort();
    dirs
}

/// Run `smelt build` against a *copy* of the workspace (so the build's
/// `target/*.duckdb` writes never touch the checked-in tree) and return the
/// process output.
fn run_build(workspace: &Path) -> std::process::Output {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    let dest = tmp.path().join(workspace.file_name().unwrap());
    copy_dir(workspace, &dest);

    let out = Command::new(smelt_bin())
        .arg("build")
        .args(["--project-dir", dest.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));
    // Keep `tmp` alive until after the build completes.
    drop(tmp);
    out
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let path = entry.path();
        let name = entry.file_name();
        // Skip the build-artifact tree; the copy starts from a clean slate.
        if name == "target" {
            continue;
        }
        let target = dst.join(&name);
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

/// Collect (severity-counted) diagnostics for a workspace via the same
/// Salsa-direct path `example_diagnostics.rs` uses. Returns
/// `(error_messages, warning_messages)`.
fn collect_diagnostics(workspace: &Path) -> (Vec<String>, Vec<String>) {
    let config: Config = serde_yaml::from_str(
        &std::fs::read_to_string(workspace.join("smelt.yml")).expect("read smelt.yml"),
    )
    .expect("parse smelt.yml");

    let discovery = ModelDiscovery::new(workspace.to_path_buf(), config.paths.clone());
    let mut models = discovery.discover_models().expect("discover models");
    if let Ok(function_files) = discovery.discover_function_files() {
        models.extend(function_files);
    }

    let db = init_db(workspace, &models);
    let ws = Workspace::try_get(&db).expect("workspace initialized");

    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut push = |sev: DiagnosticSeverity, rel: &str, code: &str, msg: &str| {
        let line = format!("[{code}] {rel}: {msg}");
        match sev {
            DiagnosticSeverity::Error => errors.push(line),
            DiagnosticSeverity::Warning => warnings.push(line),
            _ => {}
        }
    };

    for model in &models {
        let Some(file) = db.source_file(&model.path) else {
            continue;
        };
        let rel = model
            .path
            .strip_prefix(workspace)
            .unwrap_or(&model.path)
            .display()
            .to_string();
        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            push(d.severity, &rel, &format!("{:?}", d.code), &d.message);
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            push(d.0.severity, &rel, &format!("{:?}", d.0.code), &d.0.message);
        }
    }

    (errors, warnings)
}

/// Single sweep over every example workspace, asserting the category-appropriate
/// outcome for each. One test (rather than one per workspace) keeps the
/// allow-list and category logic in one place and the `--nocapture` log linear.
#[test]
fn every_example_builds_or_is_accounted_for() {
    let allow: std::collections::HashMap<&str, &str> = KNOWN_UNBUILDABLE.iter().copied().collect();
    let never: std::collections::HashMap<&str, &str> = NEVER_BUILD.iter().copied().collect();
    let builds_clean: std::collections::HashSet<&str> =
        BROKEN_BUILDS_CLEAN.iter().copied().collect();
    let only = only_filter();
    if let Some(only) = &only {
        eprintln!(
            "SMELT_EXAMPLE_BUILDS_ONLY set — running {} workspace(s): {:?}",
            only.len(),
            only
        );
    }

    let mut failures: Vec<String> = Vec::new();

    for ws in example_workspaces() {
        let name = ws.file_name().unwrap().to_string_lossy().to_string();

        // Per-phase scoping: skip anything not in the requested subset.
        if only.as_ref().is_some_and(|set| !set.contains(&name)) {
            continue;
        }

        // Never-build set: log and skip without spawning a build (pure cost).
        if let Some(reason) = never.get(name.as_str()) {
            eprintln!("SKIP (NEVER_BUILD, not attempted) {name}: {reason}");
            continue;
        }

        if let Some(reason) = allow.get(name.as_str()) {
            // Allow-listed: log the reason, confirm it still fails to build.
            let out = run_build(&ws);
            let combined = tail_output(
                &format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                ),
                30,
            );
            eprintln!(
                "SKIP (KNOWN_UNBUILDABLE) {name}: {reason}\n  observed exit: {:?}",
                out.status.code()
            );
            if out.status.success() {
                failures.push(format!(
                    "'{name}' is on KNOWN_UNBUILDABLE but `smelt build` now SUCCEEDS — \
                     remove it from the allow-list. Recorded reason was: {reason}\n\
                     build output:\n{combined}"
                ));
            }
            continue;
        }

        if name.contains("broken") {
            // Broken fixture. The defect must be caught *somewhere*: either the
            // analyzer rejects it with an `Error` diagnostic, or `smelt build`
            // fails. (Some defects — e.g. the incremental safety classifier or
            // generator-body validators — surface at build time, not through
            // `file_diagnostics`.) A small named set intentionally builds clean
            // because the "broken" scenario is one the framework suppresses.
            if builds_clean.contains(name.as_str()) {
                let out = run_build(&ws);
                eprintln!(
                    "BROKEN (intentionally builds clean) {name}: exit {:?}",
                    out.status.code()
                );
                if !out.status.success() {
                    let combined = tail_output(
                        &format!(
                            "{}{}",
                            String::from_utf8_lossy(&out.stdout),
                            String::from_utf8_lossy(&out.stderr)
                        ),
                        30,
                    );
                    failures.push(format!(
                        "broken fixture '{name}' (BROKEN_BUILDS_CLEAN) was expected to \
                         build clean but `smelt build` failed (exit {:?}).\n{combined}",
                        out.status.code()
                    ));
                }
                continue;
            }

            let (errors, _warnings) = collect_diagnostics(&ws);
            if !errors.is_empty() {
                eprintln!(
                    "BROKEN (analyzer Error) {name}: {} error(s): {}",
                    errors.len(),
                    errors.join("; ")
                );
                continue;
            }

            // No analyzer Error — the rejection must come from the build itself.
            let out = run_build(&ws);
            if out.status.success() {
                let combined = tail_output(
                    &format!(
                        "{}{}",
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr)
                    ),
                    30,
                );
                failures.push(format!(
                    "broken fixture '{name}' produced no Error-severity diagnostic AND \
                     `smelt build` SUCCEEDED — the defect is not being caught. If the \
                     scenario is intentionally suppressed, add it to \
                     BROKEN_BUILDS_CLEAN.\nbuild output:\n{combined}"
                ));
            } else {
                eprintln!(
                    "BROKEN (build failure) {name}: exit {:?}",
                    out.status.code()
                );
            }
            continue;
        }

        // Clean workspace: `smelt build` must compile and execute every model.
        let out = run_build(&ws);
        if out.status.success() {
            eprintln!("PASS {name}");
        } else {
            let combined = tail_output(
                &format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                ),
                30,
            );
            failures.push(format!(
                "clean workspace '{name}' failed `smelt build` (exit {:?}). If this is a \
                 real codegen gap, add it to KNOWN_UNBUILDABLE with the observed reason; \
                 otherwise fix the workspace.\nbuild output:\n{combined}",
                out.status.code()
            ));
        }
    }

    let shown = failures
        .iter()
        .take(MAX_DETAILED_FAILURES)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n\n");
    let overflow = failures.len().saturating_sub(MAX_DETAILED_FAILURES);
    let overflow_note = if overflow > 0 {
        let names: Vec<&str> = failures[MAX_DETAILED_FAILURES..]
            .iter()
            .map(|f| f.lines().next().unwrap_or(f).trim())
            .collect();
        format!(
            "\n\n… and {overflow} more failure(s) (detail capped at {MAX_DETAILED_FAILURES}): {}",
            names.join(" | ")
        )
    } else {
        String::new()
    };
    assert!(
        failures.is_empty(),
        "example_builds gate found {} problem(s):\n\n{shown}{overflow_note}",
        failures.len(),
    );
}

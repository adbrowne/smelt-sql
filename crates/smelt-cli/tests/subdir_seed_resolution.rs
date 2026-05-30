//! Regression: a seed nested under a sub-directory of a scan root must be a
//! valid `smelt.<full.path>` ref target through the real CLI discovery +
//! logical-graph build path — not only through the Salsa/LSP resolver.
//!
//! `examples/ephemeral_demo` declares an ephemeral seed at
//! `models/lookup/regions.csv` (sidecar `models/lookup/regions.yml`) and a
//! model `region_report` that selects from `smelt.lookup.regions`. The LSP
//! example gates (`example_diagnostics`, `example_workspaces`) asserted the
//! workspace was clean, but the CLI `build`/`explain`/`run` path keyed its
//! seed set by leaf `name` (`"regions"`) instead of the canonical dot-path
//! (`"lookup.regions"`), so it rejected the reference with
//! `references undefined model/source 'lookup.regions'`. That asymmetry — LSP
//! clean, CLI broken — is exactly the bug class `CLAUDE.md` warns about. This
//! test drives `build_logical_graph` (the shared CLI discovery path) so the
//! seam is covered at the example level, not just by a unit test.

use std::path::PathBuf;

fn project_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("examples/ephemeral_demo")
}

#[test]
fn ephemeral_demo_subdir_seed_resolves_in_cli_path() {
    let project_dir = project_dir();
    assert!(
        project_dir.join("models/lookup/regions.csv").exists(),
        "fixture must contain the sub-directory seed: {}",
        project_dir.display()
    );

    let config = smelt_cli::Config::load(&project_dir).expect("load ephemeral_demo config");

    // Discover seeds exactly as `smelt run`/`build` does (run.rs).
    let seeds = smelt_core::discover_seed_infos_with_sidecars(&project_dir, &config.paths);
    assert!(
        seeds
            .iter()
            .any(|s| s.address_segments == ["lookup", "regions"]),
        "discovery must find the sub-directory seed lookup.regions; saw {:?}",
        seeds
            .iter()
            .map(|s| &s.address_segments)
            .collect::<Vec<_>>()
    );

    // The build itself runs dependency validation; before the fix this returned
    // `references undefined model/source 'lookup.regions'`.
    let (graph, _db) = smelt_cli::build_logical_graph(&project_dir, &config, None, &seeds, "dev")
        .expect("logical graph must build: sub-directory seed must resolve as a ref target");

    let model_names: Vec<&str> = graph.iter_nodes().map(|n| n.name.as_str()).collect();
    assert!(
        model_names.contains(&"region_report"),
        "region_report must be in the graph; saw {model_names:?}"
    );
}

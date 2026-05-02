//! Phase 2a — `smelt-core` consumes the unified `smelt.<path>` AST nodes.
//!
//! The data plane should:
//!   - extract path-form refs from a parsed file with kind dispatch decided
//!     by the workspace file format,
//!   - dispatch `smelt.seeds.*` to seeds and `smelt.sources.*` to sources,
//!   - build a path-tuple-keyed dependency graph from the workspace.
//!
//! Tests use `examples/test_workspace/` as a real-fixture oracle and ad-hoc
//! tempdirs for shape-only cases. They drive `extract_refs` through the new
//! `SmeltRef::Path` variant and the unified `to_path` adapter.

use std::path::PathBuf;

use smelt_core::refs::{extract_refs, SmeltRef};
use smelt_core::{discover_seed_infos, DependencyGraph, ModelDiscovery, SourcesConfig};
use smelt_parser::File as AstFile;

fn parse(sql: &str) -> AstFile {
    let parse = smelt_parser::parse(sql);
    AstFile::cast(parse.syntax()).expect("file ast")
}

#[test]
fn extracts_path_refs_from_unified_ast() {
    // Path form in FROM position. The unified ref carrier is `SmeltRef::Path`
    // with the segments after `smelt.` — `["models", "upstream"]`.
    let sql = "SELECT * FROM smelt.models.upstream\n";
    let file = parse(sql);
    let refs = extract_refs(&file);

    assert_eq!(refs.len(), 1, "one path-form ref expected");
    let SmeltRef::Path(segments) = &refs[0].smelt_ref;
    assert_eq!(
        segments,
        &vec!["models".to_string(), "upstream".to_string()]
    );
}

#[test]
fn extracts_path_refs_for_seed_and_source() {
    // Path-form refs to a seed (`smelt.seeds.raw.users`) and a source
    // (`smelt.sources.raw.events`). The carrier is `SmeltRef::Path` for
    // both — kind dispatch happens at resolution time, not extraction time.
    let sql = "\
SELECT *
FROM smelt.seeds.raw.users s
JOIN smelt.sources.raw.events e ON s.id = e.user_id
";
    let file = parse(sql);
    let refs = extract_refs(&file);
    assert_eq!(refs.len(), 2, "expected two path-form refs, got {refs:#?}");

    let paths: Vec<Vec<String>> = refs
        .iter()
        .map(|r| {
            let SmeltRef::Path(segs) = &r.smelt_ref;
            segs.clone()
        })
        .collect();

    assert!(paths.contains(&vec![
        "seeds".to_string(),
        "raw".to_string(),
        "users".to_string()
    ]));
    assert!(paths.contains(&vec![
        "sources".to_string(),
        "raw".to_string(),
        "events".to_string()
    ]));
}

#[test]
fn path_refs_dependency_graph() {
    // `DependencyGraph::build_from_workspace` keys edges on path tuples.
    // We use the test_workspace fixture (which already contains `path_demo.sql`
    // referencing `smelt.models.users` along with legacy `smelt.ref` /
    // `smelt.source` files).
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("test_workspace");
    assert!(
        workspace_root.exists(),
        "fixture missing: {workspace_root:?}"
    );

    let discovery = ModelDiscovery::new(workspace_root.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");

    // Load sources.yml so legacy `smelt.source(...)` refs validate.
    let sources_yaml_path = workspace_root.join("sources.yml");
    let sources_text = std::fs::read_to_string(&sources_yaml_path).unwrap_or_default();
    let sources_config: SourcesConfig = serde_yaml::from_str(&sources_text).unwrap_or_default();

    // Seeds: discover (will be empty unless we add some).
    let _seeds = discover_seed_infos(&workspace_root, &["seeds".to_string()]);

    let graph =
        DependencyGraph::build_from_workspace(models, Some(&sources_config), &workspace_root)
            .expect("build path-tuple graph");

    // Every dependency edge must be path-tuple keyed (Vec<String>), not a
    // bare model name. Look up the path-form fixture's dependency on
    // `smelt.models.users` — it must appear as a path tuple
    // `["models", "users"]` in the graph's edges.
    let path_demo_key = vec!["models".to_string(), "path_demo".to_string()];
    let deps = graph
        .path_dependencies(&path_demo_key)
        .unwrap_or_else(|| panic!("path_demo not in path-tuple graph"));

    let users_tuple = vec!["models".to_string(), "users".to_string()];
    assert!(
        deps.iter().any(|d| d == &users_tuple),
        "path_demo should depend on smelt.models.users — got {deps:#?}"
    );
}

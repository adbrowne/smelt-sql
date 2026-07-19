//! Unit tests for `resolve_address_map` — the single address-uniqueness
//! authority that operates on post-discovery descriptor sets (BUG-002, BUG-021).
//!
//! Each test constructs the descriptor sets directly (no filesystem I/O),
//! passing in `address_segments` that the caller would normally compute via
//! `ModelDiscovery::compute_address_segments` / seed/source discovery.

use smelt_core::discovery::{ModelFile, ModelKind};
use smelt_core::model_id::ModelId;
use smelt_core::resolver::{resolve_address_map, EntityRefKind};
use smelt_core::seeds::SeedInfo;
use smelt_core::sources::SourceInfo;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Minimal descriptor constructors
// ---------------------------------------------------------------------------

fn make_model(name: &str, segments: &[&str], path: &str) -> ModelFile {
    ModelFile {
        name: name.to_string(),
        path: PathBuf::from(path),
        content: String::new(),
        refs: vec![],
        parse_errors: vec![],
        metadata: None,
        kind: ModelKind::Sql,
        model_id: ModelId::from_path(PathBuf::from(path)),
        address_segments: segments.iter().map(|s| s.to_string()).collect(),
    }
}

fn make_seed(name: &str, segments: &[&str], path: &str) -> SeedInfo {
    SeedInfo {
        name: name.to_string(),
        path: PathBuf::from(path),
        columns: vec![],
        address_segments: segments.iter().map(|s| s.to_string()).collect(),
        sidecar: None,
    }
}

fn make_source(segments: &[&str], path: &str) -> SourceInfo {
    SourceInfo {
        path: PathBuf::from(path),
        address_segments: segments.iter().map(|s| s.to_string()).collect(),
        columns: vec![],
        description: None,
        name_override: None,
        tags: vec![],
        timeseries: None,
        mutation_profile: None,
        source_lateness: None,
        watermark: None,
        unique_key: None,
        retention: None,
        referential_integrity: None,
    }
}

// ---------------------------------------------------------------------------
// BUG-002: cross-kind address collisions
// ---------------------------------------------------------------------------

/// A SQL model and a seed claiming the same address → one collision.
/// Repro: `models/dup.sql` + `models/dup.csv` (architecture.md canonical case).
#[test]
fn model_vs_seed_collision_is_detected() {
    let models = vec![make_model("dup", &["dup"], "models/dup.sql")];
    let seeds = vec![make_seed("dup", &["dup"], "models/dup.csv")];

    let (_map, collisions) = resolve_address_map(&models, &seeds, &[]);

    assert_eq!(
        collisions.len(),
        1,
        "expected one collision, got: {collisions:#?}"
    );
    assert_eq!(collisions[0].address, vec!["dup".to_string()]);
    // The first entity is the model (registered first).
    assert_eq!(collisions[0].first.kind, EntityRefKind::SqlModel);
    assert_eq!(collisions[0].second.kind, EntityRefKind::Seed);
}

/// A SQL model and a source claiming the same address → one collision.
#[test]
fn model_vs_source_collision_is_detected() {
    let models = vec![make_model(
        "events",
        &["raw", "events"],
        "models/raw/events.sql",
    )];
    let sources = vec![make_source(&["raw", "events"], "models/raw/events.yml")];

    let (_map, collisions) = resolve_address_map(&models, &[], &sources);

    assert_eq!(
        collisions.len(),
        1,
        "expected one collision, got: {collisions:#?}"
    );
    assert_eq!(
        collisions[0].address,
        vec!["raw".to_string(), "events".to_string()]
    );
}

/// Two seeds with the same address (e.g. from different scan-root paths)
/// → one collision.  Mirrors the old `cross_paths_collision_errors` case from
/// resolver_kinds.rs but operates on the descriptor level.
#[test]
fn cross_paths_seed_collision_is_detected() {
    let seeds = vec![
        make_seed("users", &["users"], "models/users.csv"),
        make_seed("users", &["users"], "fixtures/users.csv"),
    ];

    let (_map, collisions) = resolve_address_map(&[], &seeds, &[]);

    assert_eq!(
        collisions.len(),
        1,
        "expected one collision, got: {collisions:#?}"
    );
    assert_eq!(collisions[0].address, vec!["users".to_string()]);
    let p1 = collisions[0].first.path.to_string_lossy();
    let p2 = collisions[0].second.path.to_string_lossy();
    assert!(
        (p1.contains("models") && p2.contains("fixtures"))
            || (p1.contains("fixtures") && p2.contains("models")),
        "collision should name both paths: {p1} and {p2}"
    );
}

// ---------------------------------------------------------------------------
// BUG-021: within-file `--- name: dup ---` section collision
// ---------------------------------------------------------------------------

/// Two ModelFile entries (from expanding a multi-model `.sql` file) that
/// both declare the same leaf name → same address → collision detected.
#[test]
fn within_file_section_collision_is_detected() {
    // Both sections live in the same file (virtual paths differ by the
    // multi_model suffix; discovery.rs already expands them).
    let models = vec![
        make_model("dup", &["dup"], "models/multi.sql::dup_a"),
        make_model("dup", &["dup"], "models/multi.sql::dup_b"),
    ];

    let (_map, collisions) = resolve_address_map(&models, &[], &[]);

    assert_eq!(
        collisions.len(),
        1,
        "expected one collision, got: {collisions:#?}"
    );
    assert_eq!(collisions[0].address, vec!["dup".to_string()]);
}

// ---------------------------------------------------------------------------
// Correct non-collision cases
// ---------------------------------------------------------------------------

/// `models/users.sql` (address `["users"]`) and
/// `models/archive/users.sql` (address `["archive","users"]`) are **not**
/// a collision — the subdirectory is real disambiguation.
#[test]
fn subdirectory_gives_distinct_address_no_collision() {
    let models = vec![
        make_model("users", &["users"], "models/users.sql"),
        make_model("users", &["archive", "users"], "models/archive/users.sql"),
    ];

    let (map, collisions) = resolve_address_map(&models, &[], &[]);

    assert!(
        collisions.is_empty(),
        "distinct addresses must not collide, got: {collisions:#?}"
    );
    assert_eq!(map.len(), 2, "both entities should be in the map");
    assert!(map.contains_key("users"));
    assert!(map.contains_key("archive.users"));
}

/// Empty inputs produce an empty map and no collisions.
#[test]
fn empty_inputs_produce_empty_output() {
    let (map, collisions) = resolve_address_map(&[], &[], &[]);
    assert!(map.is_empty());
    assert!(collisions.is_empty());
}

/// Entities whose `address_segments` is empty (unknown scan-root) are skipped
/// and do not produce false positives.
#[test]
fn entities_with_empty_segments_are_skipped() {
    let models = vec![
        make_model("a", &[], "models/a.sql"),  // empty segments
        make_model("a", &[], "models/a2.sql"), // also empty — would collide if registered
    ];

    let (map, collisions) = resolve_address_map(&models, &[], &[]);

    assert!(
        collisions.is_empty(),
        "empty segments must not produce collisions"
    );
    assert!(map.is_empty());
}

/// Multiple entities with distinct addresses → all land in the map, no collisions.
#[test]
fn multiple_distinct_entities_no_collision() {
    let models = vec![
        make_model(
            "staging_events",
            &["staging", "events"],
            "models/staging/events.sql",
        ),
        make_model("marts_users", &["marts", "users"], "models/marts/users.sql"),
    ];
    let seeds = vec![make_seed(
        "raw_orders",
        &["raw", "orders"],
        "seeds/raw/orders.csv",
    )];
    let sources = vec![make_source(
        &["external", "api"],
        "models/sources/external/api.yml",
    )];

    let (map, collisions) = resolve_address_map(&models, &seeds, &sources);

    assert!(
        collisions.is_empty(),
        "no collisions expected, got: {collisions:#?}"
    );
    assert_eq!(map.len(), 4);
}

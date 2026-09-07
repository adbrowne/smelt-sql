//! Standing drift gate closing `docs/outcomes/20260906-scd2-keyed-succession`
//! criterion 10: the succession grain's specs must stop claiming it is
//! unimplemented now that phases 2-9 landed the classifier, plan, emitters,
//! runtime driver, and conformance coverage.

use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn read_spec(name: &str) -> String {
    fs::read_to_string(repo_root().join("docs/specs").join(name)).unwrap()
}

/// Every backticked `crates/…`, `docs/…`, `examples/…` path cited in
/// `incremental_shapes.md` §References §"The succession grain" must exist on
/// disk (same shape as `state_docs_freshness::spec_references_are_live`,
/// scoped to that subsection so it cannot trip on out-of-scope path drift
/// elsewhere in the file).
#[test]
fn succession_spec_references_are_live() {
    let text = read_spec("incremental_shapes.md");

    let heading = "### The succession grain";
    let references_heading_pos = text
        .find("## References")
        .expect("incremental_shapes.md has no §References section");
    let section_start = text[references_heading_pos..]
        .find(heading)
        .map(|i| i + references_heading_pos)
        .expect("§References has no \"The succession grain\" subsection");
    let rest = &text[section_start + heading.len()..];
    let section_end = rest.find("\n### ").unwrap_or(rest.len());
    let section = &rest[..section_end];

    let paths: Vec<&str> = section
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|s| {
            s.starts_with("crates/") || s.starts_with("docs/") || s.starts_with("examples/")
        })
        .collect();

    assert!(
        !paths.is_empty(),
        "incremental_shapes.md §References §\"The succession grain\" cites no backtick-quoted \
         crates/docs/examples paths — expected Code and Tests bullets naming the landed files"
    );

    let root = repo_root();
    let missing: Vec<&str> = paths
        .into_iter()
        .filter(|p| !root.join(p.trim_end_matches('/')).exists())
        .collect();

    assert!(
        missing.is_empty(),
        "incremental_shapes.md §References §\"The succession grain\" cites paths that do not \
         exist on disk: {missing:?}"
    );
}

/// The three now-false "not yet built" bullets must be gone from §Known
/// Divergences, and `diagnostics.md` must no longer claim the twelve
/// succession codes are unimplemented.
#[test]
fn succession_divergences_are_not_stale() {
    let shapes = read_spec("incremental_shapes.md");
    let diagnostics = read_spec("diagnostics.md");

    let retired_phrases = [
        "No implementation exists yet",
        "do not exist yet",
        "has no arrival-partitioned source recipe",
    ];

    let divergences_heading = "### The succession grain";
    // There are two "### The succession grain" headings in incremental_shapes.md
    // (the body section and the Known Divergences subsection); scope to the
    // Known Divergences occurrence.
    let known_divergences_pos = shapes
        .find("## Known Divergences")
        .expect("incremental_shapes.md has no §Known Divergences section");
    let rest = &shapes[known_divergences_pos..];
    let section_start = rest
        .find(divergences_heading)
        .map(|i| i + known_divergences_pos)
        .expect("§Known Divergences has no \"The succession grain\" subsection");
    let after = &shapes[section_start + divergences_heading.len()..];
    let section_end = after
        .find("\n### ")
        .or_else(|| after.find("\n## "))
        .unwrap_or(after.len());
    let section = &after[..section_end];

    for phrase in retired_phrases {
        assert!(
            !section.contains(phrase),
            "incremental_shapes.md §Known Divergences §\"The succession grain\" still contains \
             the retired phrase {phrase:?}"
        );
    }

    assert!(
        !diagnostics.contains("succession-grain codes are specified and unimplemented"),
        "diagnostics.md still carries a \"specified and unimplemented\" succession bullet"
    );
}

/// `model_properties.md`'s keyed-succession declarations-table row and
/// `model_transforms.md`'s succession-patch row must both read `built`.
#[test]
fn succession_status_rows_read_built() {
    let properties = read_spec("model_properties.md");
    let transforms = read_spec("model_transforms.md");

    let properties_row = properties
        .lines()
        .find(|line| line.contains("Keyed-succession classification"))
        .expect("model_properties.md has no Keyed-succession classification row");
    assert!(
        properties_row.trim_end().ends_with("| built |"),
        "model_properties.md's Keyed-succession classification row does not end `built`: \
         {properties_row:?}"
    );

    let transforms_row = transforms
        .lines()
        .find(|line| line.contains("Succession-patch keyed"))
        .expect("model_transforms.md has no Succession-patch keyed row");
    assert!(
        transforms_row.trim_end().ends_with("| built |"),
        "model_transforms.md's Succession-patch keyed row does not end `built`: \
         {transforms_row:?}"
    );
}

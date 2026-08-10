//! Standing gate for `docs/specs/incremental_models.md` §"The graph layer" →
//! "Typed edges" (`docs/outcomes/20260809-output-delta-typing/outcome.md`
//! phase 3): the typed-edge component vector's three named parts and the
//! widen-never-narrow degrade rule exist as specified.

use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/smelt-logical has a parent dir")
        .parent()
        .expect("crates/ has a parent dir")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

fn section_body<'a>(doc: &'a str, heading: &str) -> &'a str {
    let start = doc
        .find(heading)
        .unwrap_or_else(|| panic!("document must have a {heading:?} heading"));
    let after_heading = &doc[start..];
    let body_start = after_heading
        .find('\n')
        .map(|i| i + 1)
        .unwrap_or(after_heading.len());
    let body = &after_heading[body_start..];
    let end = body
        .find("\n## ")
        .or_else(|| body.find("\n### "))
        .unwrap_or(body.len());
    &body[..end]
}

#[test]
fn typed_edges_section_names_the_three_component_parts() {
    let incremental_models = read("docs/specs/incremental_models.md");
    let section = section_body(&incremental_models, "### The graph layer");

    assert!(
        section.contains("Typed edges"),
        "§\"The graph layer\" must have a \"Typed edges\" subsection"
    );
    assert!(
        section.contains("delta shape") || section.contains("output-delta verdict"),
        "the typed-edge component must name the delta-shape part"
    );
    assert!(
        section.contains("addressing"),
        "the typed-edge component must name the addressing part"
    );
    assert!(
        section.contains("column set"),
        "the typed-edge component must name the column-set part"
    );
    assert!(
        section.contains("Widen-never-narrow") || section.contains("widen-never-narrow"),
        "§\"The graph layer\" must state the widen-never-narrow degrade rule for typed \
         components, not only for interval propagation"
    );
    assert!(
        section.contains("never to nothing") || section.contains("degrades to the coarsest"),
        "the widen-never-narrow degrade must state that an unprojectable component degrades \
         to whole-model dirt, never disappears"
    );
}

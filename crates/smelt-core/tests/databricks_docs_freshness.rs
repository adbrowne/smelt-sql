//! Standing drift gate for docs-site's description of the `databricks` target
//! (`docs/outcomes/20260912-databricks-dogfood-spine/outcome.md` criterion 9's
//! docs-site half). The refused-key list and the bare-hostname rule are both
//! enforced in `crates/smelt-core/src/config.rs::Config::validate_targets` —
//! this test keeps `docs-site/` in sync with that code rather than restating
//! it independently and letting the two drift.

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

fn config_source() -> String {
    let path = repo_root().join("crates/smelt-core/src/config.rs");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

/// Parses the key names out of `DATABRICKS_FOREIGN_KEYS` in `config.rs` by
/// regex over the source rather than restating the list — the const is
/// private, so this is the only way a test can see it without exporting
/// implementation detail across the crate boundary.
fn databricks_foreign_keys() -> Vec<String> {
    let source = config_source();
    let start = source
        .find("const DATABRICKS_FOREIGN_KEYS")
        .expect("crates/smelt-core/src/config.rs has no `DATABRICKS_FOREIGN_KEYS` const");
    let end = source[start..]
        .find("];")
        .map(|i| start + i)
        .expect("DATABRICKS_FOREIGN_KEYS const has no closing `];`");
    let body = &source[start..end];

    let mut keys = Vec::new();
    let mut rest = body;
    while let Some(open) = rest.find('"') {
        let after_open = &rest[open + 1..];
        let close = after_open
            .find('"')
            .expect("unterminated string literal in DATABRICKS_FOREIGN_KEYS");
        keys.push(after_open[..close].to_string());
        rest = &after_open[close + 1..];
    }

    assert!(
        !keys.is_empty(),
        "found DATABRICKS_FOREIGN_KEYS but extracted no key names from it"
    );
    keys
}

fn targets_doc() -> String {
    let path = repo_root().join("docs-site/docs/guide/targets.md");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

/// The `### Databricks` section alone — scoped so a key name that also
/// happens to be a Spark/BigQuery field (`warehouse`, `format`, `database`,
/// `project`, `dataset`, `location`) doesn't pass this test by appearing in
/// an unrelated section.
fn databricks_section(doc: &str) -> &str {
    let start = doc
        .find("### Databricks")
        .expect("docs-site/docs/guide/targets.md has no `### Databricks` section");
    let rest = &doc[start..];
    let end = rest[1..].find("\n## ").map(|i| i + 1).unwrap_or(rest.len());
    &rest[..end]
}

/// A new foreign key added to `DATABRICKS_FOREIGN_KEYS` without documenting
/// it as refused fails this test.
#[test]
fn docs_site_names_every_refused_databricks_key() {
    let doc = targets_doc();
    let section = databricks_section(&doc);
    let keys = databricks_foreign_keys();

    let missing: Vec<&String> = keys
        .iter()
        .filter(|k| !section.contains(k.as_str()))
        .collect();

    assert!(
        missing.is_empty(),
        "docs-site/docs/guide/targets.md's Databricks section does not name every key \
         DATABRICKS_FOREIGN_KEYS refuses: {missing:?}"
    );
}

/// `validate_targets`' own diagnostic requires `host` to be a bare hostname —
/// no scheme, no trailing slash. The docs-site section must state the same
/// rule, not just show an example that happens to comply with it.
#[test]
fn docs_site_states_the_databricks_host_rule() {
    let doc = targets_doc();
    let section = databricks_section(&doc);

    assert!(
        section.contains("bare hostname"),
        "docs-site/docs/guide/targets.md's Databricks section does not state that `host` must \
         be a bare hostname"
    );
    assert!(
        section.contains("no scheme") && section.contains("no trailing slash"),
        "docs-site/docs/guide/targets.md's Databricks section does not state both halves of \
         the bare-hostname rule (no scheme, no trailing slash)"
    );
}

/// Criterion 11's Asset Bundle deployment form (docs/outcomes/
/// 20260912-databricks-dogfood-spine/outcome.md, phases/11a-plan.md): the
/// docs-site guide must name the actual deploy path and the Volume state
/// location, so the guide cannot drift from what's committed at
/// `examples/github_activity/databricks.yml`.
#[test]
fn docs_site_names_the_bundle_deploy_path_and_volume_state_location() {
    let doc = targets_doc();
    let section = databricks_section(&doc);

    assert!(
        section.contains("bundle deploy") && section.contains("bundle run"),
        "docs-site/docs/guide/targets.md's Databricks section does not name the `databricks \
         bundle deploy`/`bundle run` deployment path"
    );
    assert!(
        section.contains("Volume"),
        "docs-site/docs/guide/targets.md's Databricks section does not describe the \
         Volume-resident project/state path"
    );
}

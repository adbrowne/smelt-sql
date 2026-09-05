//! Standing drift gate for docs-site's description of state residency
//! (`docs/outcomes/20260904-state-residency/outcome.md` criterion 8's docs-site half).
//! The reconciliation ledger moved into the target backend (an engine-resident
//! `_smelt_ledger` table, transactional with the fold it protects) and
//! `execute_project` now honours `state.mode`'s per-posture write set — these
//! three checks keep `docs-site/` from re-asserting the pre-residency shape.

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

fn docs_site_dir() -> PathBuf {
    repo_root().join("docs-site/docs")
}

fn walk_markdown_files(dir: &PathBuf, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            walk_markdown_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push(path);
        }
    }
}

/// The reconciliation ledger is engine-resident since
/// `docs/outcomes/20260904-state-residency` phase 2 — no user-facing page may
/// still claim a `.smelt/`-resident `reconciliation.json` file.
#[test]
fn user_docs_never_claim_a_reconciliation_json_file() {
    let mut files = Vec::new();
    walk_markdown_files(&docs_site_dir(), &mut files);

    let offenders: Vec<String> = files
        .into_iter()
        .filter(|path| {
            let text = fs::read_to_string(path).unwrap();
            text.contains("reconciliation.json")
        })
        .map(|path| {
            path.strip_prefix(repo_root())
                .unwrap()
                .display()
                .to_string()
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "docs-site pages still claim a `.smelt/`-resident reconciliation.json file \
         (the ledger is engine-resident since phase 2): {offenders:?}"
    );
}

/// `docs/specs/smelt_yml.md` §"Top-Level Fields" documents a `state` key
/// (`mode` + `warehouse_tables`); the docs-site reference must too.
#[test]
fn smelt_yml_reference_documents_the_state_block() {
    let text = fs::read_to_string(docs_site_dir().join("reference/smelt-yml.md")).unwrap();

    assert!(
        text.contains("| `state` |"),
        "docs-site/docs/reference/smelt-yml.md's Top-Level Fields table has no `state` row"
    );
    assert!(
        text.contains("`mode`") || text.contains("mode:"),
        "docs-site/docs/reference/smelt-yml.md does not document `state.mode`"
    );
    assert!(
        text.contains("warehouse_tables"),
        "docs-site/docs/reference/smelt-yml.md does not document `state.warehouse_tables`"
    );
}

/// `docs/specs/state.md` §"The residency rule": deleting `.smelt/` never
/// changes what a maintained model computes. The user-facing state reference
/// must state that invariant and the per-posture (`state.mode`) write set.
#[test]
fn state_reference_states_the_residency_invariant() {
    let text = fs::read_to_string(docs_site_dir().join("reference/state.md")).unwrap();

    assert!(
        text.contains("stateless"),
        "docs-site/docs/reference/state.md has no per-posture write-set section naming `stateless`"
    );
    assert!(
        text.contains("does not change what")
            || text.contains("never change what")
            || text.contains("never changes what"),
        "docs-site/docs/reference/state.md's recovery playbook does not state that deleting \
         `.smelt/` does not change what a maintained model computes"
    );
}

/// `docs/specs/state.md` §References must not silently rot: every path it
/// lists under Code / User docs must exist, and neither list may still read
/// the placeholder `none yet` now that phases 1-10 landed real code and docs.
#[test]
fn spec_references_are_live() {
    let text = fs::read_to_string(repo_root().join("docs/specs/state.md")).unwrap();

    let references_start = text
        .find("## References")
        .expect("docs/specs/state.md has no §References section");
    let references = &text[references_start..];

    let section = |heading: &str| -> String {
        let start = references
            .find(heading)
            .unwrap_or_else(|| panic!("§References has no `{heading}` bullet"));
        let rest = &references[start..];
        let end = rest[heading.len()..]
            .find("\n- **")
            .map(|i| i + heading.len())
            .unwrap_or(rest.len());
        rest[..end].to_string()
    };

    let code_section = section("**Code**:");
    let user_docs_section = section("**User docs**:");

    assert!(
        !code_section.contains("none yet"),
        "§References → Code still reads `none yet`"
    );
    assert!(
        !user_docs_section.contains("none yet"),
        "§References → User docs still reads `none yet`"
    );

    let paths: Vec<&str> = code_section
        .split('`')
        .skip(1)
        .step_by(2)
        .chain(user_docs_section.split('`').skip(1).step_by(2))
        .filter(|s| {
            s.starts_with("crates/") || s.starts_with("docs/") || s.starts_with("docs-site/")
        })
        .collect();

    assert!(
        !paths.is_empty(),
        "no backtick-quoted paths found in §References → Code / User docs"
    );

    let root = repo_root();
    let missing: Vec<&str> = paths
        .into_iter()
        .filter(|p| !root.join(p.trim_end_matches('/')).exists())
        .collect();

    assert!(
        missing.is_empty(),
        "docs/specs/state.md §References cites paths that do not exist on disk: {missing:?}"
    );
}

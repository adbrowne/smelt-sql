//! Standing drift gate for the docs-site coverage of external steps
//! (`docs/outcomes/20260906-external-dag-steps/outcome.md` phase 8, success
//! criterion 6's docs half). Plain `fs` reads over `docs-site/docs`, no
//! warehouse — modelled on `state_docs_freshness.rs`.

use std::fs;
use std::path::{Path, PathBuf};

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

fn guide_page_text() -> String {
    fs::read_to_string(docs_site_dir().join("guide/external-steps.md"))
        .expect("docs-site/docs/guide/external-steps.md must exist")
}

#[test]
fn external_steps_guide_page_exists_and_is_in_the_nav() {
    // Existence is asserted by guide_page_text()'s expect; check the nav entry.
    let _ = guide_page_text();

    let nav = fs::read_to_string(repo_root().join("docs-site/mkdocs.yml")).unwrap();
    let occurrences = nav.matches("guide/external-steps.md").count();
    assert_eq!(
        occurrences, 1,
        "mkdocs.yml nav must reference guide/external-steps.md exactly once, found {occurrences}"
    );
}

#[test]
fn guide_page_documents_every_declaration_key() {
    let text = guide_page_text();
    for key in ["produces", "command", "cadence", "description"] {
        assert!(
            text.contains(key),
            "guide/external-steps.md does not mention declaration key `{key}`"
        );
    }
    for placeholder in ["{run_date}", "{run_end}"] {
        assert!(
            text.contains(placeholder),
            "guide/external-steps.md does not mention placeholder `{placeholder}`"
        );
    }
}

#[test]
fn guide_page_documents_every_failure_mode() {
    let text = guide_page_text();
    for code in [
        "ExternalStepFailed",
        "ExternalStepNotInvocable",
        "MalformedExternalStep",
    ] {
        assert!(
            text.contains(code),
            "guide/external-steps.md does not name diagnostic code `{code}`"
        );
    }
}

#[test]
fn guide_page_states_what_smelt_does_not_do() {
    let text = guide_page_text();
    assert!(
        text.contains("does not author")
            || text.contains("never author")
            || text.contains("does not authors"),
        "guide/external-steps.md must state that smelt never authors the command"
    );
    assert!(
        text.to_lowercase().contains("parse") && text.to_lowercase().contains("type-check")
            || text.to_lowercase().contains("type check"),
        "guide/external-steps.md must state that smelt never parses or type-checks the command"
    );
    assert!(
        text.to_lowercase().contains("idempoten"),
        "guide/external-steps.md must state that smelt guarantees no idempotence of the external program"
    );
    assert!(
        text.to_lowercase().contains("retr"),
        "guide/external-steps.md must state that smelt guarantees no retries beyond the existing policy"
    );
}

#[test]
fn sources_guide_no_longer_claims_smelt_never_loads_sources() {
    let text = fs::read_to_string(docs_site_dir().join("guide/sources.md")).unwrap();

    assert!(
        !text.contains("smelt does not load source data"),
        "guide/sources.md still flatly claims smelt never loads source data; \
         an external step now can — rewrite §\"Loading source data\""
    );
    assert!(
        text.contains("external-steps.md"),
        "guide/sources.md §\"Loading source data\" must link to guide/external-steps.md"
    );
}

#[test]
fn sources_yml_reference_documents_the_external_step_block() {
    let text = fs::read_to_string(docs_site_dir().join("reference/sources-yml.md")).unwrap();

    assert!(
        text.contains("external_step:") || text.contains("`external_step`"),
        "reference/sources-yml.md has no `external_step:` section"
    );
    for key in ["produces", "command", "cadence", "description"] {
        assert!(
            text.contains(key),
            "reference/sources-yml.md's external_step section does not mention `{key}`"
        );
    }
    assert!(
        text.to_lowercase().contains("columns") && text.to_lowercase().contains("forbidden"),
        "reference/sources-yml.md's external_step section must state that `columns:` is forbidden alongside it"
    );
}

#[test]
fn external_step_docs_carry_no_plan_vocabulary() {
    let re = regex_lite_phase_matcher();
    let mut offenders = Vec::new();

    for rel in [
        "guide/external-steps.md",
        "guide/sources.md",
        "reference/sources-yml.md",
    ] {
        let path = docs_site_dir().join(rel);
        let text = fs::read_to_string(&path).unwrap();
        if re(&text) {
            offenders.push(rel.to_string());
        }
    }

    assert!(
        offenders.is_empty(),
        "docs carry plan/phase vocabulary (timeless-oracle rule violation): {offenders:?}"
    );
}

/// Tiny stand-in for a `Phase [A-Z0-9]` regex without pulling in the `regex` crate.
fn regex_lite_phase_matcher() -> impl Fn(&str) -> bool {
    |text: &str| {
        let bytes = text.as_bytes();
        let needle = b"Phase ";
        let mut i = 0;
        while i + needle.len() < bytes.len() {
            if &bytes[i..i + needle.len()] == needle {
                let next = bytes[i + needle.len()];
                if next.is_ascii_alphanumeric() {
                    return true;
                }
            }
            i += 1;
        }
        false
    }
}

#[test]
fn external_step_docs_links_resolve() {
    let text = guide_page_text();
    let base_dir = docs_site_dir().join("guide");

    for link in extract_relative_markdown_links(&text) {
        let target = base_dir.join(&link);
        let target = normalize(&target);
        assert!(
            target.exists(),
            "guide/external-steps.md links to `{link}`, which resolves to {target:?} and does not exist"
        );
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn extract_relative_markdown_links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b']' && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
            let start = i + 2;
            if let Some(rel_end) = text[start..].find(')') {
                let link = &text[start..start + rel_end];
                let link = link.split('#').next().unwrap_or(link);
                if !link.is_empty()
                    && !link.starts_with("http://")
                    && !link.starts_with("https://")
                    && !link.starts_with('/')
                {
                    out.push(link.to_string());
                }
            }
        }
        i += 1;
    }
    out
}

#[allow(dead_code)]
fn all_docs_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_markdown_files(&docs_site_dir(), &mut out);
    out
}

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

/// Extracts the correctness-structure names whose Trino column reads `**no**`
/// in `docs/specs/state.md`'s realisability table (`§"Which dialects realise
/// which structure"`), by parsing the table rather than restating it.
fn trino_unrealisable_structures() -> Vec<String> {
    let text = fs::read_to_string(repo_root().join("docs/specs/state.md")).unwrap();

    let header_start = text
        .find("| Structure | DuckDB |")
        .expect("docs/specs/state.md has no realisability table header");
    let header_line_end = text[header_start..]
        .find('\n')
        .map(|i| header_start + i)
        .expect("realisability table header has no line end");
    let header = &text[header_start..header_line_end];
    let header_cols: Vec<&str> = header.split('|').map(str::trim).collect();
    let trino_idx = header_cols
        .iter()
        .position(|c| c.contains("Trino"))
        .expect("realisability table header has no Trino column");

    let mut structures = Vec::new();
    for line in text[header_line_end + 1..].lines() {
        if !line.trim_start().starts_with('|') {
            break;
        }
        if line.contains("---") {
            continue;
        }
        let cols: Vec<&str> = line.split('|').map(str::trim).collect();
        if cols.len() <= trino_idx {
            continue;
        }
        if !cols[trino_idx].contains("no") {
            continue;
        }
        structures.push(cols[1].to_string());
    }

    assert!(
        !structures.is_empty(),
        "found the realisability table but extracted no `**no**` Trino structures from it"
    );
    structures
}

/// `docs/outcomes/20260913-trino-ledger` phase 11: every structure the spec's
/// realisability table marks unrealisable on Trino must be named in the
/// docs-site state reference, so a Trino user can learn what Trino costs.
#[test]
fn docs_site_names_every_unrealisable_trino_structure() {
    let text = fs::read_to_string(docs_site_dir().join("reference/state.md")).unwrap();
    let structures = trino_unrealisable_structures();

    let missing: Vec<&String> = structures
        .iter()
        .filter(|s| !text.contains(s.as_str()))
        .collect();

    assert!(
        missing.is_empty(),
        "docs-site/docs/reference/state.md does not name every Trino-unrealisable structure \
         from docs/specs/state.md's realisability table: {missing:?}"
    );
}

/// The docs-site state page must state the measured reason (autocommit-only
/// Iceberg writes), quote the connector's own words, and say the absence is
/// permanent rather than "not yet" — the distinction a user acts on.
#[test]
fn docs_site_states_the_trino_reason_and_its_permanence() {
    let text = fs::read_to_string(docs_site_dir().join("reference/state.md")).unwrap();

    assert!(
        text.contains("autocommit"),
        "docs-site/docs/reference/state.md does not state that Iceberg writes are autocommit-only"
    );
    assert!(
        text.contains("Catalog only supports writes using autocommit: iceberg"),
        "docs-site/docs/reference/state.md does not quote the connector's verbatim autocommit \
         refusal"
    );
    assert!(
        text.contains("permanent"),
        "docs-site/docs/reference/state.md does not state that Trino's absence is permanent, \
         not \"not yet\""
    );
}

/// The page must name what the absence costs (the recompute-family downgrade,
/// `MaintenanceStateDowngraded`, `smelt explain`) and the one declaration that
/// refuses instead of downgrading (`DeclaredContractRequiresState`).
#[test]
fn docs_site_states_what_the_absence_costs_and_what_replaces_it() {
    let text = fs::read_to_string(docs_site_dir().join("reference/state.md")).unwrap();

    for needle in [
        "recompute",
        "MaintenanceStateDowngraded",
        "smelt explain",
        "DeclaredContractRequiresState",
    ] {
        assert!(
            text.contains(needle),
            "docs-site/docs/reference/state.md does not mention `{needle}`"
        );
    }
}

/// The new section must cover Spark (Delta) alongside Trino (Iceberg) rather
/// than reading as a Trino-only special case — guards against the out-of-scope
/// item "tightening Spark's column is not this outcome's business" leaving the
/// user docs looking narrower than the spec table.
#[test]
fn spark_column_is_not_silently_narrower_than_trinos() {
    let text = fs::read_to_string(docs_site_dir().join("reference/state.md")).unwrap();

    assert!(
        text.contains("Spark") && text.contains("Delta"),
        "docs-site/docs/reference/state.md's new state-residency section does not cover \
         Spark (Delta) alongside Trino (Iceberg)"
    );
}

/// `docs/specs/diagnostics.md` documents `MaintenanceStateDowngraded` (Warning)
/// and `DeclaredContractRequiresState` (Error); the docs-site diagnostics
/// reference must carry a row for each with a matching severity, parsed from
/// the spec rather than restated.
#[test]
fn docs_site_diagnostics_page_lists_the_state_codes() {
    let spec = fs::read_to_string(repo_root().join("docs/specs/diagnostics.md")).unwrap();

    let severity_of = |code: &str| -> String {
        let marker = format!("| `{code}` |");
        let line_start = spec
            .find(&marker)
            .unwrap_or_else(|| panic!("docs/specs/diagnostics.md has no `{code}` row"));
        let line_end = spec[line_start..]
            .find('\n')
            .map(|i| line_start + i)
            .unwrap_or(spec.len());
        let line = &spec[line_start..line_end];
        let cols: Vec<&str> = line.split('|').map(str::trim).collect();
        cols[2].to_string()
    };

    let doc = fs::read_to_string(docs_site_dir().join("reference/diagnostics.md")).unwrap();

    for code in [
        "MaintenanceStateDowngraded",
        "DeclaredContractRequiresState",
    ] {
        let severity = severity_of(code);
        let row_marker = format!("`{code}`");
        assert!(
            doc.contains(&row_marker),
            "docs-site/docs/reference/diagnostics.md has no row for `{code}`"
        );
        let row_start = doc.find(&row_marker).unwrap();
        let row_end = doc[row_start..]
            .find('\n')
            .map(|i| row_start + i)
            .unwrap_or(doc.len());
        let row = &doc[row_start..row_end];
        assert!(
            row.contains(&severity),
            "docs-site/docs/reference/diagnostics.md's `{code}` row does not state severity \
             `{severity}` (from docs/specs/diagnostics.md)"
        );
    }
}

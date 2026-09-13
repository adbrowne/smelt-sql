//! Standing drift gate for the `trino` target spec
//! (`docs/outcomes/20260913-trino-target-spine/outcome.md` phase 1): the spec
//! is written before any Trino code exists, and these checks keep the spec's
//! own claims honest as later phases land real code.

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

/// `docs/specs/smelt_yml.md` §"Target shape" must name `port`, `user`, `tls`
/// and `password` as Trino keys, and the refusal paragraph must name all nine
/// keys a `trino` target refuses.
#[test]
fn trino_target_shape_is_specified() {
    let text = read_spec("smelt_yml.md");

    for key in ["`port`", "`user`", "`tls`", "`password`"] {
        assert!(
            text.contains(key) && text.contains("Trino"),
            "docs/specs/smelt_yml.md §\"Target shape\" does not document {key} as a Trino key"
        );
    }

    let refusal = text
        .find("A `trino` target hard-errors")
        .expect("docs/specs/smelt_yml.md has no `trino` refusal paragraph");
    let refusal_text = &text[refusal..refusal + 700.min(text.len() - refusal)];

    for key in [
        "connect_url",
        "warehouse",
        "format",
        "database",
        "settings",
        "project",
        "dataset",
        "location",
        "token",
    ] {
        assert!(
            refusal_text.contains(key),
            "the `trino` refusal paragraph does not name `{key}`"
        );
    }
}

/// `docs/specs/multi_backend.md` §Surface capability matrix must have a Trino
/// column, and every Trino cell in it must read `?` — a value would be an
/// unmeasured claim (the column is measured by execution in phase 8).
#[test]
fn trino_capability_column_exists_and_is_unmeasured() {
    let text = read_spec("multi_backend.md");

    let header_start = text
        .find("| Flag |")
        .expect("multi_backend.md has no capability matrix header row");
    let header_end = text[header_start..]
        .find('\n')
        .map(|i| header_start + i)
        .unwrap();
    let header = &text[header_start..header_end];
    assert!(
        header.contains("Trino"),
        "capability matrix header has no Trino column: {header}"
    );

    let table_end = text[header_end..]
        .find("\n\n")
        .map(|i| header_end + i)
        .unwrap_or(text.len());
    let table = &text[header_start..table_end];

    let mut non_question_rows = Vec::new();
    for line in table.lines().skip(2) {
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        // cells[0] is empty (leading `|`), cells[1] is the flag name, last Trino
        // column is the second-to-last non-empty cell.
        if let Some(trino_cell) = cells.iter().rev().find(|c| !c.is_empty()) {
            if *trino_cell != "?" {
                non_question_rows.push(line.to_string());
            }
        }
    }

    assert!(
        non_question_rows.is_empty(),
        "capability matrix has Trino cells that are not `?` (unmeasured): {non_question_rows:?}"
    );
}

/// §Known Divergences must record the implicit-`Native` emission hole and
/// name the sibling outcome that owns closing it.
#[test]
fn trino_native_emission_hole_is_recorded() {
    let text = read_spec("multi_backend.md");

    assert!(
        text.contains("emission_at") && text.contains("Native"),
        "§Known Divergences does not name `emission_at`/`Native` for Trino"
    );
    assert!(
        text.contains("20260913-trino-emission"),
        "§Known Divergences does not name the `20260913-trino-emission` outcome as owner"
    );
}

/// §"Connection security" must state that a `trino` target's `password` is
/// carried outside the connect-string mechanism and that a literal value is a
/// hard configuration error.
#[test]
fn trino_connection_security_rule_is_stated() {
    let text = read_spec("multi_backend.md");

    let section_start = text
        .find("### Connection security")
        .expect("multi_backend.md has no §\"Connection security\" section");
    let next_section = text[section_start + 1..]
        .find("\n### ")
        .map(|i| section_start + 1 + i)
        .unwrap_or(text.len());
    let section = &text[section_start..next_section];

    assert!(
        section.contains("trino") && section.contains("`password`"),
        "§\"Connection security\" does not name a `trino` target's `password` key"
    );
    assert!(
        section.contains("hard configuration error"),
        "§\"Connection security\" does not state the literal-`password`-is-a-hard-error rule for Trino"
    );
}

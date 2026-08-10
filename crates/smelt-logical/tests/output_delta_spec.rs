//! Standing gate for `docs/specs/model_properties.md` §"Output-delta shape"
//! and `docs/specs/incremental_models.md` §"The graph layer"
//! (`docs/outcomes/20260809-output-delta-typing/outcome.md` phase 1): the
//! per-column-group output-delta lattice, its transfer-rule table, the
//! fail-closed default, the §Surface catalogue row, and the typed-edge /
//! narrowed-refusal wording all exist as specified before any code consumes
//! them.

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

/// Extracts the body of a `### <heading>` section: everything between its
/// heading and the next `##`/`###` heading.
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

/// Extracts a markdown table's rows (lines starting with `|` after the
/// header separator `|---|`).
fn table_rows(section: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut seen_separator = false;
    for line in section.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') {
            continue;
        }
        if trimmed.chars().all(|c| "|-: ".contains(c)) {
            seen_separator = true;
            continue;
        }
        if !seen_separator {
            continue;
        }
        let cells: Vec<String> = trimmed
            .trim_matches('|')
            .split('|')
            .map(|c| c.trim().to_string())
            .collect();
        rows.push(cells);
    }
    rows
}

#[test]
fn lattice_levels_are_the_three_named_shapes() {
    let model_properties = read("docs/specs/model_properties.md");
    let section = section_body(&model_properties, "### Output-delta shape");

    for name in ["AppendOnlyWindow", "KeyedUpsert", "General"] {
        assert!(
            section.contains(name),
            "§\"Output-delta shape\" must name lattice level `{name}`"
        );
    }

    let append_pos = section.find("AppendOnlyWindow").unwrap();
    let keyed_pos = section.find("KeyedUpsert").unwrap();
    let general_pos = section.find("General").unwrap();
    assert!(
        append_pos < keyed_pos && keyed_pos < general_pos,
        "the three lattice levels must be introduced in ascending order \
         AppendOnlyWindow ⊑ KeyedUpsert ⊑ General"
    );

    // No fourth level: only these three CamelCase verdict constructors should
    // appear as lattice members (a loose check — flags an obvious fourth
    // level like `Retracted` or `Snapshot` sneaking in unannounced).
    for suspicious in ["Retracted", "Snapshot", "Unversioned"] {
        assert!(
            !section.contains(suspicious),
            "unexpected extra lattice level `{suspicious}` in §\"Output-delta shape\""
        );
    }
}

#[test]
fn transfer_rule_table_is_well_formed() {
    let model_properties = read("docs/specs/model_properties.md");
    let section = section_body(&model_properties, "### Output-delta shape");
    let rows = table_rows(section);
    assert!(
        !rows.is_empty(),
        "§\"Output-delta shape\" must have a transfer-rule table"
    );

    let levels = ["AppendOnlyWindow", "KeyedUpsert", "General"];
    for row in &rows {
        assert!(
            row.len() >= 3,
            "transfer-rule row {row:?} must have an operator family, an input-shape \
             condition, and an output shape"
        );
        let output_cell = row.last().expect("row has at least 3 cells");
        assert!(
            levels.iter().any(|lvl| output_cell.contains(lvl)),
            "transfer-rule row {row:?} output shape must name one of {levels:?}"
        );
    }
}

#[test]
fn fail_closed_row_is_present() {
    let model_properties = read("docs/specs/model_properties.md");
    let section = section_body(&model_properties, "### Output-delta shape");
    assert!(
        section.contains("unregistered") && section.contains("General"),
        "§\"Output-delta shape\" must state that an unregistered operator yields \
         `General`, naming that operator"
    );
}

#[test]
fn surface_row_exists_for_output_delta() {
    let model_properties = read("docs/specs/model_properties.md");
    let section = section_body(&model_properties, "### Derived proofs");
    let rows = table_rows(section);
    let row = rows.iter().find(|row| {
        row.first()
            .is_some_and(|c| c.contains("Output-delta shape"))
    });
    let row = row.expect("§Surface → Derived proofs must have a row named \"Output-delta shape\"");
    let maturity = row.last().expect("row has a maturity cell");
    assert_eq!(
        maturity, "partial (derived; consumed by edge typing, not yet by dirt propagation)",
        "§Surface → Derived proofs \"Output-delta shape\" row must carry phase 3's maturity: \
         derived by the walk and consumed by edge typing, not yet by dirt propagation"
    );
}

#[test]
fn leaf_seeding_row_is_present() {
    let model_properties = read("docs/specs/model_properties.md");
    let section = section_body(&model_properties, "### Output-delta shape");
    let rows = table_rows(section);
    let leaf_row = rows
        .iter()
        .find(|row| row.first().is_some_and(|c| c.contains("Base relation")))
        .expect(
            "§\"Output-delta shape\" transfer-rule table must have a leaf row for a base \
             relation (source/table reference)",
        );
    let output_cell = leaf_row.last().expect("leaf row has an output-shape cell");
    for outcome in ["AppendOnlyWindow", "KeyedUpsert", "General"] {
        assert!(
            output_cell.contains(outcome),
            "the leaf row must name all three profile outcomes, missing `{outcome}` in \
             {output_cell:?}"
        );
    }

    assert!(
        section.contains("model reference") && section.contains("otherwise `General`"),
        "§\"Output-delta shape\" must state that a model-reference leaf takes the referenced \
         model's own derived verdict where available, otherwise `General`"
    );
}

#[test]
fn graph_layer_states_typed_edges_and_narrowed_refusal() {
    let incremental_models = read("docs/specs/incremental_models.md");
    let section = section_body(&incremental_models, "### The graph layer");

    assert!(
        section.contains("shape")
            && section.contains("addressing")
            && section.contains("column set"),
        "§\"The graph layer\" must describe the typed edge as a \
         (shape × addressing × column set) triple"
    );
    assert!(
        section.contains("MaintenanceGraphUnsupportedNode"),
        "§\"The graph layer\" must still name `MaintenanceGraphUnsupportedNode`"
    );
    assert!(
        section.contains("General"),
        "§\"The graph layer\" must scope the keyed-node refusal to a `General` verdict"
    );
    assert!(
        section.contains("KeyedUpsert") && section.contains("keyed dirt-set"),
        "§\"The graph layer\" must describe keyed dirt-set propagation for an admitted \
         `KeyedUpsert` verdict"
    );
}

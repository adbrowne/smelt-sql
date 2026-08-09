//! Standing gate for `docs/specs/model_properties.md` §"Probe obligation"
//! (`docs/outcomes/20260809-probe-backed-facts/outcome.md` phase 1): every
//! narrowing model-scoped declaration and every source-side narrowing
//! declaration this outcome's registry names must appear in the probe
//! registry table, either as a full row (probe, fires-when, diagnostic,
//! remedy, cadence all present) or as an explicit `exempt` row with a
//! reason. A `built` row must name a real emitter. A named diagnostic must
//! be catalogued in `docs/specs/diagnostics.md`.

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

/// Extracts the body of the `### Probe obligation` section: everything
/// between its heading and the next `###`/`##` heading.
fn probe_obligation_section(model_properties: &str) -> &str {
    let start = model_properties
        .find("### Probe obligation")
        .expect("model_properties.md must have a §\"Probe obligation\" heading");
    let after_heading = &model_properties[start..];
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

/// Extracts the registry table's rows (lines starting with `|` after the
/// header separator `|---|`).
fn registry_rows(section: &str) -> Vec<Vec<String>> {
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
            // header row
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

/// Declaration identifiers the registry must account for, one way or
/// another: every declaration in model_properties.md's own §"Model-scoped
/// declarations" table, plus the fixed set of sources.md narrowing
/// declarations this phase's plan names by row
/// (`docs/outcomes/20260809-probe-backed-facts/phases/01-plan.md`).
const EXPECTED_DECLARATIONS: &[&str] = &[
    "assert_monotonic",
    "nondeterministic_columns",
    "functional_dependencies",
    "bounded_domain",
    "horizon_ceiling",
    "referential_integrity",
    "key_recurrence",
    "unique_key",
];

#[test]
fn every_narrowing_declaration_has_a_probe_row() {
    let model_properties = read("docs/specs/model_properties.md");
    let section = probe_obligation_section(&model_properties);
    let rows = registry_rows(section);
    assert!(
        !rows.is_empty(),
        "the probe registry table must have at least one row"
    );

    for decl in EXPECTED_DECLARATIONS {
        let matching: Vec<&Vec<String>> = rows
            .iter()
            .filter(|row| row.first().is_some_and(|d| d.contains(decl)))
            .collect();
        assert!(
            !matching.is_empty(),
            "declaration `{decl}` has no row in the §\"Probe obligation\" registry table"
        );
        for row in matching {
            let is_exempt = row.get(1).is_some_and(|c| c.contains("exempt"));
            if is_exempt {
                let reason = row.get(5).map(String::as_str).unwrap_or("");
                assert!(
                    !reason.is_empty() && reason != "—",
                    "exempt declaration `{decl}` must state a reason in the Remedy cell"
                );
            } else {
                // Probe | What it queries | Fires when | Diagnostic | Remedy | Default cadence
                for (idx, name) in [
                    (1, "Probe"),
                    (2, "What it queries"),
                    (3, "Fires when"),
                    (4, "Diagnostic"),
                    (5, "Remedy"),
                    (6, "Default cadence"),
                ] {
                    let cell = row.get(idx).map(String::as_str).unwrap_or("");
                    assert!(
                        !cell.is_empty() && cell != "—",
                        "declaration `{decl}`'s registry row has an empty {name} cell"
                    );
                }
            }
        }
    }
}

#[test]
fn probe_registry_built_rows_name_a_real_emitter() {
    let model_properties = read("docs/specs/model_properties.md");
    let section = probe_obligation_section(&model_properties);
    let rows = registry_rows(section);
    let emit_source = read("crates/smelt-logical/src/maintenance/emit.rs");

    let mut checked_any = false;
    for row in &rows {
        let status = row.last().map(String::as_str).unwrap_or("");
        if !status.contains("built") {
            continue;
        }
        let probe_cell = row.get(1).map(String::as_str).unwrap_or("");
        // Extract every `emit_...` identifier named in the Probe cell.
        let mut found_one = false;
        for token in probe_cell.split(|c: char| !c.is_alphanumeric() && c != '_') {
            if token.starts_with("emit_") {
                found_one = true;
                let decl = format!("pub fn {token}(");
                assert!(
                    emit_source.contains(&decl),
                    "registry row (declaration `{}`) has Status `{}` and names `{}`, \
                     but no `{}` exists in crates/smelt-logical/src/maintenance/emit.rs",
                    row.first().cloned().unwrap_or_default(),
                    status,
                    token,
                    decl
                );
                checked_any = true;
            }
        }
        assert!(
            found_one,
            "registry row (declaration `{}`) has Status `{}` (built) but names no `emit_*` \
             function in its Probe cell",
            row.first().cloned().unwrap_or_default(),
            status
        );
    }
    assert!(
        checked_any,
        "expected at least one `built` registry row (e.g. `key_recurrence`) to verify against"
    );
}

/// Widens the emitter-existence check above to specifically pin the four
/// probes phase 2 added emitters for
/// (`docs/outcomes/20260809-probe-backed-facts/phases/02-plan.md` test 10):
/// each must name its exact expected emitter, not merely some `emit_*`
/// symbol. All four are now dispatched live (`built`) — the source
/// append-only posture probe's dispatch site was the last one wired; no
/// `built (unwired)` registry row remains.
#[test]
fn all_four_declared_fact_probe_rows_are_built() {
    let model_properties = read("docs/specs/model_properties.md");
    let section = probe_obligation_section(&model_properties);
    let rows = registry_rows(section);
    let emit_source = read("crates/smelt-logical/src/maintenance/emit.rs");

    let expected: &[(&str, &str, &str)] = &[
        (
            "timeseries.assert_monotonic",
            "emit_monotonicity_probe",
            "built",
        ),
        (
            "functional_dependencies:",
            "emit_functional_dependency_probe",
            "built",
        ),
        ("bounded_domain:", "emit_bounded_domain_probe", "built"),
        ("append_only", "emit_append_only_posture_probe", "built"),
    ];

    for (decl, emitter, expected_status) in expected {
        let row = rows
            .iter()
            .find(|row| row.first().is_some_and(|d| d.contains(decl)))
            .unwrap_or_else(|| panic!("no registry row found for declaration `{decl}`"));
        let status = row.last().map(String::as_str).unwrap_or("");
        assert_eq!(
            status, *expected_status,
            "declaration `{decl}` expected Status `{expected_status}`, found `{status}`"
        );
        let probe_cell = row.get(1).map(String::as_str).unwrap_or("");
        assert!(
            probe_cell.contains(emitter),
            "declaration `{decl}`'s Probe cell must name `{emitter}`, found `{probe_cell}`"
        );
        let decl_sig = format!("pub fn {emitter}(");
        assert!(
            emit_source.contains(&decl_sig),
            "expected `{decl_sig}` in crates/smelt-logical/src/maintenance/emit.rs for \
             declaration `{decl}`"
        );
    }

    assert!(
        rows.iter()
            .all(|row| row.last().map(String::as_str) != Some("built (unwired)")),
        "no `built (unwired)` registry row should remain now that the append-only posture \
         probe has a live dispatch site"
    );
}

/// Pins the `referential_integrity` row's Status transition `built
/// (unwired)` → `built` (`docs/outcomes/20260809-probe-backed-facts/
/// phases/03-plan.md` test 8) — the tripwire is now dispatched at its one
/// live consumer (`execute_delete_insert_with_delta_restriction`), so the
/// row's Status must read exactly `built`, not `built (unwired)`.
#[test]
fn referential_integrity_row_status_is_built() {
    let model_properties = read("docs/specs/model_properties.md");
    let section = probe_obligation_section(&model_properties);
    let rows = registry_rows(section);
    let emit_source = read("crates/smelt-logical/src/maintenance/emit.rs");

    let row = rows
        .iter()
        .find(|row| {
            row.first()
                .is_some_and(|d| d.contains("referential_integrity"))
        })
        .expect("no registry row found for declaration `referential_integrity`");
    let status = row.last().map(String::as_str).unwrap_or("");
    assert_eq!(
        status, "built",
        "the `referential_integrity` row's Status must be exactly `built`, found `{status}`"
    );
    let probe_cell = row.get(1).map(String::as_str).unwrap_or("");
    assert!(
        probe_cell.contains("emit_count_preservation_probe"),
        "the `referential_integrity` row's Probe cell must name \
         `emit_count_preservation_probe`, found `{probe_cell}`"
    );
    assert!(
        emit_source.contains("pub fn emit_count_preservation_probe_from_body("),
        "expected `emit_count_preservation_probe_from_body` in \
         crates/smelt-logical/src/maintenance/emit.rs"
    );
}

#[test]
fn probe_registry_diagnostics_are_catalogued() {
    let model_properties = read("docs/specs/model_properties.md");
    let diagnostics = read("docs/specs/diagnostics.md");
    let section = probe_obligation_section(&model_properties);
    let rows = registry_rows(section);

    let mut checked_any = false;
    for row in &rows {
        let diagnostic_cell = row.get(4).map(String::as_str).unwrap_or("");
        for token in diagnostic_cell.split(|c: char| !c.is_alphanumeric() && c != '_') {
            if token.is_empty() {
                continue;
            }
            let first = token.chars().next().unwrap();
            if first.is_uppercase() && token.len() > 3 {
                let backticked = format!("`{token}`");
                assert!(
                    diagnostics.contains(&backticked),
                    "registry row (declaration `{}`) names diagnostic `{}`, which does not \
                     appear in docs/specs/diagnostics.md's catalogue",
                    row.first().cloned().unwrap_or_default(),
                    token
                );
                checked_any = true;
            }
        }
    }
    assert!(
        checked_any,
        "expected at least one registry row to name a catalogued diagnostic"
    );
}

#[test]
fn probe_obligation_section_states_the_rule() {
    let model_properties = read("docs/specs/model_properties.md");
    assert!(
        model_properties.contains("### Probe obligation"),
        "model_properties.md must have a §\"Probe obligation\" heading"
    );
    let section = probe_obligation_section(&model_properties);

    assert!(
        section.contains("no probe, no declaration"),
        "§\"Probe obligation\" must state the admissibility sentence \
         (\"no probe, no declaration\")"
    );
    assert!(
        section
            .to_lowercase()
            .contains("never downgraded to a warning")
            || section.to_lowercase().contains("never a warning"),
        "§\"Probe obligation\" must state that a firing probe is never a warning"
    );
    assert!(
        section.to_lowercase().contains("never a silent continue"),
        "§\"Probe obligation\" must state that a firing probe is never a silent continue"
    );
}

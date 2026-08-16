//! Standing gate: `render_refusal`'s codes and `docs/specs/diagnostics.md`
//! cannot drift apart (phase 10 of
//! `docs/outcomes/20260816-scheduler-delta-signatures/outcome.md` found the
//! catalogue paragraph for `MaintenanceRepairKeysNotDiscoverable` /
//! `MaintenanceRepairSliceUnbounded` had gone stale precisely because
//! nothing pinned the two together).

use smelt_logical::maintenance::ledger::render_refusal;
use smelt_logical::maintenance::Refusal;
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

/// One instance of every [`Refusal`] variant. Exhaustive by construction —
/// adding a variant without a case here is a compile error.
fn all_refusals() -> Vec<Refusal> {
    let sample = Refusal::SkeletonColumnAdded {
        column: "c".to_string(),
    };
    match sample {
        Refusal::SkeletonColumnAdded { .. } => {}
        Refusal::ScanUnbounded { .. } => {}
        Refusal::NoAdmissibleTechnique { .. } => {}
        Refusal::ReachNotDerivable { .. } => {}
        Refusal::UnsupportedGrain { .. } => {}
        Refusal::LocalityNotEstablished { .. } => {}
        Refusal::RepairKeysNotDiscoverable { .. } => {}
        Refusal::RepairSliceUnbounded { .. } => {}
    }

    vec![
        Refusal::SkeletonColumnAdded {
            column: "c".to_string(),
        },
        Refusal::ScanUnbounded {
            source: "s".to_string(),
            why: "why".to_string(),
        },
        Refusal::NoAdmissibleTechnique {
            trigger: "t".to_string(),
            why: "why".to_string(),
        },
        Refusal::ReachNotDerivable {
            edge: "e".to_string(),
            why: "why".to_string(),
        },
        Refusal::UnsupportedGrain {
            grain: "key_per_partition".to_string(),
            tracking_plan: "docs/plans/x.md".to_string(),
        },
        Refusal::LocalityNotEstablished {
            message: "KeyedForbidsTimeseries: model 'm' declares a `timeseries:` block but \
                       key temporal locality could not be established — no route applies."
                .to_string(),
        },
        Refusal::RepairKeysNotDiscoverable {
            source: "s".to_string(),
            why: "why".to_string(),
        },
        Refusal::RepairSliceUnbounded {
            source: "s".to_string(),
            why: "why".to_string(),
        },
    ]
}

#[test]
fn every_render_refusal_code_has_a_catalogue_row() {
    let diagnostics = read("docs/specs/diagnostics.md");

    for refusal in all_refusals() {
        let summary = render_refusal(&refusal);
        let row_marker = format!("`{}`", summary.code);
        assert!(
            diagnostics.contains(&row_marker),
            "render_refusal produces code {:?} but docs/specs/diagnostics.md has no \
             catalogue row for it (searched for {row_marker:?})",
            summary.code
        );
    }
}

#[test]
fn repair_family_divergence_note_is_not_stale() {
    let diagnostics = read("docs/specs/diagnostics.md");

    let paragraph_start = diagnostics
        .find("`MaintenanceRepairKeysNotDiscoverable` and `MaintenanceRepairSliceUnbounded`")
        .expect(
            "docs/specs/diagnostics.md must have a paragraph discussing the repair-family \
             refusal codes' DiagnosticCode status",
        );
    let paragraph = &diagnostics[paragraph_start..];
    let paragraph_end = paragraph.find("\n\n").unwrap_or(paragraph.len());
    let paragraph = &paragraph[..paragraph_end];

    assert!(
        !paragraph.contains("no deriving proof, technique, or emitter"),
        "the repair family has a deriving proof, technique, and emitter since \
         docs/outcomes/20260809-repair-family/outcome.md landed — the divergence note must \
         not claim otherwise"
    );
    assert!(
        paragraph.contains("smelt explain"),
        "the divergence note must say these refusals are surfaced pre-execution by \
         `smelt explain`, not left unsurfaced"
    );
}

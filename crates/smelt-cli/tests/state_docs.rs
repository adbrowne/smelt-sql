//! Doc-sync gate for the state-residency doctrine (`docs/specs/state.md`):
//! verifies `docs-site/` actually documents what phases 2-9 of
//! `docs/outcomes/20260816-state-residency/outcome.md` shipped, rather than
//! trusting hand-authored prose to stay in sync with `StateMode` and the
//! engine-resident reconciliation ledger.

use std::fs;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = workspace_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

/// Every `StateMode` variant (`crates/smelt-core/src/config.rs`) must be
/// named in `docs-site/docs/reference/smelt-yml.md`'s `state` key
/// documentation, so a future variant added without docs fails this test
/// rather than silently shipping undocumented.
#[test]
fn every_state_mode_variant_is_documented() {
    let smelt_yml_doc = read("docs-site/docs/reference/smelt-yml.md");
    let state_section_start = smelt_yml_doc.find("## State Configuration").expect(
        "smelt-yml.md must have a \"## State Configuration\" section documenting `state.mode`",
    );
    let state_section = &smelt_yml_doc[state_section_start..];

    for variant in ["stateless", "intervals", "environments"] {
        assert!(
            state_section.contains(variant),
            "docs-site/docs/reference/smelt-yml.md's State Configuration section must name \
             the `{variant}` StateMode variant"
        );
    }
}

/// Neither `state.md`'s artifact inventory table nor `deployment.md`'s
/// `.smelt/` layout block may list `reconciliation.json` as a live
/// artifact -- it is engine-resident since phase 4 of the state-residency
/// outcome. A legacy-import mention in prose (the migrate-once-then-remove
/// behaviour) is allowed and matched explicitly below, not swept up by
/// accident.
#[test]
fn reconciliation_ledger_is_not_a_documented_smelt_dir_artifact() {
    let state_doc = read("docs-site/docs/reference/state.md");
    let inventory_start = state_doc
        .find("## Inventory")
        .expect("state.md must have an \"## Inventory\" section");
    let inventory_end = state_doc[inventory_start..]
        .find("\n## ")
        .map(|offset| inventory_start + offset)
        .unwrap_or(state_doc.len());
    let inventory_table = &state_doc[inventory_start..inventory_end];
    assert!(
        !inventory_table.contains("reconciliation.json"),
        "docs-site/docs/reference/state.md's Inventory table must not list \
         reconciliation.json as a live .smelt/ artifact -- it is engine-resident"
    );

    let deployment_doc = read("docs-site/docs/guide/deployment.md");
    let layout_start = deployment_doc
        .find("## `.smelt/` state layout")
        .expect("deployment.md must have a \"## `.smelt/` state layout\" section");
    let layout_fence_start = deployment_doc[layout_start..]
        .find("```")
        .map(|offset| layout_start + offset)
        .expect("`.smelt/` state layout section must contain a fenced code block");
    let layout_fence_end = deployment_doc[layout_fence_start + 3..]
        .find("```")
        .map(|offset| layout_fence_start + 3 + offset + 3)
        .expect("`.smelt/` state layout fenced code block must close");
    let layout_block = &deployment_doc[layout_fence_start..layout_fence_end];
    assert!(
        !layout_block.contains("reconciliation.json"),
        "docs-site/docs/guide/deployment.md's `.smelt/` layout code block must not list \
         reconciliation.json -- it lives in the warehouse, not `.smelt/`"
    );

    // The legacy-import behaviour is still allowed to be described in prose
    // -- assert it is present, so this stays a deliberate mention rather
    // than an accidental one this test would otherwise be blind to.
    assert!(
        state_doc.contains("reconciliation.json")
            && state_doc.contains("predates this residency move"),
        "state.md should still describe the one-time legacy reconciliation.json import"
    );
}

/// The reference page must name the engine-resident ledger/frontier tables
/// and the availability-downgrade behaviour, so the residency claim
/// `docs/specs/state.md` makes has a user-facing home.
#[test]
fn state_reference_documents_the_downgrade_and_engine_resident_ledger() {
    let state_doc = read("docs-site/docs/reference/state.md");
    for needle in [
        "_smelt_ledger",
        "_smelt_frontier",
        "MaintenanceStateDowngraded",
    ] {
        assert!(
            state_doc.contains(needle),
            "docs-site/docs/reference/state.md must mention `{needle}`"
        );
    }

    let explain_doc = read("docs-site/docs/reference/smelt-explain.md");
    assert!(
        explain_doc.contains("state downgrade:"),
        "docs-site/docs/reference/smelt-explain.md must document the `state downgrade:` line \
         smelt explain prints for a downgraded cell"
    );
}

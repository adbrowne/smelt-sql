use smelt_cli::build_maintenance_plan_report;
use std::path::Path;

/// Doc-sync guard (`docs/outcomes/20260904-delta-signature-front-door/
/// outcome.md` phase 1): `docs-site/docs/reference/cli.md`'s `smelt explain`
/// section must document the `emits:` headline and the `delta_signature`
/// JSON object, not just the pre-existing per-cell/per-edge surface.
#[test]
fn docs_reference_documents_the_headline() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let cli_md = std::fs::read_to_string(repo_root.join("docs-site/docs/reference/cli.md"))
        .expect("read docs-site/docs/reference/cli.md");

    let explain_section_start = cli_md
        .find("## smelt explain")
        .expect("cli.md has a `## smelt explain` section");
    let explain_section = &cli_md[explain_section_start..];

    assert!(
        explain_section.contains("emits:"),
        "expected the `emits:` headline documented in the `smelt explain` section"
    );
    assert!(
        explain_section.contains("delta_signature"),
        "expected the `delta_signature` JSON object documented in the `smelt explain` section"
    );
}

/// C1 (`docs/outcomes/20260905-property-diff/phases/05-plan.md` D7): the
/// text report's `technique:` line must come from the property PROFILE, not
/// the raw plan cell — proven here by constructing a profile whose
/// `cell_verdicts[0].technique` deliberately differs from the raw
/// `PlanCell::technique` it was built from, and asserting the report shows
/// the profile's technique.
#[test]
fn text_report_technique_matches_the_profile_technique() {
    use std::collections::BTreeSet;

    use smelt_cli::explain::RelationContractView;
    use smelt_db::queries::maintenance::MaintenancePlanResult;
    use smelt_logical::analysis::profile::PropertyProfile;
    use smelt_logical::analysis::source_bounds::BoundContext;
    use smelt_logical::maintenance::{
        ColumnGroup, Corner, MaintenancePlan, PartitionLocal, PlanCell, RowIdentity,
        RowIdentityVerdict, Technique, Trigger,
    };

    let cell = PlanCell {
        group: "{amount}".to_string(),
        trigger: Trigger::UpstreamMutation {
            source: "orders".to_string(),
        },
        corner: Corner::ColumnMerge,
        technique: Technique::KeyedFold,
        partition_local: PartitionLocal::Yes,
        scans: vec![],
        ledger_catch_up: false,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::Key(vec!["id".to_string()]),
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: Default::default(),
        key_scope: None,
        state_downgrade: None,
    };
    let result = MaintenancePlanResult {
        plan: MaintenancePlan {
            cells: vec![cell],
            refusals: vec![],
            key_locality: None,
        },
        column_groups: vec![ColumnGroup {
            columns: vec!["amount".to_string()],
            mutation_sensitivity: Default::default(),
            membership_sensitivity: BTreeSet::new(),
        }],
        degenerate: vec![],
        state_columns: vec![],
        execution_postures: None,
        is_snapshot_reconcile: None,
        comparability: vec![],
        succession_advisories: vec![],
        succession_recipe: None,
    };

    // Build the profile from a cell whose technique deliberately differs
    // (`DeleteInsert` vs the raw plan's `KeyedFold`) — this is what a stale
    // "renders the raw plan" implementation would fail to reflect.
    let mut downgraded_cell = result.plan.cells[0].clone();
    downgraded_cell.technique = Technique::DeleteInsert;
    let properties = smelt_logical::analysis::profile::PropertySet::derive(
        "technique_fixture",
        "SELECT 1 AS amount",
        &[],
        &BoundContext::default(),
    )
    .expect("PropertySet::derive");
    let contract_points: Vec<smelt_logical::contract::ContractPointView> =
        vec![smelt_logical::contract::effective_contract(None, "", &[]).into()];
    let profile = PropertyProfile::assemble(
        properties,
        std::slice::from_ref(&downgraded_cell),
        &contract_points,
        &[],
        &[],
    );

    let report = build_maintenance_plan_report(
        "technique_fixture",
        &result,
        &RelationContractView::from_facts(None, None),
        &[],
        &[],
        None,
        None,
        &[],
        &[],
        smelt_core::config::ProbeCadence::PerRun,
        &[],
        None,
        None,
        &profile,
        None,
    )
    .expect("build_maintenance_plan_report");

    assert!(
        report.contains("technique: DeleteInsert"),
        "the text report must render the profile's technique, not the raw \
         plan cell's KeyedFold: {report}"
    );
    assert!(
        !report.contains("technique: KeyedFold"),
        "the raw plan's technique must not leak into the text report: {report}"
    );
}

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Every `Succession*` code named in `docs/specs/diagnostics.md` §"Succession
/// grain" must appear on the docs-site diagnostics reference, and vice versa
/// — a two-sided check so the two lists never silently drift apart.
#[test]
fn docs_site_diagnostics_reference_lists_every_succession_code() {
    let spec = std::fs::read_to_string(repo_root().join("docs/specs/diagnostics.md"))
        .expect("read docs/specs/diagnostics.md");
    let section_start = spec
        .find("### Succession grain")
        .expect("diagnostics.md has a `### Succession grain` section");
    let section_end = spec[section_start..]
        .find("\n## ")
        .map(|i| section_start + i)
        .unwrap_or(spec.len());
    let section = &spec[section_start..section_end];

    let mut spec_codes: Vec<&str> = section
        .split('`')
        .filter(|s| s.starts_with("Succession"))
        .collect();
    spec_codes.sort();
    spec_codes.dedup();
    assert!(
        !spec_codes.is_empty(),
        "expected at least one `Succession*` code in diagnostics.md's Succession grain section"
    );

    let docs_site =
        std::fs::read_to_string(repo_root().join("docs-site/docs/reference/diagnostics.md"))
            .expect("read docs-site/docs/reference/diagnostics.md");

    for code in &spec_codes {
        assert!(
            docs_site.contains(code),
            "docs-site diagnostics reference is missing `{code}` (present in the spec's \
             Succession grain section)"
        );
    }

    // Two-sided: every `Succession*` name on the docs-site page must be one
    // of the spec's own codes (no stale/invented name on the docs-site side).
    for word in docs_site.split(['`', '\n', ' ']) {
        if word.starts_with("Succession")
            && word != "Succession"
            && word.chars().all(|c| c.is_ascii_alphanumeric())
        {
            assert!(
                spec_codes.contains(&word),
                "docs-site diagnostics reference names `{word}`, which is not one of the \
                 spec's Succession grain codes: {spec_codes:?}"
            );
        }
    }
}

/// `docs-site/mkdocs.yml`'s nav must list the succession guide page, and the
/// page itself must name the admitted SQL outline, the `QUALIFY NOT <flag>`
/// delete-filter spelling, both partitioning postures, and the tombstone
/// ledger.
#[test]
fn succession_guide_page_is_navigated_and_covers_the_grain() {
    let nav = std::fs::read_to_string(repo_root().join("docs-site/mkdocs.yml"))
        .expect("read docs-site/mkdocs.yml");
    assert!(
        nav.contains("guide/scd2-succession.md"),
        "expected mkdocs.yml nav to list guide/scd2-succession.md"
    );

    let page = std::fs::read_to_string(repo_root().join("docs-site/docs/guide/scd2-succession.md"))
        .expect("read docs-site/docs/guide/scd2-succession.md");

    assert!(
        page.contains("LEAD(t)/LAG(t)") || page.contains("LEAD(t)`/`LAG(t)"),
        "expected the admitted SQL outline naming LEAD(t)/LAG(t): {page}"
    );
    assert!(
        page.contains("QUALIFY NOT"),
        "expected the QUALIFY NOT <flag> delete-filter spelling: {page}"
    );
    assert!(
        page.contains("Arrival-partitioned") || page.contains("arrival-partitioned"),
        "expected the arrival-partitioned posture named: {page}"
    );
    assert!(
        page.contains("Event-time-partitioned") || page.contains("event-time-partitioned"),
        "expected the event-time-partitioned posture named: {page}"
    );
    assert!(
        page.to_lowercase().contains("tombstone ledger"),
        "expected the tombstone ledger named: {page}"
    );
}

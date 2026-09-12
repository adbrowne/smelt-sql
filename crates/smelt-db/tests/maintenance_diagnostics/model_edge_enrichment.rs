use super::*;

/// The dimension model: clockless, keyed on `repo_id`, feeding
/// `gold.events_enriched` as an enrichment join
/// (`docs/specs/incremental_models.md` §"Upstream model edges").
const REPO_DIM: &str = r#"---
materialization: table
refresh: incremental
unique_key: [repo_id]
---
SELECT
    repo_id,
    MAX(repo_name) AS current_repo_name
FROM smelt.sources.repo_naming
GROUP BY repo_id
"#;

const REPO_NAMING_SOURCE: &str = r#"
description: Repo naming events, clocked on created_at.
mutation_profile: append_only
timeseries:
  partition_column: created_at
  event_time_column: created_at
  granularity: day
columns:
  - { name: repo_id, type: INTEGER, nullable: false }
  - { name: repo_name, type: VARCHAR, nullable: false }
  - { name: created_at, type: TIMESTAMP, nullable: false }
"#;

const EVENTS_SOURCE: &str = r#"
description: Deduped events, append-only.
mutation_profile: append_only
columns:
  - { name: event_id, type: INTEGER, nullable: false }
  - { name: event_date, type: DATE, nullable: false }
  - { name: repo_id, type: INTEGER, nullable: false }
"#;

fn events_enriched_model(allow_full_scan: bool, project_repo_id: bool) -> String {
    let scan_bounds = if allow_full_scan {
        "maintenance:\n  scan_bounds:\n    per_source:\n      gold.repo_dim:\n        \
         allow_full_scan: true\n"
    } else {
        ""
    };
    let repo_id_col = if project_repo_id {
        "    e.repo_id,\n"
    } else {
        ""
    };
    format!(
        "---\n\
         timeseries:\n\
         \x20\x20event_time_column: event_date\n\
         \x20\x20partition_column: event_date\n\
         \x20\x20granularity: day\n\
         refresh: incremental\n\
         grain: partition\n\
         {scan_bounds}---\n\
         SELECT\n\
         \x20\x20\x20\x20e.event_id,\n\
         \x20\x20\x20\x20e.event_date,\n\
         {repo_id_col}\
         \x20\x20\x20\x20dim.current_repo_name\n\
         FROM smelt.sources.events e\n\
         LEFT JOIN smelt.gold.repo_dim dim ON e.repo_id = dim.repo_id\n"
    )
}

/// The enrichment-keyed route admits a `ColumnScopedMerge` cell when the
/// join key is projected and `allow_full_scan` is declared — no
/// `RepairKeysNotDiscoverable` refusal, via the SAME Salsa-wired
/// `derive_model_maintenance_plan_with_edges` path `smelt explain` uses
/// (`maintenance_plan_report` — see this file's `repair_keys_not_
/// discoverable_raises_a_diagnostic` for why this suite uses `plan_for`
/// rather than `diagnostics_for` for a model-edge fixture).
#[test]
fn enrichment_keyed_route_admits_no_diagnostic() {
    let model = events_enriched_model(true, true);
    let plan = plan_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/repo_naming.yml", REPO_NAMING_SOURCE),
            ("models/sources/events.yml", EVENTS_SOURCE),
            ("models/gold/repo_dim.sql", REPO_DIM),
            ("models/gold/events_enriched.sql", &model),
        ],
        "gold/events_enriched",
    );
    assert!(
        !plan.plan.refusals.iter().any(|r| matches!(
            r,
            smelt_logical::maintenance::Refusal::RepairKeysNotDiscoverable { .. }
        )),
        "expected no RepairKeysNotDiscoverable refusal, got {:?}",
        plan.plan.refusals
    );
    let cell = plan
        .plan
        .cells
        .iter()
        .find(|c| matches!(&c.trigger, smelt_logical::maintenance::Trigger::UpstreamMutation { source } if source == "gold.repo_dim"))
        .unwrap_or_else(|| panic!("expected an UpstreamMutation(gold.repo_dim) cell: {:?}", plan.plan.cells));
    assert_eq!(
        cell.technique,
        smelt_logical::maintenance::Technique::ColumnScopedMerge
    );
}

/// A model edge that declines every route (here: the join key is not
/// projected by the downstream, so the enrichment-keyed route cannot
/// address a merge write either) raises `RepairKeysNotDiscoverable`,
/// mapped to the `MaintenanceRepairKeysNotDiscoverable` `DiagnosticCode` by
/// `diagnostic_for_refusal` (asserted directly, matching `refusal_codes`'
/// own agreement test).
///
/// Uses `plan_for` (`maintenance_plan_report`, `smelt explain`'s own query)
/// rather than `diagnostics_for` (`file_diagnostics()`, the LSP/CLI
/// diagnostics query): `maintenance_plan_diagnostics`
/// (`crate::queries::maintenance::maintenance_plan_diagnostics`, the
/// function `file_diagnostics()` calls) is wired to
/// `derive_model_maintenance_plan` — the source-only, no-model-edges
/// variant — never `..._with_edges`, so it cannot see a model edge at all
/// today. This is a PRE-EXISTING gap (the same one `Refusal::
/// ReachNotDerivable`'s own doc comment already names: "Recorded in the
/// plan ... but not yet folded into `file_diagnostics()`"), not something
/// this phase introduces or is scoped to close — `RepairKeysNotDiscoverable`
/// now has a real `DiagnosticCode`, but only `smelt explain`/`maintenance_
/// plan_report` reaches it until a future phase threads model edges into
/// the LSP-facing query too.
#[test]
fn repair_keys_not_discoverable_raises_a_diagnostic() {
    let model = events_enriched_model(true, false);
    let plan = plan_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/repo_naming.yml", REPO_NAMING_SOURCE),
            ("models/sources/events.yml", EVENTS_SOURCE),
            ("models/gold/repo_dim.sql", REPO_DIM),
            ("models/gold/events_enriched.sql", &model),
        ],
        "gold/events_enriched",
    );
    let refusal = plan
        .plan
        .refusals
        .iter()
        .find(|r| matches!(r, smelt_logical::maintenance::Refusal::RepairKeysNotDiscoverable { source, .. } if source == "gold.repo_dim"))
        .unwrap_or_else(|| panic!("expected a RepairKeysNotDiscoverable refusal naming the edge: {:?}", plan.plan.refusals));
    let (severity, code, message) = smelt_db::queries::maintenance::diagnostic_for_refusal(
        &smelt_db::queries::maintenance::MaintenanceRefusal::RepairKeysNotDiscoverable {
            source: match refusal {
                smelt_logical::maintenance::Refusal::RepairKeysNotDiscoverable {
                    source, ..
                } => source.clone(),
                _ => unreachable!(),
            },
            why: match refusal {
                smelt_logical::maintenance::Refusal::RepairKeysNotDiscoverable { why, .. } => {
                    why.clone()
                }
                _ => unreachable!(),
            },
        },
    )
    .expect("diagnostic_for_refusal must map RepairKeysNotDiscoverable");
    assert_eq!(code, DiagnosticCode::MaintenanceRepairKeysNotDiscoverable);
    assert_eq!(
        severity,
        smelt_db::diagnostics_types::DiagnosticSeverity::Error
    );
    assert!(message.contains("gold.repo_dim"));
}

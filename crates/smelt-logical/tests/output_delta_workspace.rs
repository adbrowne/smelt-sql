//! Cross-model output-delta fold over a real (small, in-memory) workspace
//! (`docs/outcomes/20260809-output-delta-typing/phases/04-plan.md`): a
//! consuming model's walk resolves an upstream **model's** own derived
//! output-delta verdict, folded in dependency order via
//! `derive_workspace_output_deltas`.

use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::output_delta::{
    derive_workspace_output_deltas, ModelDeltaInput, MutationProfile, OutputDelta, SourceFacts,
};

fn source(name: &str, profile: MutationProfile, axis: Option<&str>) -> SourceFacts {
    SourceFacts {
        name: name.to_string(),
        axis: axis.map(|a| a.to_string()),
        mutation_profile: Some(profile),
        delta_identity: None,
    }
}

fn model(address: &str, sql: &str, sources: Vec<SourceFacts>) -> ModelDeltaInput {
    ModelDeltaInput {
        address: address.to_string(),
        sql: sql.to_string(),
        ctx: JoinContext::new(),
        sources,
    }
}

fn shape_of<'a>(
    verdicts: &'a std::collections::BTreeMap<String, smelt_logical::OutputDeltaFacts>,
    model_addr: &str,
    column: &str,
) -> &'a OutputDelta {
    &verdicts
        .get(model_addr)
        .unwrap_or_else(|| panic!("no verdict for model '{model_addr}' in {verdicts:?}"))
        .columns
        .iter()
        .find(|(n, _)| n == column)
        .unwrap_or_else(|| panic!("no column '{column}' for model '{model_addr}'"))
        .1
}

#[test]
fn chain_folds_verdicts_in_dependency_order() {
    // source(append_only) -> a (GROUP BY key) -> b (projection)
    let events = source("events", MutationProfile::AppendOnly, Some("event_date"));
    let inputs = vec![
        model(
            "a",
            "SELECT user_id, COUNT(*) AS n FROM smelt.sources.events GROUP BY user_id",
            vec![events],
        ),
        model("b", "SELECT user_id, n FROM smelt.models.a", Vec::new()),
    ];
    let verdicts = derive_workspace_output_deltas(&inputs);
    assert_eq!(
        *shape_of(&verdicts, "a", "user_id"),
        OutputDelta::KeyedUpsert {
            keys: vec!["user_id".to_string()]
        }
    );
    assert_eq!(
        *shape_of(&verdicts, "b", "user_id"),
        OutputDelta::KeyedUpsert {
            keys: vec!["user_id".to_string()]
        },
        "b must fold a's derived KeyedUpsert shape, not fail closed to General"
    );
}

#[test]
fn passthrough_model_preserves_append_only_window() {
    // source(append_only) -> a (SELECT ... WHERE) -> b (pass-through)
    let events = source("events", MutationProfile::AppendOnly, Some("event_date"));
    let inputs = vec![
        model(
            "a",
            "SELECT id, amount FROM smelt.sources.events WHERE amount > 0",
            vec![events],
        ),
        model("b", "SELECT id, amount FROM smelt.models.a", Vec::new()),
    ];
    let verdicts = derive_workspace_output_deltas(&inputs);
    assert_eq!(
        *shape_of(&verdicts, "b", "amount"),
        OutputDelta::AppendOnlyWindow {
            axis: "event_date".to_string()
        },
        "b must stay AppendOnlyWindow on the same axis through a pass-through consumer"
    );
}

#[test]
fn unknown_or_cyclic_model_ref_is_general_and_terminates() {
    // A ref to a model not in the input set.
    let inputs = vec![model(
        "a",
        "SELECT id FROM smelt.models.does_not_exist",
        Vec::new(),
    )];
    let verdicts = derive_workspace_output_deltas(&inputs);
    match shape_of(&verdicts, "a", "id") {
        OutputDelta::General { .. } => {}
        other => panic!("expected General for an unknown model ref, got {other:?}"),
    }

    // A two-model cycle: must terminate (bounded passes), never hang, and
    // yield General for both members.
    let cyclic_inputs = vec![
        model("x", "SELECT id FROM smelt.models.y", Vec::new()),
        model("y", "SELECT id FROM smelt.models.x", Vec::new()),
    ];
    let verdicts = derive_workspace_output_deltas(&cyclic_inputs);
    match shape_of(&verdicts, "x", "id") {
        OutputDelta::General { .. } => {}
        other => panic!("expected General for a cyclic model ref, got {other:?}"),
    }
    match shape_of(&verdicts, "y", "id") {
        OutputDelta::General { .. } => {}
        other => panic!("expected General for a cyclic model ref, got {other:?}"),
    }
}

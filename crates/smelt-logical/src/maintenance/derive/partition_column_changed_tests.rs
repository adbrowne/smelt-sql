use super::*;

fn partition_inputs<'a>(
    old_partition_col: Option<&'a str>,
    partition_col: &str,
) -> ModelInputs<'a> {
    ModelInputs {
        sql: "SELECT 1",
        output: OutputSpec {
            table: "t".to_string(),
            grain: Grain::Partition {
                partition_col: partition_col.to_string(),
            },
            skeleton_columns: BTreeSet::new(),
        },
        sources: Vec::new(),
        column_groups: Vec::new(),
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col,
    }
}

/// A `Grain::Partition` output whose declared `partition_column` differs
/// from the deployed-schema snapshot's recorded address derives a
/// `Refusal::PartitionColumnChanged` naming both columns.
#[test]
fn partition_column_rename_derives_refusal() {
    let inputs = partition_inputs(Some("event_date"), "event_day");
    let plan = derive_maintenance_plan(&inputs, &[]);
    assert!(
        plan.refusals.iter().any(|r| matches!(
            r,
            Refusal::PartitionColumnChanged { from, to }
                if from == "event_date" && to == "event_day"
        )),
        "expected PartitionColumnChanged, got {:?}",
        plan.refusals
    );
}

/// A case-insensitively-equal recorded/current column derives no
/// refusal — a rename is a different address, not a different spelling.
#[test]
fn unchanged_partition_column_derives_no_refusal() {
    let inputs = partition_inputs(Some("Event_Date"), "event_date");
    let plan = derive_maintenance_plan(&inputs, &[]);
    assert!(
        !plan
            .refusals
            .iter()
            .any(|r| matches!(r, Refusal::PartitionColumnChanged { .. })),
        "expected no PartitionColumnChanged, got {:?}",
        plan.refusals
    );
}

/// No recorded address (no snapshot, or one written before this field
/// existed) derives no refusal — fail-closed, never a guessed rename.
#[test]
fn absent_old_partition_column_fails_closed() {
    let inputs = partition_inputs(None, "event_date");
    let plan = derive_maintenance_plan(&inputs, &[]);
    assert!(
        !plan
            .refusals
            .iter()
            .any(|r| matches!(r, Refusal::PartitionColumnChanged { .. })),
        "expected no PartitionColumnChanged, got {:?}",
        plan.refusals
    );
}

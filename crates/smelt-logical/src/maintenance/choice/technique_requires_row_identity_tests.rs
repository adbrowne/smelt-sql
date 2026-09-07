use super::*;

#[test]
fn delete_insert_needs_no_row_identity() {
    assert!(!technique_requires_row_identity(Technique::DeleteInsert));
}

#[test]
fn every_targeted_write_technique_needs_row_identity() {
    for t in [
        Technique::KeyedFold,
        Technique::ColumnScopedMerge,
        Technique::InPlaceUpdate,
    ] {
        assert!(
            technique_requires_row_identity(t),
            "{t:?} addresses rows individually and must require a proven row identity"
        );
    }
}

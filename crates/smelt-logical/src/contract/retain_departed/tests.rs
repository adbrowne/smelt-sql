use super::*;

fn cols(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn admitted_only_on_keyed_mutable_snapshot() {
    assert!(validate(GrainLabel::Key, true, None, &cols(&["id"]), "m").is_ok());

    let err = validate(GrainLabel::Partition, true, None, &cols(&["id"]), "m").unwrap_err();
    assert!(err.contains("partition"), "got: {err}");

    let err = validate(GrainLabel::Key, false, None, &cols(&["id"]), "m").unwrap_err();
    assert!(err.contains("mutable_snapshot"), "got: {err}");
}

#[test]
fn retain_departed_refused_on_a_succession_model() {
    let err = validate(GrainLabel::Succession, true, None, &cols(&["id"]), "m").unwrap_err();
    assert!(
        err.contains("succession"),
        "error must name the succession grain, not a Key fallback, got: {err}"
    );
}

#[test]
fn tombstone_column_must_exist_in_output() {
    let err = validate(
        GrainLabel::Key,
        true,
        Some("is_departed"),
        &cols(&["id"]),
        "m",
    )
    .unwrap_err();
    assert!(err.contains("is_departed"), "got: {err}");

    assert!(validate(
        GrainLabel::Key,
        true,
        Some("is_departed"),
        &cols(&["id", "is_departed"]),
        "m"
    )
    .is_ok());
}

#[test]
fn oracle_exempts_departed_keys() {
    assert_eq!(classify_key(true, false, false), KeyVerdict::Strict);
    assert_eq!(classify_key(true, true, false), KeyVerdict::Strict);
    assert_eq!(classify_key(false, false, false), KeyVerdict::Exempt);
    assert_eq!(classify_key(false, true, true), KeyVerdict::Exempt);
    assert_eq!(
        classify_key(false, true, false),
        KeyVerdict::UnmarkedDeparture
    );
}

#[test]
fn reconcile_disposition_ladder() {
    assert_eq!(reconcile_disposition(None), DepartedKeyDisposition::Delete);
    assert_eq!(
        reconcile_disposition(Some(&RetainDeparted::Bool(true))),
        DepartedKeyDisposition::Retain { tombstone: None }
    );
    assert_eq!(
        reconcile_disposition(Some(&RetainDeparted::Bool(false))),
        DepartedKeyDisposition::Delete
    );
    assert_eq!(
        reconcile_disposition(Some(&RetainDeparted::Tombstone {
            tombstone: "is_departed".to_string()
        })),
        DepartedKeyDisposition::Retain {
            tombstone: Some("is_departed".to_string())
        }
    );
}

#[test]
fn probe_emits_antijoin_over_stored_minus_current() {
    let stmt = emit_departed_key_probe("main.stored", "main.current", &["id"], None);
    assert!(
        stmt.sql.contains("LEFT JOIN main.current c ON s.id = c.id"),
        "{}",
        stmt.sql
    );
    assert!(stmt.sql.contains("WHERE c.id IS NULL"), "{}", stmt.sql);
    assert!(
        stmt.sql.contains("COUNT(*) AS retained_departed_count"),
        "{}",
        stmt.sql
    );
    assert!(!stmt.sql.contains("unmarked_departed_count"));

    let stmt2 =
        emit_departed_key_probe("main.stored", "main.current", &["id"], Some("is_departed"));
    assert!(
        stmt2.sql.contains("unmarked_departed_count"),
        "{}",
        stmt2.sql
    );
}

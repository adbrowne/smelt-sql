use super::*;

#[test]
fn single_column_key_resolves() {
    let key = vec!["user_id".to_string()];
    assert_eq!(enrichment_restrict_column(&key), Some("user_id"));
}

#[test]
fn composite_key_refuses() {
    let key = vec!["a".to_string(), "b".to_string()];
    assert_eq!(enrichment_restrict_column(&key), None);
}

#[test]
fn no_key_refuses() {
    assert_eq!(enrichment_restrict_column(&[]), None);
}

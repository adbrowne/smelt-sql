use super::*;

fn suppressed() -> WriteSuppression {
    WriteSuppression::Suppressed {
        compared_columns: vec!["event_count".to_string()],
    }
}

fn unconditional() -> WriteSuppression {
    WriteSuppression::Unconditional {
        why: "column(s) notes are not proven comparable".to_string(),
    }
}

fn pattern(name: &str) -> &'static super::super::WritePattern {
    super::super::lookup_write_pattern(name)
        .unwrap_or_else(|| panic!("registry entry '{name}' must exist"))
}

#[test]
fn merge_capable_backend_always_resolves_to_merge_never_staged_candidate() {
    // Even a fully-comparable group stays on MERGE when the backend
    // can run one — the staged-candidate mechanism is never a silent
    // substitute for a MERGE the backend could have executed.
    let resolved = resolve_keyed_write_mechanism(&suppressed(), true, None);
    assert_eq!(resolved, Ok(Some(KeyedWriteMechanism::Merge(suppressed()))));

    let resolved_unconditional = resolve_keyed_write_mechanism(&unconditional(), true, None);
    assert_eq!(
        resolved_unconditional,
        Ok(Some(KeyedWriteMechanism::Merge(unconditional())))
    );
}

#[test]
fn merge_less_backend_with_comparable_group_admits_staged_candidate() {
    let resolved = resolve_keyed_write_mechanism(&suppressed(), false, None);
    assert_eq!(
        resolved,
        Ok(Some(KeyedWriteMechanism::StagedCandidate {
            compared_columns: vec!["event_count".to_string()]
        }))
    );
}

#[test]
fn merge_less_backend_with_no_admissible_suppression_resolves_to_none() {
    // Fail-closed: no merge-less unconditional keyed-fold mechanism
    // exists in this catalogue — the caller must fall back further
    // (e.g. region recompute), never invent a substitute here.
    let resolved = resolve_keyed_write_mechanism(&unconditional(), false, None);
    assert_eq!(resolved, Ok(None));
}

#[test]
fn an_unpinned_cell_resolves_exactly_as_before() {
    assert_eq!(
        resolve_keyed_write_mechanism(&suppressed(), true, None),
        Ok(Some(KeyedWriteMechanism::Merge(suppressed())))
    );
    assert_eq!(
        resolve_keyed_write_mechanism(&unconditional(), true, None),
        Ok(Some(KeyedWriteMechanism::Merge(unconditional())))
    );
    assert_eq!(
        resolve_keyed_write_mechanism(&suppressed(), false, None),
        Ok(Some(KeyedWriteMechanism::StagedCandidate {
            compared_columns: vec!["event_count".to_string()]
        }))
    );
    assert_eq!(
        resolve_keyed_write_mechanism(&unconditional(), false, None),
        Ok(None)
    );
}

#[test]
fn staged_candidate_pin_selects_staged_mechanism_on_a_merge_capable_backend() {
    let resolved =
        resolve_keyed_write_mechanism(&suppressed(), true, Some(pattern("staged_candidate")));
    assert_eq!(
        resolved,
        Ok(Some(KeyedWriteMechanism::StagedCandidate {
            compared_columns: vec!["event_count".to_string()]
        }))
    );
}

#[test]
fn staged_candidate_pin_over_an_unconditional_verdict_refuses() {
    for backend_supports_merge in [true, false] {
        let resolved = resolve_keyed_write_mechanism(
            &unconditional(),
            backend_supports_merge,
            Some(pattern("staged_candidate")),
        );
        match resolved {
            Err(ChoiceRefusal { pinned, .. }) => {
                assert_eq!(pinned, PinnedRequest::Write("staged_candidate".to_string()));
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
}

#[test]
fn keyed_conditional_pin_selects_merge_and_refuses_on_a_merge_less_backend() {
    let refused =
        resolve_keyed_write_mechanism(&suppressed(), false, Some(pattern("keyed_conditional")));
    match refused {
        Err(ChoiceRefusal { pinned, .. }) => {
            assert_eq!(
                pinned,
                PinnedRequest::Write("keyed_conditional".to_string())
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }

    let admitted =
        resolve_keyed_write_mechanism(&suppressed(), true, Some(pattern("keyed_conditional")));
    assert_eq!(admitted, Ok(Some(KeyedWriteMechanism::Merge(suppressed()))));
}

#[test]
fn a_pin_outside_the_keyed_fold_family_leaves_the_default_selection() {
    let resolved = resolve_keyed_write_mechanism(&suppressed(), true, Some(pattern("region")));
    assert_eq!(resolved, Ok(Some(KeyedWriteMechanism::Merge(suppressed()))));
}

-- Phase 28 fixture: verify that PASSING clauses parse correctly after a
-- smelt.fn.* call and do not produce diagnostics. The PASSING clause is
-- parsed into PASSING_CLAUSE CST nodes attached to the SMELT_FN_CALL; the
-- type checker (Phase 29) will process them. For now they are silently
-- ignored by the type checker because only the ARG_LIST children are
-- examined for argument binding.
SELECT
    smelt.fn.safe_divide(user_id, event_id) PASSING ratio_label AS (event_type) AS safe_ratio,
    event_type
FROM smelt.source('source.events')

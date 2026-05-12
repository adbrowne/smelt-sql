-- Wide-reflection worked example: union all models tagged `cohort`.
--
-- smelt.models.with_tag('cohort') returns List<ModelRef> — the three cohort_a,
-- cohort_b, cohort_c models in path-sorted order.  Because ModelRef <: TableExpr,
-- reduce(union_all) lifts the list to a UNION ALL query at expansion time.
SELECT map(smelt.models.with_tag('cohort'), fn m => m.name)

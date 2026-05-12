-- Wide-reflection worked example: collect names of all models tagged `cohort`.
--
-- smelt.models.with_tag('cohort') returns List<ModelRef> — the three cohort_a,
-- cohort_b, cohort_c models in path-sorted order.
-- map projects each ModelRef to its name field (a Text value).
SELECT map(smelt.models.with_tag('cohort'), fn m => m.name)

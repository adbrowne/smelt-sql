-- Intentional error: `materialization` is not in the closed ModelRef field set
-- {path, name, tags, columns}. Accessing `m.materialization` emits
-- ModelRefFieldUnknown at the `materialization` field token.
SELECT map(smelt.models.with_tag('cohort'), fn m => m.materialization)

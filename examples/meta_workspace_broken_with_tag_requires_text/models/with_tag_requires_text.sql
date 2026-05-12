-- Intentional error: with_tag argument must be a compile-time Text literal.
-- 42 is an integer, not a Text value.
-- Emits: WithTagRequiresText at the `42` argument span.
SELECT map(smelt.models.with_tag(42), fn m => m.name)

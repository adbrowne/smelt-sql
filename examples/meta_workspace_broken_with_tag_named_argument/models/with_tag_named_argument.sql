-- Intentional error: with_tag takes one positional argument; named arguments
-- are not supported. `tag => 'cohort'` is a named argument.
-- Emits: WithTagNamedArgument at the named-argument span.
SELECT map(smelt.models.with_tag(tag => 'cohort'), fn m => m.name)

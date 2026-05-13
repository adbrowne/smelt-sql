-- Intentional error: smelt.models.all() takes no arguments.
-- Passing 42 emits WideReflectionUnexpectedArgument at the `42` argument span.
SELECT map(smelt.models.all(42), fn m => m.name)

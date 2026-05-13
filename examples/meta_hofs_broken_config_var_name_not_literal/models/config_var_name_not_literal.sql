-- Intentional error: smelt.config.var requires a string literal as its argument.
-- Passing a column reference (or any non-literal expression) is not allowed in Phase B.
-- Emits: ConfigVarNameNotLiteral
SELECT smelt.config.var(some_column)

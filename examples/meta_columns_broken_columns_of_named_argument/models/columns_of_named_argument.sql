-- Intentional error: smelt.columns_of takes one positional argument.
-- Using a named argument (t => orders) is not supported.
-- Emits: ColumnsOfNamedArgument
SELECT smelt.columns_of(t => orders)

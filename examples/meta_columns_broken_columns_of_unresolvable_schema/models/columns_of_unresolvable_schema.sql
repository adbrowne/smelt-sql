-- Intentional error: smelt.columns_of(nonexistent) references a model that
-- does not exist in this workspace. At expansion time the schema cannot be
-- resolved, so the column list materialisation fails.
-- Emits: ColumnsOfUnresolvableSchema
SELECT smelt.columns_of(nonexistent)

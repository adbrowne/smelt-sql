-- Intentional error: smelt.columns_of expects a TableExpr argument; passing a
-- numeric literal (42) is clearly not a TableExpr.
-- Emits: ColumnsOfRequiresTableExpr
SELECT smelt.columns_of(42)

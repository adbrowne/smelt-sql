-- Phase 21 fixture: calls `filter_source` with a fragment argument that
-- references a column (`nonexistent`) not present in the caller-supplied
-- source schema. The call-site checker emits `FragmentColumnMissing`
-- anchored at the bad column reference inside the argument expression.
--
-- The companion `fn_fragment_col_missing_other.sql` supplies the upstream
-- model `fragment_col_source` with columns {id, amount}.

smelt.define filter_source(
    source: TableExpr,
    filters: Expr<Boolean>
) -> TableExpr AS (
    SELECT * FROM source WHERE filters
)

SELECT *
FROM smelt.filter_source(
    source => smelt.fn_fragment_col_missing_other,
    filters => nonexistent > 0
) AS t

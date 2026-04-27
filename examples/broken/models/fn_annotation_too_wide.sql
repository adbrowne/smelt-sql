-- Phase 21 fixture: `having_filter` declares `pred: Expr<Boolean, source>`
-- but the body splices `pred` only in HAVING (projected columns = {id}),
-- while `source` from the companion has {id, amount, name}. The annotation
-- is wider than the inferred splice context → `AnnotationTooWide`.
--
-- Companion `fn_annotation_too_wide_other.sql` supplies `annot_source`
-- with columns {id, amount, name}.

smelt.define having_filter(
    source: TableExpr,
    pred: Expr<Boolean, source>
) -> TableExpr AS (
    SELECT id FROM source GROUP BY id HAVING pred
)

SELECT *
FROM smelt.fn.having_filter(
    source => smelt.ref('fn_annotation_too_wide_other'),
    pred => id > 0
) AS t

-- Phase 23 broken fixture: Tier 2 body type error at definition time.
-- `revenue` is Expr<Integer> but `'text'` is Text — TypeMismatch.
smelt.define tier2_body_broken(
    revenue: Expr<Integer>
) AS (
    revenue + 'text'
)

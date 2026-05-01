-- B8 regression: qualified column refs over a bare upstream MODEL table.
--
-- After `smelt.ref('stg_orders')` is resolved by the dialect printer, the
-- FROM clause becomes `FROM main.stg_orders AS o`. The alias `o` is bound
-- to a bare table name — NOT to a smelt.ref() AST node. The TypeContext
-- builder previously only registered aliases for smelt.ref()/smelt.source()
-- nodes, so `o.line_revenue` resolved to Unknown and `SUM(...)` defaulted
-- to BIGINT, silently truncating the financial total.
--
-- `line_revenue` is intentionally a derived column in stg_orders (NOT in
-- sources.yml), so the unqualified-suffix-match fallback that masks the
-- bug for source columns does not apply.
SELECT
    SUM(o.line_revenue) AS qualified_line_revenue,
    SUM(o.gross_amount) AS qualified_gross_revenue,
    -- B9 regression: CASE WHEN ... THEN <DOUBLE upstream col> ELSE 0.0 END
    -- previously narrowed to DECIMAL(2,1) at runtime when iter-2 of the
    -- smelt-shop loop hit values > 9.9 (chain consequence of B8 — fixed
    -- now that o.line_revenue resolves to DOUBLE).
    SUM(CASE WHEN o.status_code = 'RT' THEN o.line_revenue ELSE 0.0 END)
        AS qualified_returned_revenue,
    SUM(COALESCE(o.line_revenue, 0.0)) AS qualified_coalesced_revenue
FROM smelt.models.stg_orders AS o


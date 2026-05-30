smelt.define cyc(source: TableExpr<{id: Numeric}>) -> TableExpr AS (
  WITH a AS (SELECT * FROM b), b AS (SELECT * FROM a)
  SELECT * FROM a
)

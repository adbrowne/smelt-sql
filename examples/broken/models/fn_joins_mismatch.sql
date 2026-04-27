---
joins:
  - table: dim_a
    on: src.id = dim_a.id
    cardinality: "1:1"
---
smelt.define fn_joins_mismatch(src: TableExpr) -> TableExpr AS (
  SELECT b.x FROM src LEFT JOIN dim_b AS dim_b ON src.id = dim_b.id
)

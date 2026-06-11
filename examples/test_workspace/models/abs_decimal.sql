--- name: abs_decimal
--- description: Verify ABS(Decimal) preserves precision and scale (Phase 5 fixture).
SELECT
    ABS(CAST(-1.23 AS DECIMAL(10,2))) AS abs_result
---

---
mateializaton: table
---
-- BUG-016 regression: `mateializaton` is a typo of `materialization` and is an
-- unknown top-level key. Must emit FrontmatterParseError, not silently default to VIEW.
SELECT 1 AS val

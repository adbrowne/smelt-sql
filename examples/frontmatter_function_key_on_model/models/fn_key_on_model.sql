---
materialization: table
deterministic: true
---
-- A model with a function-only key (`deterministic`) in its frontmatter.
-- The catalogue emits an inapplicable-key Warning but retains `materialization: table`.
-- This workspace must build successfully as a TABLE with one Warning.
SELECT 1 AS val

-- Intentional error: `union_all` has no identity element for an empty list.
-- `reduce([], union_all)` would produce nothing — union_all requires at least
-- one operand and has no identity (unlike and_all which has TRUE, or plus_chain
-- which has 0). Emits: ReducerEmptyNoIdentity
SELECT reduce([], union_all)

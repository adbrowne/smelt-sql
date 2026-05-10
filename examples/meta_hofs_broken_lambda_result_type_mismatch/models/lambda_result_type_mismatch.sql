-- Intentional error: filter requires a Boolean predicate, but `fn c => c`
-- returns Expr<SmallInt> (the element type), not Boolean.
-- Emits: LambdaResultTypeMismatch
SELECT filter([1, 2, 3], fn c => c)

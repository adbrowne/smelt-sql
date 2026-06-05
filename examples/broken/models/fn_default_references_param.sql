-- BUG-003 negative fixture: a default expression that references a sibling
-- parameter violates Semantics #9 ("a default expression must not reference
-- other parameters") and must emit DefaultReferencesParameter.
smelt.define f(a: Expr<Integer>, b: Expr<Integer> = a + 1) AS (a + b)

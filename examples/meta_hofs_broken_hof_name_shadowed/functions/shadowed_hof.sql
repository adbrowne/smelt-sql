-- Intentional error: `smelt.define map(...)` declares a function whose name
-- shadows the built-in HOF `map`. HOF names (map, filter, reduce) are reserved.
-- Emits: HofNameShadowed
smelt.define map(x: Expr<Integer>) -> Expr<Integer> AS (
    x + 1
)

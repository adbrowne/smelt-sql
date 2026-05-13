-- Intentional error: `smelt.define concat(...)` declares a function whose name
-- shadows the built-in reducer `concat`. Reducer names (comma_sep, and_all,
-- or_any, union_all, intersect_all, plus_chain, concat) are reserved.
-- Emits: ReducerNameShadowed
smelt.define concat(x: Expr<Text>) -> Expr<Text> AS (
    x
)

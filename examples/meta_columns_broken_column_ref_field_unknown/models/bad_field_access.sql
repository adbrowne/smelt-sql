-- Intentional error: the HOF lambda accesses c.invalid, which is not in the
-- closed ColumnRef field set {name, type, is_numeric}.
-- smelt.columns_of(orders) returns List<ColumnRef>; the lambda parameter `c`
-- is therefore ColumnRef-typed. Accessing `c.invalid` fires ColumnRefFieldUnknown
-- at the `invalid` field token.
SELECT map(smelt.columns_of(orders), fn c => c.invalid)
FROM orders

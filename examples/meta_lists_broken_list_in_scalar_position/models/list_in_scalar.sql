-- Broken: a bare meta List<T> in a Data-World SELECT-item (scalar) position.
-- `[1, 2, 3]` is a List<Integer>; left bare (not consumed by a spread, a
-- reducer, or a HOF) it cannot materialise as a scalar value, and there is no
-- implicit auto-spread. Expect exactly one MetaListInScalarPosition.
-- No FROM clause: the select-shape check must still run (it lives in the meta
-- walk, not the FROM-gated check_type_diagnostics).
SELECT [1, 2, 3]

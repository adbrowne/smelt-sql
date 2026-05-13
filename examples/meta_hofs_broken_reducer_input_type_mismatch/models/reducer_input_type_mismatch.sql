-- Intentional error: `and_all` expects List<Boolean>, but [1, 2, 3] is List<SmallInt>.
-- Emits: ReducerInputTypeMismatch
SELECT reduce([1, 2, 3], and_all)

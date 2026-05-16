-- Intentional error: concat_with requires exactly one argument (the separator),
-- but it is called with no arguments here.
-- Emits: ReducerArityMismatch
SELECT reduce(['alpha', 'beta'], concat_with())

-- Intentional error: lambda appears as a top-level SELECT item, not inside a HOF.
-- `fn c => c` is only valid as an argument to map, filter, or reduce.
-- Emits: LambdaInForbiddenPosition
SELECT fn c => c

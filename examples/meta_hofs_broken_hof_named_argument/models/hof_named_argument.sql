-- Intentional error: map/filter/reduce do not accept named arguments.
-- `map(list: [1, 2, 3], fn c => c * 2)` uses `list:` as a named arg.
-- Emits: HofNamedArgument
SELECT map(list => [1, 2, 3], fn c => c * 2)

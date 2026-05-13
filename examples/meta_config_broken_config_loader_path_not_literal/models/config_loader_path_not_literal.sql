-- Intentional error: loader path is a column reference (not a string literal).
-- smelt.config.load_yaml requires a string literal path; using a variable or
-- identifier emits ConfigLoaderPathNotLiteral at the argument expression.
SELECT smelt.config.load_yaml(some_variable, {name: Text, threshold: Integer})

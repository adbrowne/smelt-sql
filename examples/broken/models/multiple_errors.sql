-- Intentional: undefined ref and trailing syntax error
SELECT col1, col2,
FROM smelt.ref('does_not_exist')
WHERE

-- This model has an undefined reference - should show diagnostic
SELECT *
FROM {{ ref('nonexistent_model') }}

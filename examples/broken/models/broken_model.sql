-- This model has an undefined reference - should show diagnostic
SELECT *
FROM smelt.ref('nonexistent_model')

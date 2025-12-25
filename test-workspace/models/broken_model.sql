-- This model has an undefined reference - should show diagnostic
SELECT *
FROM sqt.ref('nonexistent_model')

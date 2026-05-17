-- Intentional error: integer 42 is not a Boolean, so the ternary condition
-- fails type checking.
-- Emits: TernaryConditionNotBoolean
SELECT if 42 then 1 else 2

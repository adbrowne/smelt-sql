-- Intentional error: the then-branch is an integer (1) and the else-branch is
-- a text literal ('oops'). The two branch types do not unify.
-- Emits: TernaryBranchTypeMismatch
SELECT if true then 1 else 'oops'

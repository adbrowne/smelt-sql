-- Two columns in VALUES but only one name in the alias list.
-- Emits: AliasColumnArityMismatch
SELECT a
FROM (VALUES (CAST(1 AS INTEGER), CAST(2 AS INTEGER))) AS t(a)

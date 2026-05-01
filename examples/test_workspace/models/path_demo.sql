-- Minimal fixture for the unified `smelt.<path>` value-form grammar
-- (smelt.<path> migration, Phase 1). The parser must accept this file with
-- zero parse diagnostics. Kind dispatch (resolving `smelt.models.users` to
-- the actual model) lands in Phase 2a — until then this file is parser-only
-- coverage.
SELECT * FROM smelt.models.users

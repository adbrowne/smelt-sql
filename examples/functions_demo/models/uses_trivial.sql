-- Phase 2 fixture: exercise the `smelt.fn.*` call surface. The call parses
-- but is not yet type-checked (that arrives in later phases), so the model
-- must remain diagnostic-clean under example_diagnostics.
SELECT smelt.fn.trivial(1) AS trivial_call
FROM smelt.source('source.events')

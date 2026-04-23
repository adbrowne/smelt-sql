-- Phase 10 fixture: a call site for the `regex_match` extern declared in
-- `../functions/externs.sql`. Both arguments are VARCHAR (normalises to
-- Text), matching the declared `Expr<Text>` parameter constraints. The
-- unified resolver dispatches user-declared externs through the same path
-- as `smelt.define` (minus the body re-walk), so this file must remain
-- diagnostic-clean.
SELECT smelt.fn.regex_match(event_type, 'prefix_.*') AS event_type_matches
FROM smelt.source('source.events')

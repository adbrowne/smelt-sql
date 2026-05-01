-- Phase 10: `smelt.extern` declares a function that exists in the target
-- backend(s) but has no smelt-level body. The parser accepts the signature
-- only (no `AS (...)`) and the workspace indexes it alongside `smelt.define`
-- functions; collision against the built-in registry is an error.
--
-- `regex_match` here stands in for DuckDB's `regexp_matches` (and Spark's
-- equivalent): a Boolean-returning predicate taking two Text arguments.
-- The workspace remains diagnostic-clean so long as no call sites pass a
-- wrong-typed argument.
smelt.extern regex_match(text: Expr<Text>, pattern: Expr<Text>) -> Expr<Boolean>


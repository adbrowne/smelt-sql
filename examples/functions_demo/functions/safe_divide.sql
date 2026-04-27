---
backends: [duckdb]
---
-- Phase 11: per-declaration frontmatter. The declared `backends: [duckdb]`
-- narrows the body's inferred set (`all` — body uses only generic SQL);
-- that is an accepted narrowing under the §16 #23 narrow-only rule.
-- Legacy single-block frontmatter on stand-alone models still works, but
-- function files now use the per-decl form so every smelt.define can
-- carry its own attributes independently.
smelt.define safe_divide(numerator: Expr<Numeric>, denominator: Expr<Numeric>) -> Expr<Double>
    AS (CASE WHEN denominator = 0 OR denominator IS NULL THEN NULL ELSE CAST(numerator AS DOUBLE) / CAST(denominator AS DOUBLE) END)

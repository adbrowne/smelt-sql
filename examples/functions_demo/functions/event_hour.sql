-- Phase 36: real body added. Extracts the hour from a Timestamp field
-- contained in a row-polymorphic struct parameter.
smelt.define event_hour(
    event: Expr<Struct<{ts: Timestamp, ..r}>>
) -> Expr<BigInt> AS (
    EXTRACT(HOUR FROM event.ts)
)

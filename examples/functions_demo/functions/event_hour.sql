-- Phase 35 fixture: declaration of event_hour using a row-polymorphic struct
-- parameter. The body receives a full check in Phase 36.
smelt.define event_hour(
    event: Expr<Struct<{ts: Timestamp, ..r}>>
) -> Expr<BigInt> AS (
    CAST(NULL AS BIGINT)
)

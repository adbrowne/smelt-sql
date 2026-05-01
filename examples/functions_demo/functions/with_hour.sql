-- Phase 37 fixture: row variable in return position.
-- At a call site with {ts: Timestamp, user_id: Integer}, the return
-- type resolves to Expr<Struct<{hour: BigInt, user_id: Integer}>>.
smelt.define with_hour(
    event: Expr<Struct<{ts: Timestamp, ..r}>>
) -> Expr<Struct<{hour: BigInt, ..r}>> AS (
    {EXTRACT(HOUR FROM event.ts) AS hour, ..event}
)


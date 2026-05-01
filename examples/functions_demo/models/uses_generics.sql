-- Phase 8 fixture: exercise the generic/variadic shapes seeded in the
-- canonical registry. `sources.yml` declares `source.events` with
-- `event_id INTEGER`, `user_id INTEGER`, `event_type VARCHAR`,
-- `ts TIMESTAMP`.
--
-- Each call below covers a different registry form:
--   * MIN(event_id)                          — `MIN<T: Ordered>(T) → T`
--   * COALESCE(user_id, 0)                   — `COALESCE<T: Any>(T...) → T`
--   * GREATEST(event_id, user_id, 0)         — `GREATEST<T: Ordered>(T...) → T`
--   * CONCAT(event_type, '-', event_type)    — `CONCAT(Text...) → Text`
--
-- Phase 8 is data-only: the registry is not yet wired into
-- `infer_function_type`. Today these calls flow through the existing
-- hand-written inference path, so the fixture must stay clean under the
-- legacy checker. Phase 9 rewires the checker to drive inference through
-- the registry directly.
--
-- Phase 27 fixture: exercise bidirectional widening via a Tier 3 function
-- that declares `-> Expr<Double>` over an `Expr<Integer>` body using ABS.
--   * smelt.fn.widen_to_double(event_id)     — Integer arg widened to Double
--                                               by expected_return propagation
--
-- Phase 50 fixture: newly-seeded registry built-ins.
--   * STDDEV(event_id)                       — aggregate: Numeric → Double
--   * NTILE(4) OVER (ORDER BY event_id)      — window: BigInt → BigInt
--   * LEFT(event_type, 3)                    — string scalar: Text → Text
--   * DATE_PART('year', ts)                  — temporal scalar: Text, Timestamp → Double
SELECT
    MIN(event_id) AS min_event_id,
    COALESCE(user_id, 0) AS user_id_or_zero,
    GREATEST(event_id, user_id, 0) AS max_numeric,
    CONCAT(event_type, '-', event_type) AS doubled_event_type,
    smelt.functions.widen_to_double(event_id) AS event_id_as_double,
    STDDEV(event_id) AS stddev_event_id,
    NTILE(4) OVER (ORDER BY event_id) AS event_quartile,
    LEFT(event_type, 3) AS event_type_prefix,
    DATE_PART('year', ts) AS event_year
FROM smelt.sources.source.events


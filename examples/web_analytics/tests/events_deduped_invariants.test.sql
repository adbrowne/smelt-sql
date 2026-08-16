-- `silver.events_deduped` invariants — the composed keyed+timeseries dedupe
-- stage (`docs/specs/incremental_shapes.md` §"Key temporal locality (the
-- time-partitioned output)"): exactly one row per `event_id`, regardless of
-- how many times a redelivered duplicate of that event appears in the raw
-- feed. This test mocks `sources.raw.events` directly (the model's sole
-- external dependency) with a redelivery fixture — `event_id: 1` arrives
-- twice, byte-identical except `arrival_time` (`datagen.yaml`'s
-- `redelivery:` block shape) — and asserts the model's own per-column `MIN`
-- extremal fold converges both copies to a single row.
smelt.test test_events_deduped_one_row_per_event_id AS (
    SELECT
        (SELECT COUNT(*) FROM smelt.silver.events_deduped) AS n_rows,
        (SELECT COUNT(DISTINCT event_id) FROM smelt.silver.events_deduped) AS n_distinct_event_ids
)
PASSING sources.raw.events AS (
    -- event_id 1: redelivered — the original and a duplicate arriving one
    -- day later, byte-identical except arrival_time.
    {event_id: 1, device_id: 10, user_id: NULL, seconds_in_day: 32400, event_time: '2026-04-01T09:00:00', arrival_time: '2026-04-01T09:00:00', utm_campaign: NULL, payload: '{"event_name": "page_view", "platform": "web", "url": "https://example.com/home"}', event_date: '2026-04-01'},
    {event_id: 1, device_id: 10, user_id: NULL, seconds_in_day: 32400, event_time: '2026-04-01T09:00:00', arrival_time: '2026-04-02T09:00:00', utm_campaign: NULL, payload: '{"event_name": "page_view", "platform": "web", "url": "https://example.com/home"}', event_date: '2026-04-01'},
    -- event_id 2: delivered once, signed-in, campaign-attributed.
    {event_id: 2, device_id: 11, user_id: 200, seconds_in_day: 36000, event_time: '2026-04-01T10:00:00', arrival_time: '2026-04-01T10:00:00', utm_campaign: 'spring_sale', payload: '{"event_name": "click", "platform": "ios", "url": "https://example.com/product"}', event_date: '2026-04-01'},
    -- event_id 3: delivered once, anonymous, on a different day.
    {event_id: 3, device_id: 12, user_id: NULL, seconds_in_day: 3600, event_time: '2026-04-02T01:00:00', arrival_time: '2026-04-02T01:00:00', utm_campaign: NULL, payload: '{"event_name": "scroll", "platform": "android", "url": "https://example.com/cart"}', event_date: '2026-04-02'}
)
EXPECT (
    {n_rows: 3, n_distinct_event_ids: 3}
)

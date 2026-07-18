-- Dual-session-identity invariants for `silver.events_enriched`
-- (`docs/research/20260711-clock-vs-root-anchored-sessions.md`
-- §"Enrichment — silver.events_enriched carries both"): every event row
-- must carry both the clock-anchored identity (`session_id`,
-- `session_utm_campaign`, from `silver.sessions`) and the root-anchored
-- identity (`session_id_chained`, `session_utm_campaign_chained`, from
-- `silver.sessions_chained`). This test mocks all three of
-- `events_enriched`'s external dependencies directly with pre-computed
-- session rows (it does not re-derive sessionization) and asserts the
-- join behavior: the ids agree when neither table's cap fired differently
-- for a device's events, and they diverge only where the two cut rules
-- actually disagree.
--
-- device 3 — agreement fixture: a single two-event session with no cap in
-- play at all (mirrors `session_boundary_invariants.test.sql` device 3 /
-- `session_boundary_chained_invariants.test.sql` device 3). Both
-- `silver.sessions` and `silver.sessions_chained` assign the identical
-- root (`session_start_ts = 2026-04-01 12:00:00`), so `session_id` and
-- `session_id_chained` (same `device_id-session_start_ts` identity scheme
-- in both tables) are textually equal for every event in this device's
-- session, and so are the two campaign columns.
--
-- device 8 — the cap-divergence fixture, mirroring
-- `session_boundary_chained_invariants.test.sql` device 8: the same
-- three-event stream (2026-04-10 23:55, 2026-04-11 00:10, 2026-04-11
-- 00:35 — all inside the 30-minute inactivity gap, so no gap/platform
-- boundary ever fires) is mocked as already sessionized by BOTH upstream
-- tables, reflecting what each table's own cut rule actually produces for
-- this device's history:
--   * `silver.sessions` (clock-anchored): rooted 2026-04-10 23:55 (>=
--     00:30, so the deadline reaches the *next* day's end) — one single
--     session covers all three events, since none of them cross that
--     deadline.
--   * `silver.sessions_chained` (root-anchored): an already-open session
--     rooted 2026-04-09 00:05 continues through the 23:55 event (51
--     events folded in, per the identical device-8 fixture in
--     `session_boundary_chained_invariants.test.sql`), but the
--     root-anchored 2-day cutoff forces a NEW root at the very next event
--     (2026-04-11 00:10), producing a second session distinct from the
--     first.
-- Every one of the three events therefore carries a DIFFERENT
-- `session_id_chained` than its `session_id` — the two tables' cut rules
-- disagreed on this device's history, and the divergence is directly
-- queryable on the enriched event grain.
smelt.test test_enrichment_dual_session_invariants AS (
    SELECT
        event_id,
        device_id,
        event_date,
        session_id,
        session_utm_campaign,
        session_id_chained,
        session_utm_campaign_chained
    FROM smelt.silver.events_enriched
)
PASSING silver.events_deduped AS (
    -- device 3: no boundary, no cap in play — both tables agree.
    {event_id: 1, device_id: 3, user_id: NULL, amplitude_id: NULL, event_ts: '2026-04-01 12:00:00', event_date: '2026-04-01', first_seen_date: '2026-04-01', event_name: 'view', platform: 'web', url: NULL, utm_campaign: 'spring_sale'},
    {event_id: 2, device_id: 3, user_id: NULL, amplitude_id: NULL, event_ts: '2026-04-01 12:05:00', event_date: '2026-04-01', first_seen_date: '2026-04-01', event_name: 'view', platform: 'web', url: NULL, utm_campaign: NULL},
    -- device 8: the cap-divergence fixture (see header comment).
    {event_id: 3, device_id: 8, user_id: NULL, amplitude_id: NULL, event_ts: '2026-04-10 23:55:00', event_date: '2026-04-10', first_seen_date: '2026-04-10', event_name: 'view', platform: 'web', url: NULL, utm_campaign: NULL},
    {event_id: 4, device_id: 8, user_id: NULL, amplitude_id: NULL, event_ts: '2026-04-11 00:10:00', event_date: '2026-04-11', first_seen_date: '2026-04-11', event_name: 'view', platform: 'web', url: NULL, utm_campaign: NULL},
    {event_id: 5, device_id: 8, user_id: NULL, amplitude_id: NULL, event_ts: '2026-04-11 00:35:00', event_date: '2026-04-11', first_seen_date: '2026-04-11', event_name: 'view', platform: 'web', url: NULL, utm_campaign: NULL}
)
PASSING silver.sessions AS (
    -- device 3: single session, no cap fired.
    {
        session_id: '3-2026-04-01T12:00:00',
        device_id: 3,
        session_start_ts: '2026-04-01 12:00:00',
        session_start_date: '2026-04-01',
        session_start: '2026-04-01 12:00:00',
        session_end: '2026-04-01 12:05:00',
        event_count: 2,
        platform: 'web',
        utm_campaign: 'spring_sale'
    },
    -- device 8: clock-anchored rule roots at 23:55 (>= 00:30, deadline
    -- reaches the next day's end) — a single session covers all three
    -- events (none of them cross that deadline).
    {
        session_id: '8-2026-04-10T23:55:00',
        device_id: 8,
        session_start_ts: '2026-04-10 23:55:00',
        session_start_date: '2026-04-10',
        session_start: '2026-04-10 23:55:00',
        session_end: '2026-04-11 00:35:00',
        event_count: 3,
        platform: 'web',
        utm_campaign: NULL
    }
)
PASSING silver.sessions_chained AS (
    -- device 3: identical root — the two tables agree exactly.
    {
        session_id: '3-2026-04-01T12:00:00',
        device_id: 3,
        session_start_ts: '2026-04-01 12:00:00',
        session_start_date: '2026-04-01',
        session_start: '2026-04-01 12:00:00',
        session_end: '2026-04-01 12:05:00',
        event_count: 2,
        platform: 'web',
        utm_campaign: 'spring_sale'
    },
    -- device 8, session A: the already-open session rooted 2026-04-09
    -- 00:05, extended through the 23:55 event (51 events folded in, same
    -- as `session_boundary_chained_invariants.test.sql` device 8).
    {
        session_id: '8-2026-04-09T00:05:00',
        device_id: 8,
        session_start_ts: '2026-04-09 00:05:00',
        session_start_date: '2026-04-09',
        session_start: '2026-04-09 00:05:00',
        session_end: '2026-04-10 23:55:00',
        event_count: 51,
        platform: 'web',
        utm_campaign: NULL
    },
    -- device 8, session B: the root-anchored 2-day cutoff forces a new
    -- root at 00:10, folding in the 00:10 and 00:35 events.
    {
        session_id: '8-2026-04-11T00:10:00',
        device_id: 8,
        session_start_ts: '2026-04-11 00:10:00',
        session_start_date: '2026-04-11',
        session_start: '2026-04-11 00:10:00',
        session_end: '2026-04-11 00:35:00',
        event_count: 2,
        platform: 'web',
        utm_campaign: NULL
    }
)
EXPECT (
    -- device 3: both identities agree on every event.
    {event_id: 1, device_id: 3, event_date: '2026-04-01', session_id: '3-2026-04-01T12:00:00', session_utm_campaign: 'spring_sale', session_id_chained: '3-2026-04-01T12:00:00', session_utm_campaign_chained: 'spring_sale'},
    {event_id: 2, device_id: 3, event_date: '2026-04-01', session_id: '3-2026-04-01T12:00:00', session_utm_campaign: 'spring_sale', session_id_chained: '3-2026-04-01T12:00:00', session_utm_campaign_chained: 'spring_sale'},
    -- device 8: every event's clock-anchored id differs from its
    -- root-anchored id — the divergence the header comment describes.
    {event_id: 3, device_id: 8, event_date: '2026-04-10', session_id: '8-2026-04-10T23:55:00', session_utm_campaign: NULL, session_id_chained: '8-2026-04-09T00:05:00', session_utm_campaign_chained: NULL},
    {event_id: 4, device_id: 8, event_date: '2026-04-11', session_id: '8-2026-04-10T23:55:00', session_utm_campaign: NULL, session_id_chained: '8-2026-04-11T00:10:00', session_utm_campaign_chained: NULL},
    {event_id: 5, device_id: 8, event_date: '2026-04-11', session_id: '8-2026-04-10T23:55:00', session_utm_campaign: NULL, session_id_chained: '8-2026-04-11T00:10:00', session_utm_campaign_chained: NULL}
)

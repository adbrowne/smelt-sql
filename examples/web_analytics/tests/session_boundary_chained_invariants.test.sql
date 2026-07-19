-- Session boundary invariants for the **root-anchored** cut
-- (`docs/research/20260711-clock-vs-root-anchored-sessions.md`
-- §"silver.sessions_chained — root-anchored cut"): the same 30-minute
-- inactivity + platform-change rules as `silver.sessions`, mirrored here
-- against the real `silver/sessions_chained` model — the assertion query
-- targets `smelt.silver.sessions_chained#aggregated` (`docs/specs/testing.md`
-- §"CTE isolation" — a full-query reference cannot distinguish the model
-- under test from the self-reference inside it, since both share the same
-- `smelt.<path>` text) and PASSING mocks its two external deps: the source
-- (silver.events_deduped) and its own prior output (silver.sessions_chained,
-- the self-reference).
--
-- Devices 1-4 mirror `session_boundary_invariants.test.sql` exactly (same
-- gap/platform rules, no self-continuation in play, so root-anchored and
-- clock-anchored agree):
--   device 1 — gap boundary: 35-minute gap triggers new session
--   device 2 — platform boundary: 5-minute gap + platform change triggers new session
--   device 3 — no boundary: 5-minute gap, same platform -> one session of 2 events
--   device 4 — cross-midnight continuation: 20-minute gap straddling midnight -> one session
--
-- device 8 — the divergence fixture: an already-open session (mocked via
-- PASSING silver.sessions_chained, rooted 2026-04-09 00:05, already
-- extended to 2026-04-10 23:50 with 50 prior events folded in) receives a
-- continuing event 5 minutes later (2026-04-10 23:55, well within the
-- 30-minute inactivity rule) — this DOES continue the open session (51
-- events). The NEXT event, 15 minutes after that (2026-04-11 00:10), is
-- ALSO within the 30-minute gap rule, but its own calendar date
-- (2026-04-11) is two days after the open session's root date
-- (2026-04-09) — the root-anchored cutoff forces a NEW root here even
-- though no gap/platform boundary fired. A clock-anchored table fed the
-- identical event stream would instead have cut this chain at a midnight
-- (whichever one the clock rule's own deadline lands on); this fixture
-- pins the root-anchored table's own, different, cut point.
smelt.test test_session_boundary_chained_invariants AS (
    SELECT device_id, session_start, event_count, platform
    FROM smelt.silver.sessions_chained#aggregated
)
PASSING silver.events_deduped AS (
    -- device 1: gap boundary (35 minutes) → 2 sessions
    {device_id: 1, event_ts: '2026-04-01 10:00:00', event_date: '2026-04-01', platform: 'web', utm_campaign: NULL},
    {device_id: 1, event_ts: '2026-04-01 10:35:00', event_date: '2026-04-01', platform: 'web', utm_campaign: NULL},
    -- device 2: platform boundary (5 min, platform change web→ios) → 2 sessions
    {device_id: 2, event_ts: '2026-04-01 11:00:00', event_date: '2026-04-01', platform: 'web', utm_campaign: NULL},
    {device_id: 2, event_ts: '2026-04-01 11:05:00', event_date: '2026-04-01', platform: 'ios', utm_campaign: NULL},
    -- device 3: no boundary (5 min, same platform) → 1 session of 2 events
    {device_id: 3, event_ts: '2026-04-01 12:00:00', event_date: '2026-04-01', platform: 'web', utm_campaign: NULL},
    {device_id: 3, event_ts: '2026-04-01 12:05:00', event_date: '2026-04-01', platform: 'web', utm_campaign: NULL},
    -- device 4: cross-midnight continuation (20 min, same platform) → 1 session
    {device_id: 4, event_ts: '2026-04-01 23:50:00', event_date: '2026-04-01', platform: 'web', utm_campaign: NULL},
    {device_id: 4, event_ts: '2026-04-02 00:10:00', event_date: '2026-04-02', platform: 'web', utm_campaign: NULL},
    -- device 8: the divergence fixture (see header comment).
    {device_id: 8, event_ts: '2026-04-10 23:55:00', event_date: '2026-04-10', platform: 'web', utm_campaign: NULL},
    {device_id: 8, event_ts: '2026-04-11 00:10:00', event_date: '2026-04-11', platform: 'web', utm_campaign: NULL},
    {device_id: 8, event_ts: '2026-04-11 00:35:00', event_date: '2026-04-11', platform: 'web', utm_campaign: NULL}
)
PASSING silver.sessions_chained AS (
    -- device 8's already-open session: rooted 2026-04-09 00:05, already
    -- extended (by earlier runs, not modeled here) through 2026-04-10
    -- 23:50 with 50 events folded in.
    {
        session_id: '8-2026-04-09T00:05:00',
        device_id: 8,
        session_start_ts: '2026-04-09 00:05:00',
        session_start_date: '2026-04-09',
        session_start: '2026-04-09 00:05:00',
        session_end: '2026-04-10 23:50:00',
        event_count: 50,
        platform: 'web',
        utm_campaign: NULL
    }
)
EXPECT (
    -- device 1: two sessions from the gap rule; each has 1 event
    {device_id: 1, session_start: '2026-04-01T10:00:00', event_count: 1, platform: 'web'},
    {device_id: 1, session_start: '2026-04-01T10:35:00', event_count: 1, platform: 'web'},
    -- device 2: two sessions from the platform-boundary rule
    {device_id: 2, session_start: '2026-04-01T11:00:00', event_count: 1, platform: 'web'},
    {device_id: 2, session_start: '2026-04-01T11:05:00', event_count: 1, platform: 'ios'},
    -- device 3: one session (no boundary triggered)
    {device_id: 3, session_start: '2026-04-01T12:00:00', event_count: 2, platform: 'web'},
    -- device 4: one cross-midnight session; both events merge into a single session
    {device_id: 4, session_start: '2026-04-01T23:50:00', event_count: 2, platform: 'web'},
    -- device 8: the open session continues through the 23:55 event (51
    -- events: 50 folded-in + 1 new), then the root-anchored 2-day cutoff
    -- forces a new root at 00:10 the next occurrence — even though the
    -- 15-minute gap alone would have continued the chain — merging the
    -- 00:10 and 00:35 events into that new session (2 events).
    {device_id: 8, session_start: '2026-04-09T00:05:00', event_count: 51, platform: 'web'},
    {device_id: 8, session_start: '2026-04-11T00:10:00', event_count: 2, platform: 'web'}
)

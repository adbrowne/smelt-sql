-- Phase 12 happy-path: three-frame expansion chain
-- `outer_call → middle → safe_divide`. Passing a Numeric column through
-- the chain leaves no call-site or body diagnostics. The paired
-- broken fixture `examples/broken/models/fn_nested_call_error.sql`
-- exercises the failure path (three-frame trailer + related-info).
SELECT CAST(smelt.fn.outer_call(event_id) AS INTEGER) AS threaded
FROM smelt.source('source.events')

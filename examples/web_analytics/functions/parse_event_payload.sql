-- Decode the raw JSON payload column from bronze.raw_events into a typed
-- struct. Bronze stores the payload as VARCHAR because smelt-datagen's
-- json_object generator emits Utf8; this function lifts that into named,
-- typed fields.
--
-- The current silver/events_parsed model inlines `json_extract_string`
-- directly rather than calling this function (smelt's struct-returning
-- function expansion in model contexts is not yet ergonomic — see the
-- per-phase plan's "Deferred during implementation"). This declaration is
-- kept here as the canonical signature for future callers and so the
-- diagnostics gate exercises a struct-returning `smelt.define`.
smelt.define parse_event_payload(
    payload_json: Expr<Text>
) -> Expr<Struct<{event_name: Text, platform: Text, url: Text}>> AS (
    {
        json_extract_string(payload_json, '$.event_name') AS event_name,
        json_extract_string(payload_json, '$.platform') AS platform,
        json_extract_string(payload_json, '$.url') AS url
    }
)

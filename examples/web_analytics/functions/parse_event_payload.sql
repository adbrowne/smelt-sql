-- Decode the raw JSON payload column from bronze.raw_events into a typed
-- struct. Bronze stores the payload as VARCHAR because smelt-datagen's
-- json_object generator emits Utf8; this function lifts that into named,
-- typed fields called by silver/events_parsed.
smelt.define parse_event_payload(
    payload_json: Expr<Text>
) -> Expr<Struct<{event_name: Text, platform: Text, url: Text}>> AS (
    {
        json_extract_string(payload_json, '$.event_name') AS event_name,
        json_extract_string(payload_json, '$.platform') AS platform,
        json_extract_string(payload_json, '$.url') AS url
    }
)

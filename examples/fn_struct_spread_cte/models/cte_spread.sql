-- D2 adversarial fixture: struct spread INSIDE a CTE body.
-- The outer SELECT reads the struct fields from the CTE alias.
-- Per function_schema_inference.md §4, propagation through CTEs must be
-- transparent: the fields lat and lon should resolve to Double here.
-- If the schema layer does not expand struct spreads inside CTE bodies,
-- lat and lon will resolve to Unknown.
WITH parsed AS (
    SELECT
        reading_id,
        smelt.functions.parse_lat_lon(raw_coords).*
    FROM smelt.sources.gps_readings
)
SELECT
    reading_id,
    lat,
    lon
FROM parsed

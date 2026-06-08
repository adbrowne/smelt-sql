-- D2 fixture: struct spread at top-level SELECT (the working case).
-- smelt.functions.parse_lat_lon(raw_coords).* should expand to lat and lon
-- at the schema layer via collect_struct_spread_columns.
SELECT
    reading_id,
    smelt.functions.parse_lat_lon(raw_coords).*
FROM smelt.sources.gps_readings

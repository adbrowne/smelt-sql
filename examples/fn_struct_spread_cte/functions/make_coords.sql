-- D2 adversarial fixture: closed struct return used in *.spread.
-- parse_lat_lon takes a text column and returns a closed struct with two
-- typed fields: lat (Double) and lon (Double).
-- Used by both top_level_spread.sql (top-level SELECT) and
-- cte_spread.sql (spread inside a CTE body) to verify that struct
-- fields propagate through CTEs in the schema layer.
smelt.define parse_lat_lon(
    raw: Expr<Text>
) -> Expr<Struct<{lat: Double, lon: Double}>> AS (
    {
        CAST(SPLIT_PART(raw, ',', 1) AS DOUBLE) AS lat,
        CAST(SPLIT_PART(raw, ',', 2) AS DOUBLE) AS lon
    }
)

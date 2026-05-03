-- region_report: selects from the ephemeral seed smelt.lookup.regions.
-- The seed is declared materialization: ephemeral in models/lookup/regions.yml,
-- so this query must NOT create a lookup_regions table — instead the seed
-- rows are inlined as a VALUES CTE at compile time.
SELECT region_id, region_name FROM smelt.lookup.regions

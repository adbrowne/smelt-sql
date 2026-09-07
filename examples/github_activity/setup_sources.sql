-- Create the empty `raw.github_events` physical table so `smelt build`/`smelt
-- run` see the schema before any day is loaded. `run_incremental.py` appends
-- rows into it day by day (real day D plus the redelivered slice of D-1) from
-- `seeds/github_events_sample.parquet`; this script does not load any rows
-- itself.
--
-- Without a `name:` override, `smelt.sources.raw.github_events` resolves to
-- `<target_schema>.sources_raw_github_events` (`smelt-runtime`'s default
-- materialization name mapping, `docs/specs/architecture.md`); `smelt.yml`
-- here declares `schema: main`.
CREATE OR REPLACE TABLE main.sources_raw_github_events AS
SELECT * FROM read_parquet('seeds/github_events_sample.parquet') WHERE 1 = 0;

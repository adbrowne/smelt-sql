-- Load smelt-datagen-generated parquet into DuckDB source tables.
-- Run after `smelt-datagen --config datagen.yaml` writes Parquet under data/.
-- `smelt build` does not invoke this — it expects the raw schema to exist.

CREATE SCHEMA IF NOT EXISTS raw;

CREATE OR REPLACE TABLE raw.users AS
SELECT * FROM read_parquet('data/users/data.parquet');

CREATE OR REPLACE TABLE raw.devices AS
SELECT * FROM read_parquet('data/devices/data.parquet');

CREATE OR REPLACE TABLE raw.events AS
SELECT * FROM read_parquet('data/events/event_date=*/data.parquet', hive_partitioning=true);

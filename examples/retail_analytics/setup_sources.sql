-- Load generated parquet data into DuckDB source tables.
-- Run from the retail-analytics directory after data generation.

CREATE SCHEMA IF NOT EXISTS raw;

CREATE OR REPLACE TABLE raw.customers AS
SELECT * FROM read_parquet('data/customers/data.parquet');

CREATE OR REPLACE TABLE raw.products AS
SELECT * FROM read_parquet('data/products/data.parquet');

CREATE OR REPLACE TABLE raw.stores AS
SELECT * FROM read_parquet('data/stores/data.parquet');

CREATE OR REPLACE TABLE raw.orders AS
SELECT * FROM read_parquet('data/orders/order_date=*/data.parquet', hive_partitioning=true);

CREATE OR REPLACE TABLE raw.order_items AS
SELECT * FROM read_parquet('data/order_items/item_date=*/data.parquet', hive_partitioning=true);

CREATE OR REPLACE TABLE raw.web_events AS
SELECT * FROM read_parquet('data/web_events/event_date=*/data.parquet', hive_partitioning=true);

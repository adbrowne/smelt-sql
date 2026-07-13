<!-- GENERATED FILE — edit examples/web_analytics/tutorial_pages/changing-things.md and run python3 examples/web_analytics/generate_tutorial.py -->

# Backfills, new columns, and late updates

<!-- PLACEHOLDER: intro — the three ways an existing pipeline changes. -->

## Backfilling a range

```bash
smelt backbuild silver.events_parsed --start 2026-04-01 --end 2026-04-19 --dry-run
```

<!-- smelt-generate: @cwd=tutorial_stages/03_late_data @render=skeleton backbuild silver.events_parsed --start 2026-04-01 --end 2026-04-19 --dry-run -->
```sql
-- Would run: bronze.raw_events (materialization shown below)
-- … model SELECT body (see the full SQL below) …

) _smelt_typed

-- Would run: silver.events_parsed (materialization shown below)
-- … model SELECT body (see the full SQL below) …

) _smelt_typed

-- chunk 1/2: [2026-04-01, 2026-04-10)
BEGIN
  DELETE FROM main.silver_events_parsed WHERE event_date >= '2026-04-01' AND event_date < '2026-04-10'
  INSERT INTO main.silver_events_parsed SELECT * FROM (
-- … model SELECT body (see the full SQL below) …

) AS _smelt_output_clamp WHERE event_date >= '2026-04-01' AND event_date < '2026-04-10'
COMMIT
-- chunk 2/2: [2026-04-10, 2026-04-19)
BEGIN
  DELETE FROM main.silver_events_parsed WHERE event_date >= '2026-04-10' AND event_date < '2026-04-19'
  INSERT INTO main.silver_events_parsed SELECT * FROM (
-- … model SELECT body (see the full SQL below) …

) AS _smelt_output_clamp WHERE event_date >= '2026-04-10' AND event_date < '2026-04-19'
COMMIT
```

??? example "Full dry-run transcript — `smelt backbuild silver.events_parsed --start 2026-04-01 --end 2026-04-19 --dry-run`"

    <!-- smelt-generate: @cwd=tutorial_stages/03_late_data backbuild silver.events_parsed --start 2026-04-01 --end 2026-04-19 --dry-run -->
    ```sql
    -- Would run: bronze.raw_events (materialization shown below)
    SELECT CAST(event_id AS BIGINT) AS event_id, CAST(device_id AS INTEGER) AS device_id, CAST(user_id AS INTEGER) AS user_id, CAST(seconds_in_day AS INTEGER) AS seconds_in_day, CAST(event_time AS TIMESTAMP) AS event_time, CAST(arrival_time AS TIMESTAMP) AS arrival_time, CAST(utm_campaign AS VARCHAR) AS utm_campaign, CAST(payload AS VARCHAR) AS payload, CAST(event_date AS DATE) AS event_date FROM (
    SELECT
        event_id,
        device_id,
        user_id,
        seconds_in_day,
        CAST(event_time AS TIMESTAMP) AS event_time,
        CAST(arrival_time AS TIMESTAMP) AS arrival_time,
        utm_campaign,
        payload,
        CAST(event_date AS DATE) AS event_date
    FROM raw.events
    
    ) _smelt_typed
    
    -- Would run: silver.events_parsed (materialization shown below)
    SELECT CAST(event_id AS BIGINT) AS event_id, CAST(device_id AS INTEGER) AS device_id, CAST(user_id AS INTEGER) AS user_id, CAST(amplitude_id AS VARCHAR) AS amplitude_id, CAST(event_ts AS TIMESTAMP) AS event_ts, CAST(event_date AS DATE) AS event_date, CAST(utm_campaign AS VARCHAR) AS utm_campaign, CAST(event_name AS VARCHAR) AS event_name, CAST(platform AS VARCHAR) AS platform, CAST(url AS VARCHAR) AS url FROM (
    SELECT
        event_id,
        device_id,
        user_id,
        CASE WHEN user_id IS NOT NULL
             THEN 'u:' || CAST(user_id AS VARCHAR)
             ELSE 'd:' || CAST(device_id AS VARCHAR)
        END AS amplitude_id,
        CAST(event_time AS TIMESTAMP) AS event_ts,
        CAST(event_date AS DATE) AS event_date,
        utm_campaign,
        json_extract_string(payload, '$.event_name') AS event_name,
        json_extract_string(payload, '$.platform') AS platform,
        json_extract_string(payload, '$.url') AS url
    FROM main.bronze_raw_events
    WHERE event_date
        BETWEEN CAST(arrival_time AS DATE) - INTERVAL '3 days'
            AND CAST(arrival_time AS DATE)
    QUALIFY ROW_NUMBER() OVER (PARTITION BY event_id ORDER BY arrival_time) = 1
    
    ) _smelt_typed
    
    -- chunk 1/2: [2026-04-01, 2026-04-10)
    BEGIN
      DELETE FROM main.silver_events_parsed WHERE event_date >= '2026-04-01' AND event_date < '2026-04-10'
      INSERT INTO main.silver_events_parsed SELECT * FROM (
    SELECT
        event_id,
        device_id,
        user_id,
        CASE WHEN user_id IS NOT NULL
             THEN 'u:' || CAST(user_id AS VARCHAR)
             ELSE 'd:' || CAST(device_id AS VARCHAR)
        END AS amplitude_id,
        CAST(event_time AS TIMESTAMP) AS event_ts,
        CAST(event_date AS DATE) AS event_date,
        utm_campaign,
        json_extract_string(payload, '$.event_name') AS event_name,
        json_extract_string(payload, '$.platform') AS platform,
        json_extract_string(payload, '$.url') AS url
    FROM (SELECT * FROM main.bronze_raw_events WHERE event_date >= '2026-03-29' AND event_date < '2026-04-10')
    WHERE event_date
        BETWEEN CAST(arrival_time AS DATE) - INTERVAL '3 days'
            AND CAST(arrival_time AS DATE)
    QUALIFY ROW_NUMBER() OVER (PARTITION BY event_id ORDER BY arrival_time) = 1
    
    ) AS _smelt_output_clamp WHERE event_date >= '2026-04-01' AND event_date < '2026-04-10'
    COMMIT
    -- chunk 2/2: [2026-04-10, 2026-04-19)
    BEGIN
      DELETE FROM main.silver_events_parsed WHERE event_date >= '2026-04-10' AND event_date < '2026-04-19'
      INSERT INTO main.silver_events_parsed SELECT * FROM (
    SELECT
        event_id,
        device_id,
        user_id,
        CASE WHEN user_id IS NOT NULL
             THEN 'u:' || CAST(user_id AS VARCHAR)
             ELSE 'd:' || CAST(device_id AS VARCHAR)
        END AS amplitude_id,
        CAST(event_time AS TIMESTAMP) AS event_ts,
        CAST(event_date AS DATE) AS event_date,
        utm_campaign,
        json_extract_string(payload, '$.event_name') AS event_name,
        json_extract_string(payload, '$.platform') AS platform,
        json_extract_string(payload, '$.url') AS url
    FROM (SELECT * FROM main.bronze_raw_events WHERE event_date >= '2026-04-07' AND event_date < '2026-04-19')
    WHERE event_date
        BETWEEN CAST(arrival_time AS DATE) - INTERVAL '3 days'
            AND CAST(arrival_time AS DATE)
    QUALIFY ROW_NUMBER() OVER (PARTITION BY event_id ORDER BY arrival_time) = 1
    
    ) AS _smelt_output_clamp WHERE event_date >= '2026-04-10' AND event_date < '2026-04-19'
    COMMIT
    ```

<!-- PLACEHOLDER: chunking walkthrough. -->

## Adding a column

<!-- PLACEHOLDER: the is_purchase change (stage 04). -->

```bash
smelt diff --select silver.events_parsed
```

<!-- smelt-generate: @cwd=tutorial_stages/04_add_column @fixture-schemas @render=text @expect-exit=1 diff --select silver.events_parsed -->
```text
Model: silver.events_parsed
  ADD COLUMN is_purchase BOOLEAN NULL
  -> Safe: ALTER TABLE (no data loss)
     ALTER TABLE main.silver_events_parsed ADD COLUMN is_purchase BOOLEAN

Summary: 1 changed, 0 new, 0 removed, 0 unchanged
```

<!-- PLACEHOLDER: ALTER + NULL history, backfill:, NOT NULL caveat. -->

## When upstream data changes

```bash
smelt run --since-upstream \
  --source silver.events_parsed --landed 2026-04-10..2026-04-11 --dry-run
```

<!-- smelt-generate: @cwd=tutorial_stages/05_enrichment @render=dirty-set run --since-upstream --source silver.events_parsed --landed 2026-04-10..2026-04-11 --dry-run -->
```text
Dirty set (--since-upstream):
  silver.events_enriched <- silver.events_parsed: [2026-04-10, 2026-04-11)
  silver.events_enriched <- silver.sessions: [2026-04-08, 2026-04-14)
  silver.sessions <- silver.events_parsed: [2026-04-09, 2026-04-13)
  RUN silver.events_enriched: [2026-04-08, 2026-04-14)
  RUN silver.sessions: [2026-04-09, 2026-04-13)
```

<!-- PLACEHOLDER: propagation walkthrough; note the canonical example's
self-referential table makes the propagation graph refuse. -->

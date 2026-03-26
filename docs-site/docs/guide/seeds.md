# Seeds

Seeds are CSV files that smelt loads into your database as tables. They are useful for small reference datasets, lookup tables, and test data that you want to version-control alongside your models.

## Directory structure

Place CSV files in the `seeds/` directory (configurable via `seed_paths` in `smelt.yml`). Subdirectories map to schema names in the database.

```
my_project/
  seeds/
    raw/
      users.csv
      transactions.csv
      events.csv
      sessions.csv
  models/
    ...
  smelt.yml
```

In this example, `seeds/raw/users.csv` is loaded as the table `raw.users`.

## CSV format

Seeds are standard CSV files with a header row:

```csv
user_id,user_name,signup_date
1,Alice,2025-01-01
2,Bob,2025-01-02
3,Charlie,2025-01-03
4,Diana,2025-01-04
5,Eve,2025-01-05
```

smelt infers column types from the data. The table is created (or replaced) each time you run the seed command.

## Commands

### Load all seeds

```bash
smelt seed
```

### Load and display results

```bash
smelt seed --show-results
```

This prints the loaded data in a table format after seeding, which is helpful for verifying that the CSV was parsed correctly.

### Load specific seeds

```bash
smelt seed --select users
smelt seed --select raw.users
```

Use `--select` to load only specific seed files by name or by `schema.name`.

### Build (seed + run)

```bash
smelt build
```

The `build` command combines seeding and model execution in one step. It loads all seeds first, then runs all models. This is the most common command during development.

!!! tip
    Use `smelt build` when starting fresh or resetting your development database. It ensures seeds are loaded before any models that depend on them run.

## Configuration

The seed directory defaults to `seeds/`. Override it in `smelt.yml`:

```yaml
seed_paths:
  - seeds
  - test_data
```

Multiple directories are supported. Each is scanned for CSV files, and subdirectories map to schema names as described above.

## Target selection

Like model runs, seed loading respects the `--target` flag:

```bash
smelt seed --target dev
smelt seed --target spark
```

## When to use seeds

Seeds work well for:

- **Reference data** -- Country codes, status enums, category mappings
- **Test data** -- Small datasets for development and testing
- **Static lookups** -- Data that rarely changes and is small enough to version in git

!!! warning
    Seeds are not designed for large datasets. CSV files are fully loaded into memory and inserted as a single batch. For datasets larger than a few thousand rows, use [Sources](sources.md) instead and load the data with your ingestion pipeline.

## Further reading

- [Sources](sources.md) for referencing external tables not managed by smelt
- [Targets and Backends](targets.md) for configuring where seeds are loaded

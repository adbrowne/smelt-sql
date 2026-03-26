# Sources Configuration (sources.yml)

The `sources.yml` file declares external data sources that your smelt models can reference. It defines the schema (column names and types) of tables that exist outside your project, such as raw data tables loaded by an ingestion pipeline.

Place this file at the root of your project directory alongside `smelt.yml`.

## File Format

Sources are defined in YAML with the following top-level structure:

```yaml
version: 1

sources:
  <schema_name>:
    tables:
      <table_name>:
        description: <string>
        columns:
          - name: <column_name>
            type: <SQL_TYPE>
            description: <string>  # optional
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `version` | integer | yes | Schema version (currently `1`) |
| `sources` | map | yes | Map of schema names to their table definitions |

---

## Schema Definition

Each key under `sources` represents a schema name (e.g., `raw`, `staging`). A schema contains a `tables` map.

```yaml
sources:
  raw:
    tables:
      # table definitions...
```

---

## Table Definition

Each key under `tables` is a table name. Tables have the following fields:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `description` | string | no | Human-readable description of the table |
| `columns` | list | yes | List of column definitions |

---

## Column Definition

Each column in the `columns` list has the following fields:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Column name |
| `type` | string | yes | SQL data type (see [Supported Column Types](#supported-column-types)) |
| `description` | string | no | Human-readable description of the column |

---

## Supported Column Types

Column types are specified as standard SQL type names. The following types are supported:

### Numeric Types

| Type | Description |
|------|-------------|
| `INTEGER` | 32-bit signed integer |
| `BIGINT` | 64-bit signed integer |
| `FLOAT` | 32-bit floating point |
| `DOUBLE` | 64-bit floating point |
| `DECIMAL(p,s)` | Fixed-point decimal with precision `p` and scale `s` |

### String Types

| Type | Description |
|------|-------------|
| `VARCHAR` | Variable-length character string |

### Boolean Types

| Type | Description |
|------|-------------|
| `BOOLEAN` | True/false value |

### Date and Time Types

| Type | Description |
|------|-------------|
| `DATE` | Calendar date (year, month, day) |
| `TIMESTAMP` | Date and time (without time zone) |

### Semi-Structured Types

| Type | Description |
|------|-------------|
| `JSON` | JSON data |

---

## How Sources Are Used

Source tables declared in `sources.yml` serve several purposes:

- **Type checking**: The smelt type checker uses column definitions to validate that your models reference valid columns with correct types.
- **Documentation**: Descriptions provide context for source data in the project graph.
- **LSP support**: The language server uses source definitions to provide diagnostics and completions.

Models reference source tables using `smelt.source()` with the schema-qualified name:

```sql
SELECT user_id, event_type, event_timestamp
FROM smelt.source('raw.events')
```

---

## Complete Example

The following example is based on the timeseries project and shows a typical `sources.yml` with multiple tables:

```yaml
version: 1

sources:
  raw:
    tables:
      users:
        description: Raw user data
        columns:
          - name: user_id
            type: INTEGER
          - name: user_name
            type: VARCHAR
          - name: signup_date
            type: DATE

      events:
        description: Raw event data
        columns:
          - name: event_id
            type: INTEGER
          - name: user_id
            type: INTEGER
          - name: event_type
            type: VARCHAR
          - name: event_timestamp
            type: TIMESTAMP

      transactions:
        description: Transaction events with timestamps and amounts
        columns:
          - name: transaction_id
            type: INTEGER
            description: Unique transaction identifier
          - name: user_id
            type: INTEGER
            description: User who made the transaction
          - name: amount
            type: DECIMAL(10,2)
            description: Transaction amount
          - name: transaction_timestamp
            type: TIMESTAMP
            description: When the transaction occurred
          - name: transaction_type
            type: VARCHAR
            description: Type of transaction (purchase, refund, etc.)

      sessions:
        description: User session data
        columns:
          - name: session_id
            type: INTEGER
          - name: user_id
            type: INTEGER
          - name: session_start
            type: TIMESTAMP
          - name: session_end
            type: TIMESTAMP
```

## Larger Example

For projects with more source tables, the structure scales naturally. This excerpt from a retail analytics project shows dimension and fact tables:

```yaml
version: 1

sources:
  raw:
    tables:
      customers:
        description: Customer dimension table
        columns:
          - name: customer_id
            type: INTEGER
          - name: name_prefix
            type: VARCHAR
          - name: birth_year
            type: INTEGER
          - name: gender
            type: VARCHAR
          - name: email_domain
            type: VARCHAR
          - name: country
            type: VARCHAR
          - name: city
            type: VARCHAR
          - name: signup_date
            type: VARCHAR
          - name: segment
            type: VARCHAR

      products:
        description: Product dimension table
        columns:
          - name: product_id
            type: INTEGER
          - name: category
            type: VARCHAR
          - name: subcategory
            type: VARCHAR
          - name: brand_tier
            type: VARCHAR
          - name: unit_price_cents
            type: INTEGER
          - name: weight_grams
            type: INTEGER
          - name: is_digital
            type: BOOLEAN

      orders:
        description: Order fact table
        columns:
          - name: order_id
            type: INTEGER
          - name: customer_id
            type: INTEGER
          - name: store_id
            type: INTEGER
          - name: order_date
            type: VARCHAR
          - name: status
            type: VARCHAR
          - name: payment_method
            type: VARCHAR
          - name: shipping_method
            type: VARCHAR
          - name: discount_pct
            type: INTEGER

      order_items:
        description: Order line items fact table
        columns:
          - name: order_item_id
            type: INTEGER
          - name: order_id
            type: INTEGER
          - name: product_id
            type: INTEGER
          - name: quantity
            type: INTEGER
          - name: unit_price_cents
            type: INTEGER
          - name: return_flag
            type: BOOLEAN
          - name: return_reason
            type: VARCHAR
          - name: item_date
            type: VARCHAR
```

# Records

The meta-language provides two ways to work with structured, named-field data at compile time: **named record declarations** (`smelt.record TypeName = { … }`) for shapes that recur across files, and **inline record types** (`{f: T, …}`) for one-off schemas at any type-annotation position. Record values are constructed by **record literals** (`{f: v, …}`) and navigated by **field projection** (`r.field`).

Records are meta-only: no `Record<…>` value ever reaches the database engine.

## Named record declarations

```sql
smelt.record TypeName = { field1: Type1, field2: Type2, … }
```

A `smelt.record` declaration introduces a named record type at **workspace scope**. Declarations are top-level statements (alongside `smelt.define`); the name must be unique across the entire workspace. A second declaration with the same name emits `SmeltRecordRedefinition` at the offending name token.

**Field types** may be any meta-language type that is valid at a type-annotation position:

- Scalar `DataType` literals: `Text`, `Integer`, `Float`, `Boolean`, `Timestamp`, `Date`, `Decimal`
- `List<T>` (where `T` is any valid field type)
- `Map<Text, V>` (where `V` is any valid field type)
- An inline record type `{…}`
- A previously declared `smelt.record` name

Reflection witnesses (`ColumnRef`, `ModelRef`, `SourceRef`) and `Lambda<…>` are **not** writable as field types. Using one emits `RecordFieldTypeForbidden` at the field's type span.

**Referencing a named record.** The bare name serves as a type in any type-annotation position: as a loader schema, as a field type of another record, or as the `V` in `Map<Text, V>`:

```sql
smelt.record Region = { name: Text, threshold: Integer }
smelt.record Config  = { regions: List<Region>, debug: Boolean }
```

## Inline record types

At any type-annotation position, a brace-delimited field list is an **inline (anonymous) record type**:

```sql
{name: Text, region: Text, threshold: Integer}
```

Inline records are **structurally typed**: two inline records with the same field set (names and types, in any order) are the same type. An inline record is interchangeable with a named record whose field set matches, per the width-subtyping rules below.

Inline types are the primary surface for one-shot loader schemas. Named declarations are the surface for shapes that recur across files and want a goto-definition target.

## Record literals

A record value is constructed by a brace-delimited key-value list:

```sql
{field1: value1, field2: value2, …}
```

The literal is **bidirectionally type-checked** against its surrounding target type. When the target type is known (from a loader schema, a function parameter annotation, or a HOF return position), the type checker validates each field in the literal against the target:

- Every field in the target type must appear in the literal exactly once. A missing field emits `RecordFieldMissing` at the literal's closing brace.
- A literal field whose name is not in the target type emits `RecordFieldUnknown` at the offending field-name token.
- A literal that names the same field twice emits `RecordFieldDuplicate` at the second occurrence.
- Each field value must be assignable to the declared field type; mismatches emit `RecordFieldTypeMismatch` at the value expression.

A record literal in a position where no target type can be inferred emits `RecordLiteralUnknownTarget` at the opening brace.

## Field projection

For a record-typed value `r`, the expression `r.fieldname` is field projection. It synthesises the declared field's type and lifts it into the surrounding splice context. Projection of an unknown field emits `RecordFieldUnknown` at the field-name token, listing the valid field names.

Field projection is **recursive**: `r.outer.inner` projects `outer`, then `inner`. Each step type-checks independently. Projecting through a non-record-typed intermediate emits `RecordFieldNotProjectable` at the offending step.

## Width subtyping

Record types follow **width subtyping**: a record with _more_ fields is a subtype of a record with _fewer_ fields. A value typed `{a: T, b: U}` is assignable to any position expecting `{a: T}`. The reverse is not true — a position expecting `{a: T, b: U}` cannot be satisfied by a value typed `{a: T}` alone.

This applies uniformly to named and inline records:

```sql
smelt.record Tenant = { plan: Text, threshold: Integer }
-- Tenant <: {plan: Text}  ← the wider record is assignable to the narrower shape
-- {plan: Text} is NOT assignable to Tenant — threshold is missing
```

Width subtyping is particularly useful with HOF lambdas: a lambda that consumes only `entry.plan` can receive a `Tenant`-typed value without an explicit projection. It does not weaken field-projection diagnostics — `r.b` on a value statically typed `{a: T}` still emits `RecordFieldUnknown`.

## Worked example

The `examples/meta_config/` workspace declares a `Cohort` record and loads a YAML config file as a list of cohorts:

```sql
-- examples/meta_config/models/cohorts.sql
smelt.record Cohort = { name: Text, region: Text, threshold: Integer }

SELECT smelt.config.load_yaml('configs/cohorts.yaml', List<{name: Text, region: Text, threshold: Integer}>)
```

With `configs/cohorts.yaml`:

```yaml
- name: us_west
  region: us-west-2
  threshold: 100
- name: us_east
  region: us-east-1
  threshold: 100
- name: eu
  region: eu-west-1
  threshold: 50
```

The loader validates each YAML record against the inline schema `{name: Text, region: Text, threshold: Integer}` at compile time. Because `Cohort` and that inline type have the same field set, a `Cohort`-typed value is width-assignable to the loader's inline schema. The named declaration adds a goto-definition target and improves hover display.

## LSP support

- **Hover** on a `smelt.record` declaration name shows the full field list with types and the file where the declaration lives.
- **Hover** on a record-typed binding shows the record name (or `{…}` for an inline record) and the closed field list.
- **Hover** on a field projection `r.fieldname` shows the field's declared type.
- **Goto-definition** on a `smelt.record` name reference resolves to the declaration site.
- **Goto-definition** on a record literal's field name resolves to the corresponding field declaration in the record type.
- **Completion** at a record literal field-key position (`{<cursor>…}`) offers the unfilled fields of the target type with their declared types.
- **Completion** at a field-projection site (`r.<cursor>`) offers the record's closed field list.

## Diagnostic codes

---

!!! warning "SmeltRecordRedefinition"
    **When it fires:** A `smelt.record` declaration uses a name that is already declared in the workspace.

    **Message:** `record '{name}' is already declared in {path}; record names must be unique workspace-wide`

    **Example:**
    ```sql
    -- file A
    smelt.record Config = { threshold: Integer }
    -- file B
    smelt.record Config = { debug: Boolean }  -- ← SmeltRecordRedefinition
    ```

    **What to fix:** Choose a distinct name for the second declaration, or consolidate both declarations into one.

---

!!! warning "RecordFieldUnknown"
    **When it fires:** A field projection or record literal uses a field name not declared on the target record type.

    **Message:** `record '{type}' has no field '{name}'; expected one of: {fields}`

    **Example:**
    ```sql
    smelt.record Cohort = { name: Text, region: Text, threshold: Integer }
    -- r.country is not a Cohort field:
    SELECT map(entries, fn r => r.country)  -- ← RecordFieldUnknown
    ```

    **What to fix:** Use one of the declared field names, or add the missing field to the `smelt.record` declaration if the field should exist.

---

!!! warning "RecordFieldMissing"
    **When it fires:** A record literal omits a field required by the target type.

    **Message:** `record literal for '{type}' is missing required field '{name}'`

    **Example:**
    ```sql
    smelt.record Cohort = { name: Text, region: Text, threshold: Integer }
    -- threshold is omitted:
    SELECT {name: 'eu', region: 'eu-west-1'}  -- ← RecordFieldMissing: threshold
    ```

    **What to fix:** Add the missing field with an appropriate value, or use a narrower inline record type if the field is not needed in this position.

---

!!! warning "RecordFieldDuplicate"
    **When it fires:** A record literal names the same field twice.

    **Message:** `field '{name}' already appears in this record literal`

    **Example:**
    ```sql
    SELECT {name: 'eu', name: 'us'}  -- ← RecordFieldDuplicate on the second `name`
    ```

    **What to fix:** Remove the duplicate field occurrence.

---

!!! warning "RecordFieldTypeMismatch"
    **When it fires:** A field value's type is not assignable to the declared field type.

    **Message:** `record field '{name}' expects {expected}; found {actual}`

    **Example:**
    ```sql
    smelt.record Cohort = { name: Text, threshold: Integer }
    SELECT {name: 'eu', threshold: 'not_a_number'}  -- ← RecordFieldTypeMismatch: threshold
    ```

    **What to fix:** Cast or replace the value so it matches the declared field type.

---

!!! warning "RecordLiteralUnknownTarget"
    **When it fires:** A record literal appears in a position where no target type can be inferred.

    **Message:** `cannot infer record type from context; annotate the target type`

    **Example:**
    ```sql
    SELECT {name: 'eu', threshold: 50}  -- ← RecordLiteralUnknownTarget: no target
    ```

    **What to fix:** Provide context for the type checker — pass the literal as a loader schema, annotate a `smelt.define` parameter, or wrap in a typed position.

---

!!! warning "RecordFieldNotProjectable"
    **When it fires:** A mid-chain field projection steps through a non-record-typed value.

    **Message:** `value of type {type} has no fields; projection '{field}' is not valid`

    **Example:**
    ```sql
    smelt.record Cohort = { name: Text, threshold: Integer }
    -- threshold is Integer, not a record — .sub_field doesn't exist:
    SELECT r.threshold.sub_field   -- ← RecordFieldNotProjectable
    ```

    **What to fix:** Stop the projection at the correct depth. Use `r.threshold` to get the `Integer` value.

---

!!! warning "RecordFieldTypeForbidden"
    **When it fires:** A `smelt.record` field type references a reflection witness (`ColumnRef`, `ModelRef`, `SourceRef`, or `Lambda<…>`), which are not user-writable as field types.

    **Message:** `record field types may not reference {type}; reflection witnesses are not user-writable`

    **Example:**
    ```sql
    smelt.record Broken = { columns: ColumnRef }  -- ← RecordFieldTypeForbidden
    ```

    **What to fix:** Use a concrete `DataType` or a user-authored record type. Reflection witnesses (`ColumnRef`, `ModelRef`, `SourceRef`) can only be produced and consumed via the corresponding reflection APIs — they are not storable in user-declared record fields.

---

!!! warning "RecordCyclicDeclaration"
    **When it fires:** A `smelt.record` declaration references its own name (directly or through a chain of other declarations), forming a cycle.

    **Message:** `record '{name}' is cyclic; record declarations must form a DAG`

    **Example:**
    ```sql
    smelt.record A = { child: B }
    smelt.record B = { parent: A }  -- ← RecordCyclicDeclaration
    ```

    **What to fix:** Inline one of the records into the other, or break the cycle by replacing one self-reference with a scalar type. Records must form a DAG.

---

!!! warning "RecordInDataWorld"
    **When it fires:** A record-typed binding is referenced bare in a Data-World SQL position (e.g. inside a `WHERE` clause or a SELECT item) without projecting one of its fields. A record value cannot materialise as SQL on its own; only its projected fields can.

    **Message:** `record-typed value '{name}' has no Data-World representation; project a field (e.g. `{name}.field`) or consume it inside a meta-language splice`

    **Example:**
    ```sql
    smelt.define cohort_query(c: Cohort) -> TableExpr AS (
        SELECT id FROM smelt.sources.raw.users WHERE c       -- ← RecordInDataWorld
    )
    ```

    **What to fix:** Project a field of the record (`c.name`, `c.threshold`) instead of using the bare binding. Record values live in the meta-world; their projected scalar fields cross into the Data-World.

# Wide Reflection: smelt.models.*, smelt.sources.*, ModelRef, SourceRef

Reference doc for the wide-reflection accessor surface. Use this alongside
`smelt docs show meta-language/reflection` for the canonical API.

## Quick summary

```sql
-- All models tagged 'cohort', in path-sorted order
smelt.models.with_tag('cohort')    -- -> List<ModelRef>

-- Every model in the workspace
smelt.models.all()                  -- -> List<ModelRef>

-- All sources tagged 'audit'
smelt.sources.with_tag('audit')    -- -> List<SourceRef>

-- Every declared source
smelt.sources.all()                 -- -> List<SourceRef>
```

## Closed field sets

**ModelRef** and **SourceRef** have exactly four fields each (same shape):

| Field     | Type            | Meaning                                              |
|-----------|-----------------|------------------------------------------------------|
| `path`    | `Text`          | Workspace-relative path (e.g. `models/orders.sql`)  |
| `name`    | `Text`          | Short model/source name (e.g. `orders`)              |
| `tags`    | `List<Text>`    | Merged tag set (smelt.yml first, then frontmatter)   |
| `columns` | `List<ColumnRef>` | Column list — equivalent to `smelt.columns_of(m)` |

Unknown fields emit `ModelRefFieldUnknown` / `SourceRefFieldUnknown`.

## Subtyping: ModelRef and SourceRef lift to TableExpr

`ModelRef <: TableExpr` and `SourceRef <: TableExpr`. This means you can pass
a `ModelRef` value wherever a `TableExpr` is required — including the `reduce`
argument and `smelt.columns_of`:

```sql
-- GOOD: direct reduce — no explicit projection needed
SELECT map(smelt.models.with_tag('cohort'), fn m => m.name)

-- GOOD: columns_of accepts ModelRef directly
SELECT map(smelt.columns_of(m), fn c => c.name)  -- m: ModelRef
```

You do NOT need to write `.table_expr` or any explicit projection. The
subtyping lift is invisible.

## The identifier-lift positions are NOT widened

`m.path` and `m.name` resolve to `Text` at body-check time. They lift to SQL
identifiers at the same four positions as Phase C's `ColumnRef.name`:
- SELECT alias: `c.name AS alias` (but only `c.name` not `m.path`)
- Column reference in WHERE/GROUP BY/ORDER BY
- Column literal in SELECT
- CTE column name

Phase D does NOT add new lift positions. `m.path` cannot be used as a table
alias or a CTE name — those lift positions land in Phase E2.

## Argument rules for with_tag and all

- `with_tag('tag')` — exactly one positional Text literal. Named arguments
  (`tag => 'cohort'`) emit `WithTagNamedArgument`. A non-literal argument
  like `UPPER('cohort')` emits `WithTagRequiresText`.
- `all()` — no arguments. Any argument emits `WideReflectionUnexpectedArgument`.

## Path-sorted determinism

`smelt.models.with_tag(t)` and `smelt.models.all()` return models sorted by
their workspace-relative path, byte-lexicographic with `/` separators. This
order is stable under workspace edits and is guaranteed to be byte-equal
across Salsa re-evaluations on the same workspace state.

A `reduce(smelt.models.with_tag('cohort'), union_all)` query renders UNION ALL
branches in this path order. If row order matters downstream, document the
dependency on this guarantee.

## m.columns vs smelt.columns_of(m)

`m.columns` and `smelt.columns_of(m)` are equivalent — both return
`List<ColumnRef>` with the same column list. Use whichever reads more
naturally. At body-check time both return `List<ColumnRef>` parametrically;
at expansion time both resolve the concrete schema.

## Common diagnostics

| Code | Cause | Fix |
|------|-------|-----|
| `WithTagRequiresText` | `with_tag` argument is not a compile-time Text literal | Use a string literal: `'my-tag'` |
| `WithTagNamedArgument` | `with_tag` called with a named argument | Use positional: `with_tag('my-tag')` not `with_tag(tag => 'my-tag')` |
| `WideReflectionUnknownAccessor` | Unknown accessor like `smelt.models.bogus` | Use `with_tag` or `all` only |
| `WideReflectionUnexpectedArgument` | `all()` called with an argument | Remove the argument |
| `ModelRefFieldUnknown` | Unknown field on ModelRef (e.g. `m.materialization`) | Use only `path`, `name`, `tags`, `columns` |
| `SourceRefFieldUnknown` | Unknown field on SourceRef (e.g. `s.schema`) | Use only `path`, `name`, `tags`, `columns` |

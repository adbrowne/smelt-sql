# Map consumption surface — `keys` / `values` / `entries`

**Date:** 2026-06-03
**Status:** Design approved (Andrew, 2026-06-03) — precursor to a `docs/specs/` increment and plan phase **P7d** in `docs/plans/20260531-diagnostic-parity.md`.
**Resolves:** the P7c pause recorded in `7ca33704` — a `Map<K,V>` config-loader value has no parser-supported in-model consumer, so the now-forbidden bare `SELECT load_yaml(..., Map<…>)` form has no clean replacement.

## Problem

P7c's resolved design decision (loaders must be consumed) extended the P6 "bare
`List<T>` in scalar position is `MetaListInScalarPosition`" detector to cover
config-loader values. That detector, committed in `58c2fcd4`, now forbids the
exact bare form used by the `meta_config` example's own happy-path fixtures:

```sql
-- examples/meta_config/models/tenants.sql
SELECT smelt.config.load_yaml('configs/tenants.yaml', Map<Text, {plan: Text, threshold: Integer}>)
```

The **List** fixture (`cohorts.sql`) has a clean consuming rewrite
(`reduce(map(load_yaml(...), fn c => c.region), concat_with(', '))`), but the
**Map** fixture does not: a `Map<K,V>` value has no parser-supported in-model
consumer. Both attempted forms fail to parse:

- `load_yaml(...) |> m => m.keys()` → *"pipe RHS must be a function call"*
- `load_yaml(...).keys()` → *"Expected RPAREN, found DOT"*

So the bare Map form is forbidden with no replacement, and the Map-root-loader
shape is documented across `maps.md`, `config-loaders.md`, `reference.md`, and
the canonical `tenants.sql`. This design adds the missing surface (direction B:
*wire Map consumption*, chosen over A: *drop Map-in-model* and C: *exempt bare
loaders*).

## Decision: general, documented surface

A real Map-consumption surface usable in any model — the surface the docs
already imply exists — not a minimal fixture-only consumer.

### Surface

Three new builtin meta-functions over a `Map<K,V>` value:

| Function     | Returns                       |
|--------------|-------------------------------|
| `keys(m)`    | `List<K>`                     |
| `values(m)`  | `List<V>`                     |
| `entries(m)` | `List<{ key: K, value: V }>`  |

Ordinary call syntax — **no parser changes**. `keys(x)` is a function call, and
the pipe form `m |> entries()` already parses because the pipe RHS is a call.
They compose with the existing pipe + HOF + spread machinery:

```sql
SELECT ...entries(load_yaml('configs/tenants.yaml', Map<Text, {plan: Text, threshold: Integer}>))
       |> map(fn e => e.key)
```

`entries(m)` yields records with **fixed field names** `key` and `value`. The
`value` field may itself be a record, so `e.value.plan` works via existing
record field access.

### Invariant preserved

The result of all three is a `List`, so it remains governed by the existing
"lists must be consumed" rule. A **bare** `keys(m)` / `values(m)` / `entries(m)`
in a Data-World scalar / SELECT-item position still emits
`MetaListInScalarPosition`. The Map surface adds no new escape hatch; P7c's
invariant stays intact. This is why Approach A (free functions) was chosen over
method syntax (`m.keys()`) — it reuses the List-consumption machinery wholesale
and introduces no method-dispatch concept into a language that today has only
functions and pipes.

## Layers touched

1. **Parser** — none. `keys(x)` / `values(x)` / `entries(x)` are function calls;
   `m |> entries()` already parses.
2. **Analyzer / type inference (`smelt-db`)** — recognize the three names as Map
   builtins; infer the return types above; a non-`Map` first argument produces a
   typed diagnostic, mirroring how `map` / `filter` validate their first arg.
   Registered alongside the existing builtin sites:
   `crates/smelt-db/src/queries/project.rs:776`, the HOF-name list at
   `crates/smelt-db/src/lib.rs:136`, and `crates/smelt-db/src/type_inference/`.
3. **Build-path meta-eval (`smelt-runtime`)** — evaluate `keys` / `values` /
   `entries` against the resolved Map value, materializing the `List` of element
   values and reusing the P7a/P7b record-element machinery (`entries` → records
   with `key` / `value` fields). Slots in next to `map` / `filter` / `reduce` at
   `crates/smelt-runtime/src/meta_eval.rs:679+`.

## Rejected alternatives

- **B-method syntax** (`m.keys()`, `m.values()`, `m.entries()`) — reads more
  naturally but requires parser work (postfix `.method()` on meta values is
  currently rejected) and introduces method-dispatch, a brand-new concept in a
  language with only functions + pipes. More surface for the same power.
- **C-`for k, v in m` generator** — most expressive for key+value iteration but
  largest scope (new generator syntax + lowering path specific to maps), and
  redundant once `entries()` exposes key+value via a record.

## Example, docs, spec

- **Rewrite `examples/meta_config/models/tenants.sql`** to a consuming form using
  `entries` / `keys`, keeping it a build+execute fixture.
- **Fold in the List half**: rewrite `examples/meta_config/models/cohorts.sql` to
  `reduce(map(load_yaml(...), fn c => c.region), concat_with(', '))` so pre-flight
  goes fully green in the same phase.
- **User docs** — `docs-site/docs/meta-language/maps.md` and `config-loaders.md`:
  document the three functions; add a negative case asserting
  `MetaListInScalarPosition` fires on the bare Map-loader form. Correct
  `reference.md` where the bare form implies a non-existent surface.
- **Spec increment** — `docs/specs/meta_language.md` (add the three builtins to
  the meta-function surface) and `docs/specs/meta_config_loading.md` (loader
  value reaches Data-World only through a consuming form; Map loaders via
  `keys` / `values` / `entries`).

## Tests (red-green)

- **Type inference** — `keys` / `values` / `entries` return types
  (`List<K>` / `List<V>` / `List<{key,value}>`); non-`Map` argument emits the
  typed diagnostic.
- **Scalar position** (`crates/smelt-db/tests/meta_list_scalar.rs`) — bare
  `entries(loader)` still emits `MetaListInScalarPosition`; a consumed
  `entries(...) |> map(...)` form emits zero.
- **Build + execute** (`crates/smelt-cli/tests/meta_config_e2e.rs`) — `tenants.sql`
  materializes the expected values from `configs/tenants.yaml`.

## Plan integration

New phase **P7d — Map consumption surface** in
`docs/plans/20260531-diagnostic-parity.md`, inserted before **P8 — Close-out**.
The List-half rewrite of `cohorts.sql` lands within P7d so pre-flight returns
green in one step. The autonomy loop resumes on P7d once the spec increment and
plan row are in place.

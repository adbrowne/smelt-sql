# Map loader consumption — resolving the P7c pause

**Date:** 2026-06-03
**Status:** Resolved (Andrew, 2026-06-03). Direction: **conform to the existing spec** — implement the already-specified method-call Map API; do *not* add a new free-function surface. Precursor to plan phase **P7d** in `docs/plans/20260531-diagnostic-parity.md`.
**Resolves:** the P7c pause recorded in `7ca33704` — a `Map<K,V>` config-loader value had no consumable in-model form, so the now-forbidden bare `SELECT load_yaml(..., Map<…>)` had no replacement.

## Correction: the surface already exists

An earlier draft of this doc proposed a *new* free-function surface
(`keys(m)` / `values(m)` / `entries(m)`). That was wrong: it was written without
reading `meta_language.md` §"Maps", which **already specifies a complete,
normative Map API** using **method-call syntax**. The free-function design is
discarded; this doc now records the spec-conformant direction.

The spec's closed Map API (`meta_language.md` lines 538–582):

| Operation | Signature |
|---|---|
| `m.entries()` | `Map<K, V> -> List<{key: K, value: V}>` (sorted ascending by `key`) |
| `m.keys()` | `Map<K, V> -> List<K>` |
| `m.values()` | `Map<K, V> -> List<V>` |
| `m.get(k)` | `(Map<K, V>, K) -> V` |
| `m.has(k)` | `(Map<K, V>, K) -> Boolean` |

with seven diagnostic codes (`MapApiUnknown`, `MapApiArityMismatch`,
`MapApiNamedArgument`, `MapApiUnexpectedArgument`, `MapGetMissingKey`,
`MapApiArgTypeMismatch`, `MapKeyTypeNotText`), `m.get` missing-key semantics, and
LSP hover/completion/goto-def behaviour — all normative today.

It is also **partially implemented**: the parser emits a `MAP_METHOD_CALL` node
(`crates/smelt-parser/src/parser/expr.rs`, test `parse_map_method_call_*`), type
inference lives in `crates/smelt-db/src/type_inference/record.rs`, and LSP
helpers exist — all with unit coverage.

## The actual gap

Three things stand between the specified-and-partially-implemented Map API and a
buildable `tenants.sql`:

1. **Parser — method call only on a bare-identifier receiver.** `MAP_METHOD_CALL`
   is produced only in the `IDENT . method ( … )` branch
   (`expr.rs:623–649`); the postfix loop after a primary handles only array
   subscript (`expr.rs:706–711`), so `load_yaml(...).entries()` fails with
   *"Expected RPAREN, found DOT"* — there is no postfix `.method()` on a call
   expression. The spec does not restrict the receiver to identifiers
   ("operations on a `Map<K, V>` value `m`"), so the fix is to allow a Map method
   call to chain on **any Map-typed expression** — notably a loader call. This is
   a parser change with **no surface change**.

2. **Build path — the loader family is not lowered at build.** Per
   `meta_language.md` §"Known Divergences" (the loader-family bullet) the
   `smelt.config.load_yaml` / `load_json` family is "analyzer-validated but not
   yet lowered at build" — it reaches the engine verbatim. P7d lowers a loader
   value consumed by a Map method call (and the List half via spread/HOF) into
   plain Data-World SQL at compile time, reusing the P7a/P7b record-element
   machinery.

3. **Diagnostics — `MAP_METHOD_CALL` not walked by `file_diagnostics`.** Per the
   same §"Known Divergences", the Map codes are emitted by pure inference
   functions but `check_file_diagnostics` does not yet walk `MAP_METHOD_CALL`
   nodes, so the editor shows no squiggle. Wiring this is in scope for parity.

## Example, docs, spec

- **Rewrite `examples/meta_config/models/tenants.sql`** to consume the Map loader
  via the spec's method API, e.g.
  `SELECT ...load_yaml('configs/tenants.yaml', Map<Text, {plan: Text, threshold: Integer}>).entries() |> map(fn e => e.key)`.
- **Fold in the List half**: rewrite `examples/meta_config/models/cohorts.sql` to
  a consuming form (`reduce(map(load_yaml(...), fn c => c.region), concat_with(', '))`)
  so pre-flight returns green in one step.
- **Spec** — a one-line clarification in `meta_language.md` §"Map API" that the
  receiver may be any `Map`-typed expression (a loader call included), not only a
  bound identifier; and a "Build-path execution" note for the loader/Map family
  mirroring the existing notes. No new surface. The §"Known Divergences" bullets
  (loader not lowered; `MAP_METHOD_CALL` not walked) are narrowed on land.
- **User docs** — `docs-site/docs/meta-language/{maps,config-loaders}.md`:
  show the `.entries()`/`.keys()` consuming form on a loader; add the negative
  case (bare Map loader → `MetaListInScalarPosition`).

## Tests (red-green)

- **Parser** — `load_yaml(...).entries()` (and `.keys()`) parses to a
  `MAP_METHOD_CALL` over the loader call receiver.
- **Type inference** — the loader-call-receiver method call infers
  `List<{key,value}>` / `List<K>`; non-`Map` receiver still diagnoses.
- **Scalar position** (`crates/smelt-db/tests/meta_list_scalar.rs`) — a bare
  `load_yaml(..., Map<…>).entries()` (a `List` result) still emits
  `MetaListInScalarPosition`; the consumed `… |> map(...)` form emits zero.
- **Build + execute** (`crates/smelt-cli/tests/meta_config_e2e.rs`) — `tenants.sql`
  materialises the expected keys/values from `configs/tenants.yaml`.

## Rejected

- **Free functions** (`keys(m)` / `entries(m)` …) — contradicts the normative
  method-call Map API and discards already-landed inference/diagnostics/LSP code.
- **A model-level `let` binding** so `m.entries()` has an identifier receiver —
  larger new surface than the postfix-receiver parser change, and unmotivated
  once the method call chains on the loader call directly.

## Plan integration

New phase **P7d — Map loader consumption** in
`docs/plans/20260531-diagnostic-parity.md`, before **P8 — Close-out**. The
autonomy loop resumes on P7d once the spec clarification and plan row are in
place.

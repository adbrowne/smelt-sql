# Phase A: meta-list literals and spread (`[…]`, `...xs`)

When you find yourself enumerating the same column list or expression sequence
multiple times, lift it to a `List<T>` literal and spread it at each use site.

## When to use

- Same set of columns referenced in two or more SELECT lists
- Dimension set driven by config or repeated pattern
- Repeated `coalesce` / `greatest` / `least` pattern over a stable column set

## Surface (Phase A)

- `[a, b, c]` — list literal; type `List<T>` where `T` is the LUB of elements
- `...xs` — spread the list into a comma-separated grammar position
- Empty list `[]` needs an inferable target; bare `[]` in a SELECT item fires
  `MetaListEmptyTypeUnknown`
- `...[]` (spread of an explicit empty list) silently elides — no diagnostic

## Where it works (Phase A)

- SELECT lists ✓ — `SELECT id, ...[name, email] FROM t`
- Parsing succeeds in other comma-separated positions (GROUP BY, function args,
  etc.), but type-check wiring for those positions is Phase B+.  Code in those
  positions will not trigger Phase A diagnostics; it will also not be
  type-checked for list correctness until Phase B lands.

## Phase A diagnostics

| Code | Trigger |
|------|---------|
| `MetaListHeterogeneous` | `[1, 'hello']` — element types don't unify |
| `MetaListEmptyTypeUnknown` | bare `[]` with no inferable target sort |
| `MetaSpreadInForbiddenPosition` | `...xs` in a WHERE clause |
| `MetaSpreadOnNonList` | `...42` where the operand is not a `List<T>` |

## Workflow gotchas

- **Bare `[]` vs `...[]`**: only bare `[]` as a SELECT item fires
  `MetaListEmptyTypeUnknown`. Spreading an empty list (`...[]`) elides
  silently — useful for no-op placeholders.
- **WHERE clause spreads**: the parser ejects `...` outside the WHERE node;
  the type-checker detects the orphan and emits
  `MetaSpreadInForbiddenPosition`. Use the `and_all` reducer (Phase B) for
  boolean composition.
- **Function-arg spreads**: `coalesce(...[a, b], 0)` parses without error but
  is not yet type-checked for list correctness in Phase A. No diagnostic fires;
  no expansion happens at codegen time either. Wrap the spread in a SELECT list
  position if you need Phase A guarantees.
- **Non-list operand**: `...42` (spread of a literal integer) fires
  `MetaSpreadOnNonList`. Make sure the operand is a list literal `[…]` or a
  variable typed `List<T>`.

See `docs-site/docs/meta-language/lists.md` for full surface and worked
examples (Phase 6 user docs).

# Meta-language polish surfaces (ternary, multi-arg lambdas, parameterised reducers)

Workflow gotchas for the meta-world polish surfaces. For the full syntax and
diagnostic-code reference, read:

- `docs-site/docs/meta-language/ternary.md`
- `docs-site/docs/meta-language/lambdas.md` (Multiple parameters)
- `docs-site/docs/meta-language/reducers.md` (Parameterised reducers)

This doc captures only the authoring-time mistakes that the docs don't make
obvious on first read.

## Ternary (`if cond then a else b`)

- `if`, `then`, `else` are **reserved meta-namespace keywords**. A
  `smelt.define if(...)`, `smelt.record else { ... }`, or lambda parameter
  named `then` emits `TernaryKeywordShadowed` and refuses to compile — there is
  no silent shadowing. Rename the declaration.
- The ternary is **meta-only**. Writing it inside a SQL `WHERE` clause, a SQL
  `SELECT` item that does not admit meta evaluation, or any other Data-World
  position emits `TernaryInDataPosition`. For runtime branching on a SQL value,
  use `CASE WHEN cond THEN a ELSE b END` instead.
- The condition must be a `Boolean` meta value. There is **no truthy/falsy
  coercion** — `if 'yes' then 1 else 0` emits `TernaryConditionNotBoolean`.
  Use an explicit comparison (`x > 0`, `m.has('k')`, `v = 'true'`).
- The two branches must unify under the same LUB rules used for list literals.
  Mixing sorts (`Expr<INTEGER>` vs `Expr<TEXT>`, `List<T>` vs `Expr<U>`) emits
  `TernaryBranchTypeMismatch` at the `else` keyword. Numeric-numeric is fine
  (promoted to the LUB); cross-sort is not.
- The ternary has **lower precedence than `|>`**. Parenthesise to pipe a
  ternary as a HOF argument: `map(xs, fn x => (if x > 0 then x else 0))`.
  Without the parens, `xs |> map(fn x => x) if cond then a else b` reads the
  entire pipe chain as the ternary's condition.
- **`else if` chains work without a special keyword.** The grammar is
  right-associative, so `if c1 then a else if c2 then b else c` parses as the
  obvious nested ternary. No `elsif`/`elseif` exists.
- **Map-defaulting idiom** — short-circuit evaluation suppresses
  `MapGetMissingKey` for the unreached branch, making this safe:
  ```sql
  SELECT if m.has('env') then m.get('env') else 'production'
  ```
  Without the `m.has` guard, `m.get('env')` emits `MapGetMissingKey` whenever
  the key is absent.

## Multi-arg lambdas (`fn (a, b) => body`)

- **Parens are mandatory for arity ≥ 2.** `fn a, b => body` is a parse error
  at the first comma; the parser cannot tell when the parameter list ends
  without the parens. The single-arg form `fn x => body` does not require
  parens (but `fn (x) => body` is also accepted).
- **No v1 HOF accepts a multi-arg lambda.** `map` and `filter` both require
  arity 1. Passing `fn (a, b) => body` to either emits `LambdaArityMismatch`
  at the lambda's parameter list. Multi-arg lambdas parse and type-check
  cleanly today but become useful only when a multi-list HOF is added.
- `fn () => body` (zero parameters) emits `LambdaZeroParameters`. If the body
  ignores its parameter, name it `_` (`fn _ => 42`).
- Duplicate parameter names within one lambda — `fn (x, x) => x` — emit
  `LambdaDuplicateParameter` at the second occurrence.

## Parameterised reducers (`concat_with(sep)`)

- The parameterised reducer call **must appear directly as the second argument
  to `reduce`**. `reduce(xs, concat_with(', '))` is correct; assigning the
  reducer to a variable or passing it through a list element is not supported.
- The separator argument must be **compile-time-resolvable**. A string literal
  (`', '`) or a `smelt.config.var('sep')` result is accepted. A SQL column
  reference, a `UPPER(...)` call, or any other runtime expression emits
  `ReducerArgNotCompileTime` at the offending argument.
- Argument count and types are checked against the closed registry:
  `concat_with(42)` → `ReducerArgTypeMismatch` (expects `Text`),
  `concat_with()` → `ReducerArityMismatch` (expects 1).
- **Positional arguments only.** `concat_with(sep => ', ')` (named argument)
  emits `ReducerNamedArgument`. The reducer registry is closed and does not
  support the `name => value` syntax.
- The empty-list identity for `concat_with(sep)` is the empty string `''`,
  independent of `sep`. An empty input list produces `''`, not an error — this
  differs from `union_all`/`intersect_all` which have no empty-list identity.
- `concat_with` is reserved at the meta-namespace level alongside the seven
  bare reducer names. `smelt.define concat_with(...)` emits
  `ReducerNameShadowed`.

## Diagnostic-code quick lookup

| Code | Surface | What triggered it |
|---|---|---|
| `TernaryConditionNotBoolean` | ternary | Condition is not `Boolean` |
| `TernaryBranchTypeMismatch` | ternary | Branches do not unify under LUB |
| `TernaryKeywordShadowed` | ternary | `if`/`then`/`else` used as a declared name |
| `TernaryInDataPosition` | ternary | Used in a SQL grammar position |
| `TernaryDanglingThen` | ternary | `then` with no preceding `if` |
| `TernaryDanglingElse` | ternary | `else` outside the current ternary |
| `LambdaArityMismatch` | multi-arg lambda | Lambda arity ≠ HOF requirement |
| `LambdaZeroParameters` | multi-arg lambda | `fn () => body` |
| `LambdaDuplicateParameter` | multi-arg lambda | Same parameter name twice |
| `ReducerArityMismatch` | parameterised reducer | Wrong arg count |
| `ReducerArgTypeMismatch` | parameterised reducer | Wrong arg type |
| `ReducerArgNotCompileTime` | parameterised reducer | Arg is a runtime expression |
| `ReducerNamedArgument` | parameterised reducer | `name => value` call form |
| `ReducerNameShadowed` | parameterised reducer | `concat_with` used as a declared name |

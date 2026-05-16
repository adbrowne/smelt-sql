# meta_polish

A minimal workspace exercising two of the three "polish" surfaces of the
meta-language:

- **`concat_with(sep)`** — parameterised reducer that joins a `List<Text>` with a
  compile-time separator string. `reduce(['alpha', 'beta', 'gamma'], concat_with(' OR '))`
  folds left with the separator, producing a single Text expression.

- **Ternary `if … then … else …`** — compile-time branching driven by
  `smelt.config.var('env')` from the workspace `vars:` block. The condition
  must synthesise to `Boolean`; the two branches must unify under LUB.
  Short-circuit evaluation means the statically-unreached branch is type-checked
  but not evaluated. Flip `vars.env` between `dev` and `prod` to change the
  emitted SQL without editing the model.

The multi-argument lambda surface (`fn (a, b) => …`) is part of the meta-language,
but no v1 higher-order function accepts a multi-arg lambda — placing one at an
arity-1 HOF call site emits `LambdaArityMismatch`. The negative path is covered
by `examples/meta_hofs_broken_lambda_arity_not_supported/`; this workspace does
not include a multi-arg lambda model because there is no arity-2-or-higher HOF
to consume one cleanly.

## Models

| File | Surface exercised |
|---|---|
| `models/concat_with_separator.sql` | `concat_with(' OR ')` parameterised reducer |
| `models/ternary_env_branch.sql` | `if smelt.config.var('env') = 'prod' then … else …` ternary |

## Broken sibling workspaces

Each sibling demonstrates one specific diagnostic and is covered by
`cargo test -p smelt-cli --test example_diagnostics`:

| Workspace | Diagnostic |
|---|---|
| `meta_polish_broken_ternary_non_boolean_cond/` | `TernaryConditionNotBoolean` |
| `meta_polish_broken_ternary_branch_mismatch/` | `TernaryBranchTypeMismatch` |
| `meta_polish_broken_reducer_arity/` | `ReducerArityMismatch` |

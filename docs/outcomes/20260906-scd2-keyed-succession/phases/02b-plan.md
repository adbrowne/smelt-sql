# Phase 2b plan — Pay down the `hardening_budget` regression in `analysis/succession.rs`

## Objective

`bash .claude/scripts/verify-phase.sh` is red on `smelt-core`'s
`hardening_budget::gate_detects_regression`: phase 2's classifier added four production
`unwrap`/`expect` sites in `crates/smelt-logical/src/analysis/succession.rs`, taking
`smelt-logical` from baseline `unwrap 1` / `expect 1` to `2` / `4`. Restructure those four
sites out of existence so the counts return to exactly the committed baseline, without
touching `.claude/hardening-baseline.txt`. Serves success criterion 10 (all standing gates
green) and unblocks every later phase's verification, which is otherwise ambiguous.

The gate is a **pure per-crate count ratchet** (`.claude/scripts/hardening-budget.sh` counts
`.unwrap()` / `.expect("` in the pre-`#[cfg(test)]` portion of each `src/**.rs`); there is no
"classified as infallible" allowlist. So the only baseline-honouring fix is elimination —
a `--update` here would be a silent lowering, which the fail-loud rule forbids.

## Spec delta

None. This is a code-shape change with no user-visible behaviour delta; the classifier's
verdicts and refusal reasons are unchanged, and `model_properties.md` §"Keyed-succession
classification" already describes them.

## The four sites (all in `crates/smelt-logical/src/analysis/succession.rs`)

| Line | Site | Why it is currently unwrapped |
|---|---|---|
| ~266 | `windows.into_iter().next().unwrap()` | guarded by the `1 =>` arm of `match windows.len()` |
| ~409 | `clock_col_name.expect(...)` | set on the first loop iteration; loop body ran ≥1 time |
| ~411 | `shared_order_expr.expect(...)` | same invariant |
| ~445 | `shared_partition.expect(...)` | same invariant |

## Tests

Red first: `cargo test -p smelt-core --test hardening_budget` fails on the unchanged tree
(counts 2/4 vs baseline 1/1); it is the phase's primary red-green oracle and must go green
with the baseline file untouched.

Two new unit tests in `succession.rs`'s `mod tests`, pinning the two refusal paths the
restructure rewrites and which today have no coverage:

- `refuses_order_by_expression_not_bare_column` — `LEAD(t) OVER (PARTITION BY k ORDER BY t + 1)`
  refuses as `OrderNotMonotoneClock` (the path that seeds `clock_col_name`).
- `refuses_two_window_calls_in_one_projection` — a projection containing both a `LEAD` and a
  `LAG` call refuses as `WindowFunctionNotLead` (the `_ =>` arm beside the unwrap).

The existing 39 succession unit tests are the behaviour-preservation oracle: all must stay
green with no assertion edits. Any edit to an existing assertion means the refactor changed
semantics and is wrong.

## Tasks

1. Reproduce red: run `bash .claude/scripts/hardening-budget.sh` and record the reported
   `smelt-logical` counts; confirm `2`/`4` and that no other crate has drifted (if another
   crate has drifted, fix or report it — never `--update`).
2. Add the two new unit tests; confirm they pass against the current implementation (they
   pin existing behaviour, so they are green before the refactor — the gate is the red test).
3. Site ~266: replace `match windows.len()` with `if windows.len() > 1 { refuse(...) }`
   followed by `match windows.pop() { Some(w) => window_items.push(...), None => <row-local
   branch> }`, so the single-window case is destructured rather than asserted.
4. Sites ~409/411/445: extract the per-window-item shape check into a helper
   (`fn window_shape(alias, window_call) -> Result<WindowShape, NotSuccessionReason>` returning
   `{ is_lead, partition_cols, order_text, order_expr, arg_col_name }`), then replace the
   `Option`-accumulating loop with `window_items.split_first()`: the `None` arm becomes the
   existing "no LEAD/LAG window projection found" refusal, and the `Some` arm seeds
   `partition_cols` / `order_text` / `order_expr` / `clock_col` as plain non-`Option` values
   from the first item before iterating the rest to compare. The three `expect`s disappear
   because the invariant becomes a type-level fact.
5. Keep refusal ordering and messages byte-identical (the existing tests assert on the reason
   variant; do not renumber or reword).
6. Re-run the gate; confirm `smelt-logical unwrap 1` / `expect 1` with
   `.claude/hardening-baseline.txt` unmodified (`git diff --stat` shows no baseline change).

## Verification

- `bash .claude/scripts/hardening-budget.sh` — exits 0, baseline file untouched.
- `cargo test -p smelt-core --test hardening_budget --quiet`
- `cargo test -p smelt-logical --quiet 2>&1 | tail -40` (39+2 succession tests, `walk_coverage`)
- `bash .claude/scripts/verify-phase.sh` — must now be **fully green**; this phase's exit
  criterion is that no failure remains in it. If one does, it is a new finding for the next
  planner, recorded in the summary rather than papered over.

## Commit message

`fix(smelt-logical): remove the four succession-classifier unwrap/expect sites the hardening ratchet flags`

# LLM-Optimised CLI Diagnostic Mode — `smelt check`

**Date**: 2026-04-05
**Status**: Proposed

## Goal

Expose Smelt's semantic analysis and diagnostic output via a structured CLI mode designed for LLM consumption. This enables Claude Code (and similar tools) to invoke Smelt as a feedback oracle during iterative development — getting type errors, resolution failures, and semantic diagnostics in a form that's cheap to parse and easy to scope.

This is Smelt's equivalent of `rust-analyzer analysis-stats` / `cargo check --message-format json`, but designed from the start with LLM context budgets in mind.

## Motivation

Claude Code currently has no way to verify Smelt correctness other than running transformations end-to-end. A fast, structured diagnostic CLI would let Claude:

- Confirm a change compiles before proceeding
- Get actionable error context without reading source files manually
- Scope analysis to the file(s) being edited rather than loading whole-project output
- Operate within token budget constraints without ad-hoc truncation in skill prompts

This also dogfoods Smelt's own compiler infrastructure and surfaces gaps in error quality early.

## Proposed Interface

```bash
smelt check [OPTIONS] [PATH]
```

### Flags

| Flag | Description |
|---|---|
| `--format json\|text` | Machine-readable vs human-readable output. Default `text`. |
| `--min-severity error\|warn\|info\|hint` | Filter by severity. Default `error` in CI, `warn` interactive. |
| `--scope file\|project` | Limit analysis to a single file or full project graph. Default `project`. |
| `--explain` | Include extended semantic context: inferred types, resolution chain, suggested fixes. Off by default. |
| `--budget-lines N` | Truncate output to N lines, prioritising higher severity. |
| `--no-colour` | Suppress ANSI codes (implied by `--format json`). |

### Exit Codes

| Code | Meaning |
|---|---|
| `0` | Clean |
| `1` | Warnings only (when `--min-severity warn`) |
| `2` | Errors present |
| `3` | Fatal / could not parse input |

## JSON Output Schema

```json
{
  "summary": {
    "error_count": 2,
    "warn_count": 1,
    "files_analysed": 4,
    "duration_ms": 43
  },
  "diagnostics": [
    {
      "severity": "error",
      "code": "E0201",
      "message": "Column 'user_id' not found in relation 'orders'",
      "file": "models/orders_summary.smelt",
      "span": { "line_start": 14, "col_start": 5, "line_end": 14, "col_end": 12 },
      "context": {
        "available_columns": ["order_id", "customer_id", "amount"],
        "relation_defined_at": "models/orders.smelt:1"
      },
      "explain": null
    }
  ]
}
```

`context` is always included — it's cheap and high-value for LLMs. `explain` is populated only with `--explain` and contains prose describing the resolution chain or type mismatch in detail.

## Skill + Eval Plan

The CLI mode is only half the value. The other half is empirically tuning how Claude uses it.

### Skill structure (`smelt-check` skill)

- **When to invoke**: after any model edit, before moving to next file
- **Scope selection heuristic**: `--scope file` for single-file edits, `--scope project` after cross-model refactors
- **Default invocation**: `smelt check --format json --min-severity warn --budget-lines 80`
- **On error**: extract diagnostics, pass to fix loop with file content
- **On clean**: proceed / summarise to user

### Eval harness

Each eval case is a tuple of `(broken smelt file(s), expected clean state)`. The oracle is: does `smelt check` exit 0 after Claude's fix, and does the transformation output match expected?

### Eval dimensions

1. **Diagnostic sufficiency** — does terse output (`--min-severity error`, no `--explain`) give Claude enough to fix the error without reading source? Track fix-on-first-attempt rate.
2. **Explain ROI** — does `--explain` improve first-attempt fix rate, and by how much? Is the token cost worth it per error class?
3. **Scope impact** — does `--scope file` miss cross-model errors that cause Claude to spin? Measure cases where project scope was needed.
4. **Budget sensitivity** — at what `--budget-lines` value does truncation start causing regressions?

Run evals across error classes: column resolution failures, type mismatches, backend-incompatible syntax, missing model references, malformed SQL.

## Implementation Notes

- The check command should reuse the existing compiler pipeline up to the elaboration/planning phase — no need for code generation
- JSON serialisation of diagnostics should be derived from the same internal `Diagnostic` type used by the LSP, avoiding divergence
- `--budget-lines` truncation should be severity-ordered, not file-order — don't let a noisy low-severity file crowd out errors
- Consider a `--watch` mode later (out of scope here) for interactive use in Claude Code sessions that run long

## Success Criteria

- `smelt check --format json` is reliable enough that Claude Code skill prompts can treat exit 0 as ground truth
- Eval suite covers at least 20 distinct error cases across 3+ error classes
- Empirically determined default flags documented in the skill, with rationale from eval results
- Diagnostic JSON schema is stable enough to version — breaking changes are explicit

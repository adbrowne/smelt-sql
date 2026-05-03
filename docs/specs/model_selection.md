---
feature: model_selection
status: experimental
last_reviewed: 2026-05-04
owners: [andrew]
---

# Model Selection

> **What this is.** A normative spec for the selector syntax used by `--select` and `--exclude` flags. Covers parsing rules, graph traversal semantics, union behavior for multiple selectors, and exclusion precedence.

## Surface

### Selector syntax

```
selector = ['+'] method ['+']

method = model_name
       | 'tag:' tag_name
```

| Form | Meaning |
|------|---------|
| `model_name` | Select the named model only |
| `tag:X` | Select all models with tag `X` |
| `+model_name` | Select model + all its upstream dependencies (transitive) |
| `model_name+` | Select model + all its downstream dependents (transitive) |
| `+model_name+` | Select model + all upstream + all downstream |
| `+tag:X` | Select models with tag X + their upstream dependencies |
| `tag:X+` | Select models with tag X + their downstream dependents |
| `+tag:X+` | Select models with tag X + upstream + downstream |

**Errors:** An empty selector, `tag:` with no tag name, or `+` in any position other than prefix/suffix is a parse error. `model++`, `++model`, `+a+b+` are all invalid.

### Flags

| Flag | Short | Available on |
|------|-------|-------------|
| `--select` | `-s` | `run`, `build`, `backbuild`, `diff`, `explain`, `docs generate`, `seed` |
| `--exclude` | `-e` | `run`, `build`, `diff`, `explain` |

Both flags are repeatable. Each instance adds one selector to the set.

## Semantics

### Selection algorithm

1. If no `--select` flags are given: the working set starts as all models in the project.
2. If one or more `--select` flags are given: the working set starts as the union of all matched models (including graph-traversal expansions) across all selectors.
3. For each `--exclude` flag: remove from the working set all models matched by that selector (including graph-traversal expansions).
4. The final working set is executed in topological order.

**Union of selectors.** Multiple `--select` flags are OR'd (union), not AND'd. `--select A --select B` selects models that match A **or** B.

**Exclusion is post-selection.** `--exclude` is applied to the working set after all `--select` expansions are complete. An excluded model is removed even if it was added by a `+` traversal.

**No `--select` = all models.** Omitting `--select` entirely means "all models". There is no implicit default selector.

### Graph traversal

`+` prefix (upstream): includes the target model(s) and every model they transitively depend on, following every `smelt.<path>` reference (whether the resolved entity is a model, seed, or source).

`+` suffix (downstream): includes the target model(s) and every model that transitively depends on them.

`+name+` is the union of both traversal directions starting from `name`.

Seeds are included in upstream traversal when a model references a seed via `smelt.<path>`. Ephemeral models are included in traversal even though they are not materialized.

**No depth limit.** Upstream and downstream traversal are unbounded — they walk the entire reachable subgraph, not just N levels.

### Tag matching

A model matches `tag:X` if tag `X` appears in its effective tag set (the merged union of `smelt.yml` model config tags and frontmatter tags — see `models.md` §"Tag merging" for the merge rule and the case-sensitivity contract).

If no model in the project has the given tag, the selector matches nothing (no error). The resulting working set may be empty.

### Selection methods

The only two selection methods are `ModelName` and `Tag`. There is no glob, regex, path, or directory-based selection.

A `ModelName` selector that names a model not in the project matches nothing (no error). The resulting working set may be empty.

## Design

**`+` as directional modifier, not separate flag.** Upstream and downstream expansion are inline modifiers on each selector rather than separate flags (e.g., `--upstream`, `--downstream`). This lets each selector carry its own traversal intent, which is useful when mixing: `--select +A --select B+` expands A's upstreams and B's downstreams independently before taking the union.

**Union semantics for multiple `--select`.** Multiple `--select` flags add to the set, not narrow it. This mirrors common shell usage: `grep -e pat1 -e pat2` means OR, not AND. Narrowing (intersection) would require a different flag or syntax and is not supported today.

**Exclusion as post-pass.** Applying `--exclude` after the full selection (including traversal) is simpler to reason about than applying it before traversal. The user can say "select X and all its upstreams, except model Y" and get a predictable result regardless of where Y appears in X's dependency tree.

**No glob.** Glob or regex model-name matching is not supported. The two supported methods (name and tag) cover the common cases; tags are the right mechanism for selecting "all models in this area."

## Constraints & Invariants

1. **Selector syntax is strict.** `+` may appear only as a leading prefix or trailing suffix. Any other position is a parse error.
2. **Union of selectors, not intersection.** Multiple `--select` flags produce a union.
3. **Exclusion is applied after all inclusion expansions.** An excluded model cannot be re-included by a separate `--select`.
4. **No-match is not an error.** A selector that matches no models produces no error; the working set may become empty.
5. **Tag matching is case-sensitive.** `tag:Revenue` does not match a model tagged `revenue`. The case-sensitivity contract is owned by `models.md` §"Tag merging"; this rule cross-references it.

## Known Divergences / Open Questions

- **`--select` on `smelt test` is substring match, not selector syntax.** The `--select` flag on `smelt test` does a simple substring match on test names, not full selector syntax. This is inconsistent with all other commands.
- **Seeds in downstream traversal.** Seeds are included in upstream traversal (when a model depends on a seed). It is unclear whether seeds should appear in `smelt explain --select +model` output or be silently filtered. Current behavior undocumented.
- **Selector order and deduplication.** When a model is added by multiple selectors or traversals, it appears once in the working set (deduplicated). The execution order is purely topological; selector order does not affect execution order.
- **`--exclude` with `+` traversal.** `--exclude +model_name` removes `model_name` and all its upstream dependencies. This can remove shared dependencies that other selected models also need, potentially leaving the working set in an inconsistent state. This interaction is untested.

## References

- **Code**:
  - `crates/smelt-core/src/selector.rs` — `Selector`, `SelectionMethod`, `parse_selector()`
  - `crates/smelt-cli/src/logical_graph.rs` — graph traversal applying selectors to the model DAG
- **Tests**:
  - `crates/smelt-core/src/selector.rs` (inline `#[cfg(test)]`) — parser unit tests
- **User docs**:
  - `docs-site/docs/guide/model-selection.md`
- **Related specs**:
  - `cli.md` — which commands accept `--select`/`--exclude`
  - `models.md` — tag definition and merging rules

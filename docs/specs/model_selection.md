---
feature: model_selection
status: experimental
last_reviewed: 2026-06-13
owners: [andrew]
---

# Model Selection

> **What this is.** A normative spec for the selector syntax used by `--select` and `--exclude` flags. Covers parsing rules, graph traversal semantics, union behavior for multiple selectors, and exclusion precedence. Out of scope: which commands accept these flags (see `cli.md`); tag declaration (see `models.md`); generator-file bodies (see `meta_language.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. No plan-phase headings (`### Phase A — …`), no inline phase labels (`Meta list (Phase A)`), no plan-vocabulary status callouts (`[deferred to Phase E1]`) in §Surface, §Semantics, §Design, or §Constraints. Implementation status that needs naming goes in §Known Divergences (describe behaviour, link the plan; phase numbers tolerated only when paired with a plan link) or §References → Plans (history) (link plan files; do not describe their phase structure). See the Timeless-oracle rule in `CLAUDE.md` for the full rule and good/bad examples.

## Surface

### Selector syntax

```
selector = ['+'] method ['+']

method = model_name
       | 'tag:' tag_name
       | 'generator_file:' workspace_relative_path
```

| Form | Meaning |
|------|---------|
| `model_name` | Select the named model only |
| `tag:X` | Select all models with tag `X` |
| `generator_file:path/to/file.gen.sql` | Select all models emitted by the named generator file (per `meta_language.md` §"Multi-model production"). The path is workspace-relative. |
| `+model_name` | Select model + all its upstream dependencies (transitive) |
| `model_name+` | Select model + all its downstream dependents (transitive) |
| `+model_name+` | Select model + all upstream + all downstream |
| `+tag:X` | Select models with tag X + their upstream dependencies |
| `tag:X+` | Select models with tag X + their downstream dependents |
| `+tag:X+` | Select models with tag X + upstream + downstream |
| `+generator_file:path/to/file.gen.sql` | Select generator emissions + their upstream dependencies |
| `generator_file:path/to/file.gen.sql+` | Select generator emissions + their downstream dependents |
| `+generator_file:path/to/file.gen.sql+` | Select generator emissions + upstream + downstream |

**Errors:** An empty selector, `tag:` with no tag name, `generator_file:` with no path, or `+` in any position other than prefix/suffix is a parse error. `model++`, `++model`, `+a+b+` are all invalid. A `generator_file:` selector whose path does not resolve to a workspace generator file matches nothing (no error; the working set may be empty).

### Flags

| Flag | Short | Available on |
|------|-------|-------------|
| `--select` | `-s` | `run`, `build`, `diff`, `explain`, `docs generate`, `seed` |
| `--exclude` | `-e` | `run`, `build`, `diff`, `explain` |

Both flags are repeatable. Each instance adds one selector to the set.

**`smelt backbuild` uses a positional selector.** Unlike the commands above, `backbuild` takes a single positional `<SELECTOR>` argument rather than `--select`/`--exclude` flags, and always expands upstream (the `+` prefix is implicit). There is no `--exclude` support on `backbuild`.

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

The selection methods are `ModelName`, `Tag`, and `GeneratorFile`. There is no glob, regex, or directory-based selection.

A `ModelName` selector value is an entity identifier and is resolved through the CLI argument-resolution algorithm (`cli.md` §"Argument resolution algorithm"): any leading/trailing `+` graph operators are stripped first (re-attached to the resolved full path afterwards), then the value is treated as a dot-path argument, expanded against the active scope (`--scope` or cwd-derived), and matched against the workspace's canonical `smelt.<path>` set. A `ModelName` selector that resolves to **no entity is a hard "not found" error** (non-zero exit), not a silent empty match — the same fail-loud behaviour as a bare command argument, so `--select typo_name` fails loudly rather than building nothing. (A `tag:`/`generator_file:` selector that matches no models is a *valid empty selection*, not an error — see §"Tag matching" and §"Selection methods"; this asymmetry is intentional: an entity name that cannot resolve is almost always a typo, whereas a tag with no members is a legitimate state.) A bare-leaf selector value with no active scope that matches **multiple** entities by leaf is an error (the same ambiguity diagnostic CLI argument resolution emits), so `--select events_parsed` never silently picks one of two `events_parsed` models.

A `GeneratorFile` selector takes a workspace-relative path to a generator file (a `.sql` file whose YAML frontmatter declares `generates: models` per `meta_language.md` §"Multi-model production") and matches every model the file emits during workspace-shape resolution. The match set is computed at selector-evaluation time against the post-W3 workspace shape (per `meta_language.md` §"Multi-model production" rule 4); generator-emitted models that lost a collision (`ModelDefHandAuthoredCollision`) are not in the match set. A `GeneratorFile` selector pointing at a path that is not a generator file (a hand-authored model, a missing file, a non-`.sql` file) matches nothing (no error).

## Design

**`+` as directional modifier, not separate flag.** Upstream and downstream expansion are inline modifiers on each selector rather than separate flags (e.g., `--upstream`, `--downstream`). This lets each selector carry its own traversal intent, which is useful when mixing: `--select +A --select B+` expands A's upstreams and B's downstreams independently before taking the union. Separate `--upstream` / `--downstream` flags were rejected because they apply globally to all selectors — you cannot say "expand upstream for A but downstream for B" without splitting into two command invocations.

**Union semantics for multiple `--select`.** Multiple `--select` flags add to the set, not narrow it. This mirrors common shell usage: `grep -e pat1 -e pat2` means OR, not AND. Intersection semantics were rejected because the most common use case for multiple `--select` is "run these models and those models" — not "run models that match all criteria simultaneously." Users who need intersection can apply `--exclude` as a post-pass.

**Exclusion as post-pass.** Applying `--exclude` after the full selection (including traversal) is simpler to reason about than applying it before traversal. The user can say "select X and all its upstreams, except model Y" and get a predictable result regardless of where Y appears in X's dependency tree.

**No glob.** Glob or regex model-name matching is not supported. The two supported methods (name and tag) cover the common cases; tags are the right mechanism for selecting "all models in this area."

## Constraints & Invariants

1. **Selector syntax is strict.** `+` may appear only as a leading prefix or trailing suffix. Any other position is a parse error.
2. **Union of selectors, not intersection.** Multiple `--select` flags produce a union.
3. **Exclusion is applied after all inclusion expansions.** An excluded model cannot be re-included by a separate `--select`.
4. **No-match is not an error for method selectors.** A `tag:` or `generator_file:` selector that matches no models produces no error; the working set may become empty. A `ModelName` selector that resolves to no entity is the exception — it is a hard "not found" error (§"Selection methods"), because an unresolvable entity name is almost always a typo.
5. **Tag matching is case-sensitive.** `tag:Revenue` does not match a model tagged `revenue`. The case-sensitivity contract is owned by `models.md` §"Tag merging"; this rule cross-references it.
6. **`GeneratorFile` selectors match against the post-resolution workspace shape.** The match set is what the workspace actually contains after multi-model production has resolved, including any collision losses. Generator emissions discarded by `ModelDefHandAuthoredCollision` are not selectable via the originating generator's `generator_file:` path.

## Known Divergences / Open Questions

- **`--select` on `smelt test` selector-syntax rollout.** `smelt test --select` is specified to use the full selector syntax (the same methods and `+` operators as every other command — see `cli.md` §"`smelt test` isolation"). Any remaining substring-match-on-test-names behaviour in the implementation is an unlanded gap, not the intended contract.
- **Seeds in downstream traversal.** Seeds are included in upstream traversal (when a model depends on a seed). It is unclear whether seeds should appear in `smelt explain --select +model` output or be silently filtered. Current behavior undocumented.
- **Selector order and deduplication.** When a model is added by multiple selectors or traversals, it appears once in the working set (deduplicated). The execution order is purely topological; selector order does not affect execution order.
- **`--exclude` with `+` traversal.** `--exclude +model_name` removes `model_name` and all its upstream dependencies. If a removed upstream is still required by a model that remains in the working set, smelt refuses the inconsistent set with a diagnostic rather than running a model against an absent input (see `cli.md` §"`--exclude` and inconsistent working sets"). The user narrows the exclusion (drop the `+`) or also excludes the dependent model.

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

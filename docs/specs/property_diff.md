---
feature: property_diff
status: experimental
last_reviewed: 2026-09-05
owners: [andrew]
---

# Property Diff ("explain the diff")

> **What this is.** The normative spec for the **property diff**: given a project at two
> versions (a baseline git ref and the working tree), report every model whose *derived*
> maintenance properties changed — grain, bound/reach, per-cell technique, refusals, contract
> point, probe set — classify each change as a downgrade, upgrade, or neutral, and attribute it to
> the edited file that caused it. It is surfaced by `smelt explain --diff`, by a CI pull-request
> comment, and in the editor as a code lens and warning diagnostics. Out of scope, with their own
> homes: the derivation of the properties themselves (`model_properties.md`, the composition walk);
> the maintenance plan those properties feed (`incremental_models.md`); the single-version report
> the diff is built from (`cli.md` §"`smelt explain <model>` maintenance-plan report",
> `ui_model_diagnostics.md`); migrating a **stored table** after a definition change
> (`definition_deltas.md` — that compares the working tree to the *deployed snapshot*, this spec
> compares it to a *git ref*, and neither substitutes for the other); output-schema change
> detection (`smelt diff`, `cli.md`; `schema_evolution.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (behaviour + tracking link) or §References → Plans (history).

## Overview

*This section is a non-normative primer. The normative statements live in §Surface, §Semantics,
and §Constraints & Invariants — on any conflict, those win.*

smelt proves things about a model from its SQL: whether it has a grain, how far a run must
reach into each source, which maintenance technique each cell of the model is allowed to use.
Those proofs are what make `refresh: incremental` cheap and correct. They are also fragile in
a way that is invisible at edit time: replacing `SUM` with `MAX`, joining a second source
without a clock, or adding a `DISTINCT` can silently demote a model from a keyed fold to a
full recompute, and can demote every model downstream of it, without a single diagnostic firing —
because the new SQL is perfectly valid, just more expensive to maintain.

The property diff closes that gap. It renders the same per-model **property profile** the
maintenance report already prints, at two versions of the project, and diffs the profiles. The
result is a list of changes, each one tagged with a direction:

```
$ smelt explain --diff

property diff vs merge-base(main) = 3e9c1a4a (2 files changed, 3 models shifted)

  staging/orders                 (edited)
    ▼ cell revenue@orders: technique KeyedFold → DeleteInsert
        reason: SUM(amount) → MAX(amount) — combiner is a monoid, not a group
    ▼ reach orders: bounded(7 days) → unbounded

  marts/order_facts              (downstream of staging/orders)
    ▼ cell net_amount@staging/orders: technique KeyedFold → DeleteInsert
    ▼ refusal added: MaintenanceScanUnbounded

  marts/customer_ltv             (downstream of staging/orders)
    ▼ contract point: default → frozen_horizon: 90 days
    ● column added: net_amount

4 downgrades, 0 upgrades, 1 neutral.
```

The same list feeds a machine-readable `--json` form, a `--markdown` form a CI job posts as one
pull-request comment, and the editor: a code lens above each shifted model's first line
("3 downgrades vs main") and a warning diagnostic on each downgrade.

The baseline is always a **git ref**, defaulting to the merge-base with `main`. The diff never
consults a warehouse, a deployed snapshot, or the maintenance ledger — it is a pure function of
two source trees.

Reading guide: §Surface lists the flags, output forms, and diagnostic codes; §Semantics defines
the profile, the diff, the direction ordering, and attribution; §Design says why the baseline is
materialised through the ordinary loader and why the report is a rendering of the profile.

## Surface

### `smelt explain --diff [<ref>]`

`--diff` is a mode of `smelt explain`. It takes an optional git ref; the default is
`merge-base(HEAD, main)` (or `master` when no `main` branch exists). It compares the project at
that ref against the **working tree** (uncommitted edits included). It prints one entry per
**shifted** model — a model whose property profile at the two versions differs — and a summary
line.

| Flag | Default | Description |
|------|---------|-------------|
| `--diff [<ref>]` | merge-base with `main` | Enables diff mode. `<ref>` is any revision `git rev-parse` accepts. |
| `--json` | off | Emit the diff as JSON (schema below). |
| `--markdown` | off | Emit the diff as GitHub-flavoured Markdown, ready for `gh pr comment --body-file`. Exclusive with `--json`. |
| `--fail-on <direction>` | none | `downgrade` exits `1` when any downgrade is present; `any` exits `1` when any model shifted. Without the flag the exit code is `0` whenever the diff was computed. |
| `--select <selector>` | all models | Restricts the *reported* set, not the compared set: every model is still derived at both versions so that attribution stays correct. |
| `--project-dir <path>` | cwd | The project root. Must be inside a git work tree. |

`--diff` is exclusive with the positional `<model>` argument, `--show-sql`, `--period`, and
`--technique`; combining them is a usage error (exit `2`), never a silently ignored flag.

Exit codes follow `cli.md` §"Exit codes": `0` computed (possibly with shifts), `1` a
`--fail-on` condition held, `2` usage error, including "not a git work tree", "unknown ref",
and "baseline has no `smelt.yml` at this project path".

### Output forms

**Text** (default). One block per shifted model, ordered by dependency order (upstream first),
then by name. Each block is headed by the model name and its **cause**: `(edited)` when the
model's own file changed, `(added)`, `(removed)`, or `(downstream of <model>[, <model>…])`
naming the nearest edited ancestors. Each change line carries a direction glyph — `▼` downgrade,
`▲` upgrade, `●` neutral — the changed dimension, `old → new`, and, where the derivation
exposes one, a one-line **reason** taken verbatim from the property's own refusal or
classification text (never re-derived by the diff). The final line counts downgrades, upgrades,
and neutrals. When nothing shifted the whole output is the single line
`property diff vs <ref>: no models shifted`.

**JSON** (`--json`). Append-stable (`cli.md` §Constraints item 5):

```json
{
  "baseline": { "ref": "<as given>", "commit": "<sha>", "resolved_as": "merge_base" | "explicit" },
  "edited_files": ["<project-relative path>", ...],
  "summary": { "downgrades": 3, "upgrades": 0, "neutral": 1, "shifted_models": 3 },
  "models": [
    {
      "model": "<name>",
      "cause": { "kind": "edited" | "added" | "removed" | "downstream", "of": ["<model>", ...] },
      "changes": [
        {
          "dimension": "grain" | "row_identity" | "source_bound" | "cell_technique"
                     | "cell_corner" | "cell_added" | "cell_removed" | "refusal_added"
                     | "refusal_removed" | "contract_point" | "probe_added" | "probe_removed"
                     | "column_added" | "column_removed" | "determinism" | "discriminant",
          "subject": "<cell group@source | source name | column name | probe fact>",
          "direction": "downgrade" | "upgrade" | "neutral",
          "old": <json value or null>,
          "new": <json value or null>,
          "reason": "<one line>"          // omitted when the derivation exposes none
        }
      ]
    }
  ]
}
```

`old`/`new` are the same JSON encodings the single-version `smelt explain <model> --json` report
uses for that field, so a reader can cross-reference the two without a mapping table. `models` is
ordered as in the text form.

**Markdown** (`--markdown`). A single comment body: a one-line heading with the baseline and the
summary counts, one collapsible `<details>` block per shifted model (open by default when it
contains a downgrade), a table of changes with the same columns as the JSON `changes` entries,
and a trailing HTML comment marker `<!-- smelt-property-diff -->` so a workflow can find and
replace its previous comment instead of stacking new ones.

### Pull-request comment

smelt ships a documented GitHub Actions job (`docs-site/docs/guide/ci.md`) that runs
`smelt explain --diff "$BASE_SHA" --markdown` on the pull request's merge commit and posts or
updates one comment via `gh pr comment`. The job is text, not code: it composes the CLI surface
above and the `gh` CLI, and its only smelt-specific knowledge is the marker comment. This
repository runs the same job over `examples/` on every pull request as its own dogfood, so the
Markdown form is exercised in CI.

### Editor

| Surface | Behaviour |
|---------|-----------|
| Code lens | One lens on the first line of every **shifted** model file: `N downgrades, M upgrades vs <short ref>`. An unshifted model gets no lens. Executing the lens opens the text report for that model in the editor's output channel. |
| Diagnostic | One `PropertyDowngrade` warning per downgrade, anchored at the model's first SQL token when the change has no narrower anchor, or at the SELECT-list item / `FROM` item the change's subject names when it does. The message is the text form's change line plus reason. |
| Hover on the lens | The full per-model block from the text form. |

The editor's baseline is the same default as the CLI (merge-base with `main`), resolved once per
workspace load and re-resolved when `HEAD` or the ref it points at changes (watched files:
`.git/HEAD` and the ref file it names, `lsp.md` §"Watched files"). A workspace that is not a git
work tree, or whose baseline cannot be resolved, shows no lens and no `PropertyDowngrade`
diagnostics — and logs the reason at `info`, never as a user diagnostic (an un-versioned
workspace is not an error).

### Diagnostics

| Code | Severity | Trigger |
|------|----------|---------|
| `PropertyDowngrade` | Warning | A model's property profile at the working tree is worse than at the baseline ref along one dimension (§"Direction"). Editor only; the CLI reports the same fact as a `▼` line and via `--fail-on`. |
| `PropertyDiffBaselineUnavailable` | Error (CLI only, exit `2`) | `--diff` was requested but the project is not inside a git work tree, the ref does not resolve, or the ref has no `smelt.yml` at the project's path. |

Both codes are catalogued in `diagnostics.md`.

## Semantics

### The property profile

A model's **property profile** is the pure, serialisable record of every composition-relevant
verdict the maintenance report prints for that model. It is one value per model per project
version, derived by the same pure functions the report is built from, and it contains exactly:

1. `columns` — output column names in projection order.
2. `grain` — the proven grain key (`PropertyGrain`), and `row_identity` (`RowIdentityVerdict`).
3. `source_bounds` — per upstream source, the `BoundResult` (bound/reach verdict and its
   derived interval or `Unbounded`).
4. `determinism` and `discriminants` per column.
5. `cells` — for a maintained model, each `PlanCell`'s `(group, trigger, corner, admitted
   technique, contract point)`; for an unmaintained model, empty.
6. `refusals` — the set of maintenance admission refusals (`DiagnosticCode` plus the refusal
   text), as the maintenance-plan gate would report them.
7. `probes` — the declared-fact probe set (`fact`, `probe`, `cell`, `cadence`).

The profile omits everything that is a *rendering* rather than a verdict: emitted statements,
technique previews for non-admitted techniques, presentation expressions for decomposed state,
inbound-edge contract prose. The single-version report is a rendering of the profile plus those
extras; the diff never reads the report, only the profile.

### The diff

Given the profile maps `P_old` (baseline) and `P_new` (working tree), keyed by model name:

- A model in `P_new` but not `P_old` is **added**; every profile field is reported as a change
  with `old = null`, and the whole entry has cause `added`.
- A model in `P_old` but not `P_new` is **removed**, symmetrically.
- A model in both with `P_old[m] == P_new[m]` is **unshifted** and is not reported.
- Otherwise the model is **shifted**, and its changes are the per-dimension differences, computed
  field by field with the following matching rules: cells match on `(group, trigger)`, source
  bounds on source name, columns on name, probes on `(fact, cell)`, refusals on `(code, text)`.
  A cell present on one side only is `cell_added`/`cell_removed`; a matched cell whose technique,
  corner, or contract point differs yields one change per differing field.

Renames are not detected: a renamed column or model is a removal plus an addition. This is
fail-loud by construction (§Constraints) — a rename that changes nothing else still surfaces.

### Direction

Every change carries a direction from an explicit, total ordering per dimension. The orderings
are data in `smelt-logical`, not spread across renderers:

| Dimension | Downgrade when | Upgrade when |
|-----------|----------------|--------------|
| `cell_technique` | new technique is lower on the ladder `KeyedFold` ≻ `ColumnScopedMerge` ≻ `InPlaceUpdate` ≻ `PerGroupRecompute` ≻ `DeleteInsert` | higher |
| `cell_added` / `cell_removed` | a cell is removed from a still-maintained model (a column group lost its maintenance route) | a cell is added |
| `source_bound` | `Bounded` → `Unbounded`, or a bounded interval widened | narrower, or `Unbounded` → `Bounded` |
| `grain` | a non-empty grain became empty, or lost a key column | gained a proven grain |
| `row_identity` | `Declared`/`Proven` → `WholeRow` | the reverse |
| `refusal_added` / `refusal_removed` | a refusal appeared | one disappeared |
| `contract_point` | a relaxation appeared or its interval widened (`default` → `frozen_horizon: 90 days`) | a relaxation was removed or narrowed |
| `probe_added` / `probe_removed` | a probe was **removed** (a declared fact lost its runtime check) | one was added |
| `determinism` | a column went from run-deterministic to nondeterministic | the reverse |
| `column_added` / `column_removed` / `discriminant` / `cell_corner` | — | — (always `neutral`) |

The technique ladder above is the maintenance cost ordering: it ranks a technique by how much it
must read and write per run, which is what a downgrade costs the operator. It is a distinct
ordering from the *algebraic* ladder in `incremental_models.md` §"The algebraic maintenance
ladder" (which orders combiners, not techniques) and from the override ladder (which orders
declarations). `PerGroupRecompute` sits above `DeleteInsert` because it writes only affected
groups; `InPlaceUpdate` sits above it because it reads no upstream at all.

A `contract_point` relaxation is a downgrade because it weakens the equivalence invariant the
model promises (`incremental_models.md` §"The contract lattice"); a modeller who intends it
sees a `▼` line, which is the point — the relaxation is never silent.

### Attribution

The **edited set** is the set of model files whose *frontmatter-stripped* SQL text differs
between the two versions, plus files whose `smelt.yml` model override differs, plus source
`.yml` files whose declaration changed. A shifted model whose own file is in the edited set has
cause `edited`. Otherwise its cause is `downstream` and `of` lists the **nearest edited
ancestors**: walking the working tree's dependency graph upward from the model, every edited
model or source reached without passing through another edited node. A shifted model with no
edited ancestor at all — possible only when a project-level `smelt.yml` key changed — has cause
`downstream` with `of: []` and the reason `project configuration changed`.

Attribution uses the working-tree graph. A model whose *dependencies* changed is by definition
in the edited set, so the two graphs agree on every `downstream` edge the attribution follows.

### Baseline materialisation

The baseline project is obtained by exporting the project directory at the ref
(`git archive <ref> -- <project-relative path>`) into a scratch directory, loading it through
the ordinary eager loader (`smelt_core::workspace::load_workspace`), deriving every model's
profile, and deleting the directory. Nothing under `.smelt/` at the baseline is read even if it
is committed: the profile is a function of sources only.

The working tree is loaded exactly as `smelt explain` without `--diff` loads it. In the editor,
open buffers override on-disk contents on the working-tree side only; the baseline side is always
the committed content at the ref.

### Interactions

- **Definition deltas.** `definition_deltas.md` answers "what must change in the warehouse";
  this spec answers "what did my edit do to the proofs". A change can be a definition-delta
  no-op (eclipsed, formatting only) and still be a property downgrade — for example when only a
  `smelt.yml` `contract:` block changed. The two reports never share a baseline and never
  substitute for each other.
- **`smelt diff`.** `smelt diff` (`cli.md`) reports output-*schema* changes against the deployed
  snapshot. The `column_added`/`column_removed` dimensions here are neutral precisely because
  schema change has its own owner; they appear only so a reader can see why a cell was added or
  removed.
- **Salsa purity.** The profile derivation is a pure function over already-loaded workspace data
  and is wrapped by a thin `smelt-db` query on each side; the diff is a pure function over two
  profile maps (`architecture.md` §"Salsa purity rule (analysis)").
- **Project isolation.** In a multi-project workspace the editor computes one diff per project;
  the CLI diffs the project at `--project-dir` only (`architecture.md` §"Project isolation rule").

## Design

**A git ref, not the deployed snapshot, is the baseline.** The question the feature answers is a
review question — "what did this change do?" — and review happens against a branch point, in CI
and in the editor, before anything is deployed. The deployed snapshot answers a different,
per-target question that `definition_deltas.md` already owns. Making the snapshot a second
baseline was considered and deferred (§Future Extensions): it doubles the resolver surface for
a case the migration plan already covers.

**The baseline is materialised and loaded through the ordinary loader.** Three options were
weighed. A temporary git worktree respects the loading-parity rule but leaves state in `.git`
and is awkward from a long-lived editor process. An in-memory overlay (feeding `git show`
output straight into a second Salsa database) is the most elegant but requires a file-provider
abstraction threaded through `load_workspace`, which is the one place eager discovery lives —
a parity-rule change for a first cut. `git archive` into a scratch directory keeps the loader
untouched, materialises only the project subtree, and leaves no repository state behind; its
cost is one extra load, which the editor amortises by caching per resolved baseline commit.

**The single-version report becomes a rendering of the profile.** Before this feature the
maintenance report assembled its verdicts inline. Extracting the profile as a value the report
renders means the diff and the report cannot disagree about what a model's properties are, and
it gives the profile one owner (`smelt-logical`), consistent with maintenance-plan purity
(`architecture.md` §Constraints item 12).

**Direction orderings are data.** Every renderer — text, JSON, Markdown, code lens, diagnostic —
reads the same `Direction` from the same change value. If the technique ladder ever changes,
it changes in one table.

**Whole-project derivation at both versions.** Diffing only edited files was rejected because
the case that matters most for a review gate is the silent downstream downgrade, which is only
visible by deriving the mart at both versions. The cost is bounded by the same derivation
`smelt explain` already performs for every model.

**Reasons are quoted, never re-derived.** A change's `reason` is the refusal or classification
text the property derivation already produced. The diff has no SQL knowledge of its own; it
compares verdicts.

## Constraints & Invariants

1. **Profile single ownership.** The property profile type and its derivation live in
   `smelt-logical`; `smelt-runtime`'s `ModelDiagnostics`, the CLI report, and the editor all
   consume it. No renderer computes a property verdict of its own.
2. **Diff purity.** `diff_profiles(old, new, graph) -> PropertyDiff` is a pure function over
   two profile maps and a dependency graph. It performs no I/O and reads no ledger, snapshot, or
   backend.
3. **Direction totality.** Every `Dimension` has exactly one direction rule, encoded in one
   table; a new dimension without a rule is a compile error, not a `neutral` default.
4. **Report/profile parity.** For every model, the values `smelt explain <model> --json` prints
   for grain, bounds, cells, refusals, contract point, and probes are byte-identical to the
   profile's JSON encoding of the same fields. Standing gate:
   `cargo test -p smelt-cli --test property_profile_parity`.
5. **Surface parity.** The CLI, the Markdown form, and the editor derive from the same
   `PropertyDiff` value; the editor never runs its own comparison. Standing gate:
   `cargo test -p smelt-lsp --test property_diff_parity` (the lens counts and the
   `PropertyDowngrade` set equal the CLI JSON for the same workspace and ref).
6. **Fail-loud.** An unresolvable baseline is an error (exit `2` / logged in the editor), never
   an empty diff. A profile that cannot be derived on one side (a parse error at the baseline,
   say) reports that model as `added`/`removed` with the derivation failure as its reason, never
   as unshifted.
7. **Loading parity.** Both sides load through `load_workspace`; no second discovery path.
8. **No repository mutation.** Diff mode never creates a worktree, stash, index entry, or
   ref. Its only filesystem effect is a scratch directory it deletes.
9. **Append-stable JSON.** New dimensions and fields are added, never renamed or removed
   (`cli.md` §Constraints item 5).

## Limitations

- **Renames are removal plus addition.** Detecting a renamed column or model would require a
  similarity heuristic the fail-loud posture forbids; the reader sees both lines and judges.
- **Working tree only on the new side.** `--diff <a> <b>` between two refs is not supported;
  the second side is always the working tree. Two-ref diffs are trivially composed by a caller
  that checks out `b`.
- **One project per invocation.** The CLI diffs `--project-dir`; a multi-project workspace runs
  it once per project (the CI job loops).

## Known Divergences / Open Questions

- The feature is specified but not yet implemented. Tracking plan:
  `docs/plans/20260905-property-diff.md`.
- Whether `column_added` on a *maintained* model should be neutral (as specified) or should
  inherit the direction of the `cell_added` it usually accompanies is open; the current answer
  keeps it neutral so schema change stays owned by `smelt diff`.

## Future Extensions

- **Deployed-snapshot baseline** (`--diff --against deployed`): the same diff over the profile
  derived from the recorded `model_sql` in the deployed-schema snapshot. Motivated by the
  release-manager question "what will the next run do differently?"; deferred until a concrete
  case shows the migration plan does not already answer it.
- **Cost estimates on downgrades**: annotate a technique downgrade with the row-count delta a
  `smelt bakeoff` measurement would show. Depends on the cost-aware planner direction in
  `docs/research/20260905-ten-directions.md`.

## References

- **Code**: `crates/smelt-logical/src/analysis/profile.rs` (profile + diff), `crates/smelt-cli/src/commands/explain.rs` (`--diff`), `crates/smelt-lsp/src/` (code lens, `PropertyDowngrade`), `crates/smelt-core/src/workspace.rs` (baseline export helper)
- **Tests**: `crates/smelt-cli/tests/property_profile_parity.rs`, `crates/smelt-lsp/tests/property_diff_parity.rs`, `crates/smelt-logical/tests/profile_diff.rs`
- **User docs**: `docs-site/docs/reference/smelt-explain.md`, `docs-site/docs/guide/ci.md`
- **Plans (history)**: `docs/plans/20260905-property-diff.md`
- **Related specs**: `model_properties.md`, `incremental_models.md`, `definition_deltas.md`, `cli.md`, `lsp.md`, `ui_model_diagnostics.md`, `diagnostics.md`, `architecture.md`

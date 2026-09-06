---
feature: property_diff
status: experimental
last_reviewed: 2026-09-06
owners: [andrew]
---

# Property Diff ("explain the diff")

> **What this is.** The normative spec for the **property diff**: given a project at two
> versions (a baseline git ref and the working tree), report every model whose *derived*
> maintenance properties changed — grain, bound/reach, per-cell technique, refusals, contract
> point, probe set — classify each change as a downgrade, upgrade, or neutral, fold the changes
> into the short list of reviewer-facing **stories** (each with a severity and a consequence
> sentence) that every rendered form leads with, and attribute it to the edited file that caused
> it. It is surfaced by `smelt explain --diff`, by a CI pull-request
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
a way that is invisible at edit time: joining a second source without a clock, or adding a
`DISTINCT`, can silently demote a model from a keyed fold to a full recompute, and can demote
every model downstream of it, without a single diagnostic firing — because the new SQL is
perfectly valid, just more expensive to maintain. Swapping a cell's combiner (`SUM` for `MAX`,
say) is the same class of risk, but only bites where invertibility is load-bearing — a
correction cell maintained over a source declared mutable — never over a `NewData` fold on an
append-only source, which never needs to retract a value it already folded.

The property diff closes that gap. It renders the same per-model **property profile** the
maintenance report already prints, at two versions of the project, and diffs the profiles. The
result is a list of **changes**, each one tagged with a direction. A reviewer does not read that
list first, though: the changes are folded into a handful of **stories** per model — one
sentence each, ranked by severity, answering the three questions a reviewer actually has
("is it still maintained incrementally?", "what does a run read now, and how much more?", "what
can smelt no longer guarantee?") — and every rendered form leads with the stories, with the raw
verdict list underneath for cross-reference:

```
$ smelt explain --diff

property diff vs merge-base(main) = 3e9c1a4a (2 files changed, 3 models shifted)

  staging/orders                 (edited)
    [cost] Reads more per run: each run now reads all history of orders (was 7 days).
    [cost] Costlier maintenance: changes from orders are applied to revenue by DeleteInsert instead of KeyedFold.
    verdicts:
      ▼ cell_technique revenue@orders: KeyedFold → DeleteInsert
      ▼ source_bound orders: bounded(7 days) → unbounded

  marts/order_facts              (downstream of staging/orders)
    [risk] Maintenance refused: incremental maintenance requires a bounded reach; the source's reach just became unbounded.
    [cost] Costlier maintenance: changes from staging/orders are applied to net_amount by DeleteInsert instead of KeyedFold.
    verdicts:
      ▼ cell_technique net_amount@staging/orders: KeyedFold → DeleteInsert
      ▼ refusal_added MaintenanceScanUnbounded

  marts/customer_ltv             (downstream of staging/orders)
    [risk] Contract relaxed: customer_ltv moved from default to frozen_horizon: 90 days.
    [info] Schema: adds net_amount.
    verdicts:
      ▼ contract_point customer_ltv: default → frozen_horizon: 90 days
      ● column_added net_amount

3 models shifted · 2 with correctness risks · 2 read more per run
```

The same stories feed a machine-readable `--json` form, a `--markdown` form a CI job posts as one
pull-request comment, and the editor: a code lens above each shifted model's first line
("1 risk, 1 costlier vs main") and a warning diagnostic on each risk or cost story.

The baseline is always a **git ref**, defaulting to the merge-base with `main`. The diff never
consults a warehouse, a deployed snapshot, or the maintenance ledger — it is a pure function of
two source trees.

Reading guide: §Surface lists the flags, output forms, and diagnostic codes; §Semantics defines
the profile, the diff, the direction ordering, the stories, and attribution; §Design says why
the baseline is materialised through the ordinary loader, why the report is a rendering of the
profile, and why stories are a rendering of the changes.

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
| `--select <selector>` | all models | Restricts the *reported* set, not the compared set: every model is still derived at both versions so that attribution stays correct. The summary counts and `--fail-on` are computed over the **reported** set — narrowed by `--select` when given — so the printed counts always match the printed blocks; the compared set (used only for attribution) is unaffected. |
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
naming the nearest edited ancestors. The block then lists the model's **stories**
(§"Stories"), one line each, in severity order: a severity label — `[risk]`, `[cost]`,
`[improved]`, `[info]` — then the story's lead, a colon, and its detail sentence. Under the
stories, a `verdicts:` sub-block lists every raw change: a direction glyph — `▼` downgrade, `▲`
upgrade, `●` neutral — the changed dimension, `old → new`, and, where the derivation exposes
one, a one-line **reason** taken verbatim from the property's own refusal or classification text
(never re-derived by the diff). The final line is the report **headline** (§"Stories"). When
nothing shifted the whole output is the single line `property diff vs <ref>: no models shifted`.

**JSON** (`--json`). Append-stable (`cli.md` §Constraints item 5):

```json
{
  "baseline": { "ref": "<as given>", "commit": "<sha>", "resolved_as": "merge_base" | "explicit" },
  "edited_files": ["<project-relative path>", ...],
  "summary": { "downgrades": 3, "upgrades": 0, "neutral": 1, "shifted_models": 3 },
  "headline": "<one line>",             // the report headline, §"Stories"
  "models": [
    {
      "model": "<name>",
      "cause": {
        "kind": "edited" | "added" | "removed" | "downstream",
        "of": ["<model>", ...],
        "reason": "<one line>"          // omitted except for the `of: []` project-configuration
                                          // case and a derivation failure (see below)
      },
      "stories": [                      // severity order; every change index appears in exactly one story
        {
          "kind": "maintenance_lost" | "maintenance_gained" | "refusal" | "rows_may_duplicate"
                | "row_key" | "reads" | "dependency" | "technique" | "contract" | "probe"
                | "column_semantics" | "schema" | "other",
          "severity": "risk" | "cost" | "improvement" | "info",
          "subject": "<column name | source name | cell | empty>",
          "lead": "<short phrase>",
          "detail": "<one sentence>",
          "changes": [<index into changes>, ...]
        }
      ],
      "changes": [
        {
          "dimension": "grain" | "row_identity" | "source_bound" | "cell_technique"
                     | "cell_corner" | "cell_row_identity" | "cell_added" | "cell_removed"
                     | "refusal_added" | "refusal_removed" | "contract_point" | "probe_added"
                     | "probe_removed" | "column_added" | "column_removed" | "determinism"
                     | "discriminant" | "comparability" | "fd_added" | "fd_removed"
                     | "literal_column" | "set_op_barrier" | "fan_out_join"
                     | "maintenance_lost" | "maintenance_gained" | "state_downgrade",
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

**Markdown** (`--markdown`). A single comment body: a one-line heading naming the baseline
(`### property diff vs <ref> @ <short commit>`), the **headline** in bold on the next line, one
block per shifted model, and a trailing HTML comment marker `<!-- smelt-property-diff -->` so a
workflow can find and replace its previous comment instead of stacking new ones. The marker and
the heading are emitted **even when nothing shifted**: the body is the same `property diff vs
<ref>: no models shifted` line the text form prints, followed by the marker. This is what lets
the documented job (below) update a stale downgrade comment to the cleared state once the
regression is fixed, instead of leaving it standing after the code it warned about no longer
exists.

A model block opens with the model name in bold and its cause in parentheses (`**<model name>**
(<cause>)`, with `<cause>` the same string the text form's block header uses), then one bullet
per story in severity order: a severity glyph — 🔴 `risk`, ⚠️ `cost`, 🟢 `improvement`, ℹ️
`info` — the story's lead in bold with a trailing full stop, and its detail sentence. The raw
changes follow inside one collapsed `<details>` block titled `Verdict table`: a table whose
columns are the JSON `changes` entry fields (`dimension`, `subject`, `direction`, `old`, `new`,
`reason`). The stories are always visible; the table is always collapsed — a reviewer who wants
the verdicts opens it, and a reviewer who does not is never shown a JSON blob.

Every value in the Markdown form is produced by the same primitives the text form uses — the
story lead and detail, the severity label, the humanised durations, the `old`/`new` display —
never re-spelled for the comment.

GitHub rejects an issue comment body over 65,536 characters. The rendered Markdown body is
bounded to keep any diff, however large, postable: it renders at most the first 50 shifted
models in full, and lists any remainder by name only inside one final `<details>` block
(`… and N more shifted models`). The marker is always the last line, so the workflow can still
find and update the comment on a capped body. The cap applies to the rendered Markdown body
only — it never changes `summary`, `--fail-on`, or the JSON form, all of which report every
shifted model regardless of size.

### Pull-request comment

smelt ships a documented GitHub Actions job (`docs-site/docs/guide/ci.md`) that runs
`smelt explain --diff "$BASE_SHA" --markdown` on the pull request's merge commit and posts or
updates one comment via `gh pr comment`/`gh api`. The comment it updates is found among **that
pull request's own** comments: the marker identifies which of a PR's comments is the property
diff's, never which pull request a repository-wide comment listing belongs to. The job is text,
not code: it composes the CLI surface above and the `gh` CLI, and its only smelt-specific
knowledge is the marker comment. This
repository runs the same job over `examples/` on every pull request as its own dogfood, so the
Markdown form is exercised in CI.

A `pull_request` event triggered from a fork receives a read-only `GITHUB_TOKEN`, so the
documented job always writes the rendered body to the job summary (which needs no write
permission) and posts or updates the PR comment only when the head repository is the base
repository — the same fork guard `gh` write operations need everywhere in this repository's
workflows. The job does not gate the build by default: `--fail-on` is documented as an opt-in a
user's own project can add, with the tradeoff that a repository whose tracked examples shift
routinely will find a default-on gate goes red on legitimate work and gets bypassed, defeating
the signal it exists for. This repository's own dogfood job runs without `--fail-on` for exactly
that reason: its build-failing assertion is that `smelt explain --diff --markdown` exits `0` — a
broken renderer or an example that fails to derive breaks the job; a legitimate property shift
does not.

### Editor

| Surface | Behaviour |
|---------|-----------|
| Code lens | One lens on the first line of every **shifted** model file: the model's **lens title** (§"Stories" — `N risks, M costlier vs <short ref>`, zero terms omitted; `changed vs <short ref>` when it has neither). An unshifted model gets no lens. Executing the lens opens the text report for that model in the editor's output channel. |
| Diagnostic | One `PropertyDowngrade` warning per story of severity `risk` or `cost`, anchored at the model's first SQL token when the story has no narrower anchor, or at the SELECT-list item / `FROM` item the story's subject names when it does. The message is the story's lead, a colon, and its detail — the same line the text form prints. |
| Hover on the lens | The full per-model block from the text form (stories, then verdicts). |

The editor's baseline is the same default as the CLI (merge-base with `main`), resolved once per
workspace load and re-resolved when `HEAD` or the ref it points at changes (watched files:
`.git/HEAD` and the ref file it names, `lsp.md` §"Watched files"). A workspace that is not a git
work tree, or whose baseline cannot be resolved, shows no lens and no `PropertyDowngrade`
diagnostics — and logs the reason at `info`, never as a user diagnostic (an un-versioned
workspace is not an error).

The lens's `<short ref>` is the baseline's `ref` string, except when that string is a full 40-hex
commit sha, in which case its 7-character abbreviation.

The diff is recomputed on workspace load, on a model file being saved or changed outside the
editor, and when the resolved baseline commit changes — never on every keystroke. Open buffers
override on-disk contents for **model files** on the working-tree side; an unsaved `smelt.yml` or
source-YAML buffer takes effect only once it is saved.

While a derivation is running, the editor shows the previously computed diff if one exists, and
nothing at all on first load. It never shows a half-computed diff.

A story's anchor is only as narrow as its `subject` supports: a story whose `subject` names a
column anchors at that SELECT-list item (its alias when aliased, the whole item otherwise); a
story whose `subject` names a source or upstream model anchors at that `FROM`/`JOIN` item; every
other story — a cell subject, a refusal, or a whole-model story with no subject at all — has no
narrower anchor available and anchors at the model's first SQL token.

### Diagnostics

| Code | Severity | Trigger |
|------|----------|---------|
| `PropertyDowngrade` | Warning | A model's property profile at the working tree is worse than at the baseline ref: one warning per `risk` or `cost` story (§"Stories"), each of which folds at least one downgrade (§"Direction"). Editor only; the CLI reports the same fact as the story line, as `▼` verdict lines, and via `--fail-on`. |
| `PropertyDiffBaselineUnavailable` | Error (CLI only, exit `2`) | `--diff` was requested but the project is not inside a git work tree, the ref does not resolve, or the ref has no `smelt.yml` at the project's path. |

Both codes are catalogued in `diagnostics.md`.

## Semantics

### The property profile

A model's **property profile** is the pure, serialisable record of every composition-relevant
verdict the maintenance report prints for that model. It is one value per model per project
version, derived by the same pure functions the report is built from, and it contains exactly:

1. `properties` — the model's derived property set (`PropertySet`): output columns in
   projection order, the proven grain key (`Grain`) and `row_identity` (`RowIdentityVerdict`),
   per-upstream-source `source_bounds` (bound/reach verdict and its derived interval or
   `Unbounded`), and per-column `determinism`, `comparability`, and `discriminants`. These
   verdicts are derived together, by one pure function, and are never split back into separate
   profile fields — doing so would fork the derivation.
2. `cell_verdicts` — for a maintained model, each `PlanCell`'s `(group, trigger, trigger source,
   corner, admitted technique, partition locality, row identity, contract point, state
   downgrade)`; for an unmaintained model, empty. The trigger source is the source name a
   `NewData`/`UpstreamMutation` trigger carries (absent for `Backfill` and `ColumnAdded`),
   exposed structurally so a story can name it without parsing the trigger's display string.
   Partition locality is the cell's `PartitionLocal` verdict: local, or not local with the
   source and the reason text the plan derived (`incremental_models.md` §"Partition-local maintenance (the K8 guardrail)") — the reason is what a `dependency` story quotes when a new source is read in
   full on every run.
   A cell's state downgrade (`incremental_models.md`, `state.md` §"The degradation contract") is
   present when the technique the plan actually admits was forced down from a cheaper one because
   a state structure the cheaper technique needs has no realisation on the target — a state
   downgrade is by definition a downgrade, and the diff must see it appear or disappear the same
   way it sees a `cell_technique` change. Named
   `cell_verdicts` rather than `cells` because the model-diagnostics response
   (`ui_model_diagnostics.md` §Surface) already carries an unrelated `cells` key (the
   technique-preview set) beside the flattened profile. A cell's contract point carries, in
   addition to its display strings, the `frozen_horizon`/`deferral` windows as a machine-comparable
   number of seconds — the interval a `contract_point` diff widens or narrows is decided from
   these seconds, never by re-parsing the display string.
3. `refusals` — the set of maintenance admission refusals (the diagnostic code's name — absent for
   a refusal that raises no diagnostic today — plus the refusal text), as the maintenance-plan
   gate would report them.
4. `probes` — the declared-fact probe set (`fact`, `probe`, `cell`).

The profile omits everything that is a *rendering* rather than a verdict: emitted statements,
technique previews for non-admitted techniques, presentation expressions for decomposed state,
inbound-edge contract prose. The single-version report is a rendering of the profile plus those
extras; the diff never reads the report, only the profile.

### The diff

Given the profile maps `P_old` (baseline) and `P_new` (working tree), keyed by model name:

- A model in `P_new` but not `P_old` is **added**; every profile field is reported as a change
  with `old = null`, and the whole entry has cause `added`. Every such change is graded `neutral`
  regardless of its dimension's ordinary rule — a per-dimension direction is noise for a model
  that is wholly new or gone; the `cause` already says so, and grading it would inflate the
  summary counts and `--fail-on` with a signal the dimension's rule was never meant to answer for
  this case. When the model is absent from one side not because it is genuinely new or deleted
  but because its profile could not be *derived* on that side (§Constraints item 6), the entry
  still carries cause `added`/`removed` as above, but `cause.reason` carries the derivation
  failure text verbatim, so a reader can distinguish "this model doesn't exist here" from "this
  model's SQL doesn't derive here".
- A model in `P_old` but not `P_new` is **removed**, symmetrically (also all `neutral`).
- A model in both with `P_old[m] == P_new[m]` is **unshifted** and is not reported.
- Otherwise the model is **shifted**, and its changes are the per-dimension differences, computed
  field by field with the following matching rules: cells match on `(group, trigger)`, source
  bounds on source name, columns on name, probes on `(fact, cell)`, refusals on `(code, text)`.
  A refusal's `code` is absent for the three admission refusals that raise no diagnostic today
  (§"The property profile" item 3), so the matching key is actually `(code: Option<string>, text)`:
  a `None`-coded refusal matches another `None`-coded refusal with the same `text`, and never
  matches a `Some(_)`-coded one even with identical text — collapsing the three onto one shared
  placeholder key would hide a refusal changing kind. In JSON, a `refusal_added`/`refusal_removed`
  change's `old`/`new` for one of these three carries `code: null`.
  A cell present on one side only is `cell_added`/`cell_removed`; a matched cell whose technique,
  corner, row identity, or contract point differs yields one change per differing field.
- Every field of the profile that is not already named above as a matched-cell field is still
  covered by its own dimension, so that a model can never be reported shifted with an empty
  `changes` array (§Constraints item 6): a `PropertySet`'s functional dependencies (`fd_added`/
  `fd_removed`, matched on `(key, determines)`), comparability (`comparability`, per column),
  literal columns (`literal_column`, matched on column name), set-operation barrier and fan-out
  join (`set_op_barrier`, `fan_out_join`, whole-model booleans), and a cell's row identity
  (`cell_row_identity`) all produce a change whenever they differ.
- A model going from having at least one maintained cell to having none at all — no longer
  incrementally maintained — is reported once, as `maintenance_lost`, never only as N individual
  `cell_removed` changes (which stay `cell_added`/`cell_removed`'s ordinary per-cell dimension,
  graded `neutral` in this case — see §Direction). The symmetric case (no cells to at least one)
  is `maintenance_gained`. This is deliberate: some admission paths (a `refresh: incremental` →
  `refresh: full` edit, in particular) produce an empty cell list with **no** refusal at all, so
  without a dedicated dimension the model's most consequential possible change — losing
  incremental maintenance entirely — could surface as a page of neutral lines with zero
  downgrades.

Renames are not detected: a renamed column or model is a removal plus an addition. This is
fail-loud by construction (§Constraints) — a rename that changes nothing else still surfaces.

### Direction

Every change carries a direction from an explicit, total ordering per dimension. The orderings
are data in `smelt-logical`, not spread across renderers:

| Dimension | Downgrade when | Upgrade when |
|-----------|----------------|--------------|
| `cell_technique` | new technique is lower on the ladder `KeyedFold` ≻ `ColumnScopedMerge` ≻ `InPlaceUpdate` ≻ `PerGroupRecompute` ≻ `DeleteInsert` | higher |
| `cell_added` / `cell_removed` | a cell whose maintenance is **not partition-local** is added to a still-maintained model (its trigger source is now read in full on every run, and any change to that source rebuilds the whole model); or a cell is removed from a still-maintained model while another surviving cell still reads the same trigger source (a column group lost its maintenance route for a source the model still depends on) | — (never: a model *becoming* maintained is `maintenance_gained`'s upgrade, and a new partition-local cell or a cell removed because its source was dropped altogether is `neutral` — a dependency the model gained or shed, reported by the `dependency` story, not a proof that got better) |
| `maintenance_lost` / `maintenance_gained` | the model's cell list went from non-empty to empty — no cell survived at all, so it is no longer incrementally maintained | empty to non-empty (maintenance regained) |
| `state_downgrade` | a cell's state downgrade appeared (the target gained a technique it can no longer realise the required state structure for) | a cell's state downgrade disappeared |
| `source_bound` | `Bounded` → `Unbounded`, or a bounded interval widened (`before + after` grew) | narrower, or `Unbounded` → `Bounded`. `NotDerivable` orders with `Unbounded`: `Bounded` ≻ `{Unbounded, NotDerivable}`, and a change between `Unbounded` and `NotDerivable` in either direction is `neutral` — both force a full read, so neither is worse than the other |
| `grain` / `row_identity` / `cell_row_identity` | a keyed grain became unkeyed, or the grain **widened** — the set of columns its keys cover is a strict superset of the old set, a weaker uniqueness claim (rows are no longer unique per the old key); a row identity moved `Key` → `WholeRow` | an unkeyed grain became keyed, or the grain **narrowed** (strict subset — a stronger uniqueness claim); `WholeRow` → `Key`. A grain whose column set changed without being a superset or subset of the old one is `neutral` (a different key, surfaced by the `row_key` story) |
| `refusal_added` / `refusal_removed` | a refusal appeared | one disappeared |
| `contract_point` | a relaxation appeared or its interval widened (`default` → `frozen_horizon: 90 days`, using the profile's machine-comparable seconds, never the display string), or `retain_departed` went from absent to present (per `EffectiveContract::is_default`) | a relaxation was removed or narrowed, or `retain_departed` went from present to absent. A change of `retain_departed`'s *shape* only (`Bool` → `Tombstone`, or a different tombstone column, presence unchanged) is `neutral` — there is no interval to widen |
| `probe_added` / `probe_removed` | a probe was **removed** (a declared fact lost its runtime check) | one was added |
| `determinism` | a column moved up the lattice `Clean < Run < Row` | moved down it |
| `comparability` | a column moved `Comparable` → `Incomparable` | `Incomparable` → `Comparable` |
| `set_op_barrier` / `fan_out_join` | the flag went `false` → `true` (a new FD/keying barrier appeared) | `true` → `false` |
| `column_added` / `column_removed` / `discriminant` / `cell_corner` / `fd_added` / `fd_removed` / `literal_column` | — | — (always `neutral`) |

The technique ladder above is the maintenance cost ordering: it ranks a technique by how much it
must read and write per run, which is what a downgrade costs the operator. It is a distinct
ordering from the *algebraic* ladder in `incremental_models.md` §"The algebraic maintenance
ladder" (which orders combiners, not techniques) and from the override ladder (which orders
declarations). `PerGroupRecompute` sits above `DeleteInsert` because it writes only affected
groups; `InPlaceUpdate` sits above it because it reads no upstream at all.

A `contract_point` relaxation is a downgrade because it weakens the equivalence invariant the
model promises (`incremental_models.md` §"The contract lattice"); a modeller who intends it
sees a `▼` line, which is the point — the relaxation is never silent.

A `cell_technique` change's `old`/`new` are the technique's name, unchanged from the
single-version report's own rendering of that field — a reader cross-referencing the two never
sees two different spellings for the same technique.

A grain that widens is a downgrade because a proof only ever gets weaker by needing more
columns: proving one row per `(date, user)` implies one row per `(date, user, name)`, never the
reverse, and every downstream join written against the old key may now fan out. The narrowing
case is the mirror image and is an upgrade for the same reason.

### Stories

A shifted model's changes are folded into **stories**: the short list of sentences a reviewer
reads first. A story is a rendering of the changes, not a derivation of its own: it is computed
by one pure function in `smelt-logical` from the model's `changes` alone, quotes the verdicts it
folds (the reason texts, the technique names, the bound intervals), and never consults SQL
(§Constraints item 10). Every change belongs to exactly one story; a change no rule claims lands
in the model's single `other` story, never dropped (§Constraints item 11).

A story carries a `kind`, a `severity`, a `subject` (the column, source, or cell the story is
about, empty for a whole-model story — the editor's anchor), a short `lead`, a one-sentence
`detail`, and the indices of the changes it folds.

**Severity** is derived from the folded changes' directions and a dimension class, never
assigned by a rule directly. The **guarantee dimensions** are `maintenance_lost`, `grain`,
`row_identity`, `cell_row_identity`, `fan_out_join`, `set_op_barrier`, `refusal_added`,
`contract_point`, `probe_removed`, `determinism`, and `comparability`; the **cost dimensions**
are `source_bound`, `cell_added`, `cell_removed`, `cell_technique`, and `state_downgrade`. A
story that folds at least one downgrade in a guarantee dimension is `risk`; one that folds at
least one downgrade, none of them in a guarantee dimension, is `cost`; one that folds at least
one upgrade and no downgrade is `improvement`; anything else is `info`. Consequently every
downgrade is folded by exactly one `risk` or `cost` story and every `risk`/`cost` story folds a
downgrade, so `--fail-on downgrade`, the `PropertyDowngrade` set, and the risk/cost stories can
never disagree.

**Folding rules** are applied in the order below; the first rule that claims a change owns it.
Stories are then ordered by severity (`risk`, `cost`, `improvement`, `info`), then by rule order.

| Kind | Claims | Lead / detail |
|------|--------|---------------|
| `maintenance_lost` | `maintenance_lost`; every `cell_removed`; every `refusal_added` | *No longer incrementally maintained* / "Every run rebuilds the whole table." followed by " Reason: <refusal text>." for each folded refusal |
| `maintenance_gained` | `maintenance_gained`; every `cell_added`; every `refusal_removed` | *Now incrementally maintained* / "<n> maintenance cell(s) admitted." |
| `refusal` | one story per remaining `refusal_added` / `refusal_removed` | *Maintenance refused* / "<refusal text>."; *Refusal cleared* / "<refusal text>." |
| `rows_may_duplicate` | when `fan_out_join` went `false` → `true` or `row_identity` went `Key` → `WholeRow`: both of those, `grain`, every `cell_row_identity`, every `fd_added` / `fd_removed` | *Rows may be duplicated* / "A join can now match more than one row per (<old key>), so smelt can no longer identify a row by its key." |
| `row_key` | `grain`; `row_identity`; every `cell_row_identity`; every `fd_added` / `fd_removed` | keyed → unkeyed: *Row key lost* / "No longer proves one row per (<old key>)."; unkeyed → keyed: *Row key proven* / "Now proves one row per (<new key>)."; widened: *Row key widened* / "Rows are now unique per (<new key>), no longer per (<old key>); downstream joins on the old key may fan out."; narrowed: *Row key narrowed* / "Rows are now unique per (<new key>), a stronger claim than (<old key>)."; otherwise *Row key changed* / "Was (<old key>), now (<new key>)." |
| `dependency` | for each trigger source that appears only in `cell_added` changes, or only in `cell_removed` changes: those cells, plus that source's `source_bound` change when it too is one-sided | added, not partition-local: *New dependency read in full* / "<source> has no time column, so every run reads all of it, and any change to it rebuilds the whole model."; added, partition-local: *New dependency* / "Changes to <source> are applied by <technique>."; removed: *Dependency removed* / "No longer reads <source>."; a `cell_removed` graded downgrade (its source survives): *Maintenance route lost* / "<group> no longer has a maintenance route for changes from <source>." |
| `reads` | every remaining `source_bound`, one story per distinct (`old`, `new`) pair, listing the sources that share it | widened or `Bounded` → unbounded: *Reads more per run* / "Each run now reads <new window> of <sources> (was <old window>)."; the mirror: *Reads less per run* / same shape. A window renders as "all history" for `Unbounded`/`NotDerivable`, "<d> either side of the run window" when `before == after`, and "<d> before and <d'> after the run window" otherwise |
| `technique` | one story per `cell_technique`, plus that cell's `state_downgrade` and `cell_corner` | *Costlier maintenance* / *Cheaper maintenance* / "Changes from <source> are applied to <group> by <new> instead of <old>." followed by " Reason: <state downgrade reason>." when one is folded |
| `contract` | one story per `contract_point` | *Contract relaxed* / *Contract tightened* / *Contract changed* / "<cell> moved from <old display> to <new display>." |
| `probe` | one story per `probe_added` / `probe_removed` | *Runtime check removed* / "Declared fact <fact> is no longer checked at run time."; *Runtime check added* / "Declared fact <fact> is now checked at run time." |
| `column_semantics` | `determinism` / `comparability` on a column present on both sides | *Column now nondeterministic* / "<column> is now <Run\|Row>-nondeterministic (was <old>)."; *Column now deterministic*; *Column no longer comparable* / "<column> can no longer be compared between runs."; *Column now comparable* |
| `schema` | every `column_added` / `column_removed`, plus each such column's `determinism`, `comparability`, `discriminant`, and `literal_column` | *Schema* / "Adds <columns>; removes <columns>." followed by " (<column> is <Run\|Row>-nondeterministic.)" for each added column that is not `Clean` |
| `other` | everything unclaimed, one story per model | *Also changed* / "<dimension> <subject>; <dimension> <subject>; …" |

Durations in a story are humanised in one place: a whole number of days renders as `N days`,
else whole hours, else whole minutes, else seconds, singular when `N == 1`. Column lists render
comma-separated in projection order; the key in a `row_key` story is the columns its grain's
keys cover, in the profile's order.

**Headline.** The report's one-line summary: `<n> model(s) shifted`, then, joined by ` · `, each
clause that applies — `<k> lost incremental maintenance` (models with a `maintenance_lost`
story), `<k> with correctness risks` (models with any other `risk` story), `<k> read more per
run` (models with a `cost` story), `<k> improved` (models with an `improvement` story and no
`risk`/`cost` story) — and `no downgrades` when `summary.downgrades` is zero. Counts are over the
reported set, like `summary`.

**Lens title.** Per model: `<r> risk(s), <c> costlier vs <short ref>`, with a zero term omitted;
`changed vs <short ref>` when the model has neither. `<r>` and `<c>` count the model's `risk`
and `cost` stories.

### Attribution

The **edited set** is the set of model files whose *frontmatter-stripped* SQL text **or parsed
frontmatter metadata** differs between the two versions, plus files whose `smelt.yml` model
override differs, plus source `.yml` files whose declaration changed. Frontmatter is stripped to
blank lines before the SQL comparison, so a frontmatter edit is invisible to that half of the
predicate unless it happens to change a line's byte length; comparing the parsed metadata
directly closes that gap — a `unique_key:`/`refresh:`/`grain:`/`contract:` edit is itself an edit
to the model, not a downstream effect of something else. A shifted model whose own file is in the edited set has
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
- **Salsa purity.** The per-model maintenance-plan derivation the profile is built from stays a
  thin `smelt-db` query (`maintenance_plan_report`) over pure `smelt-logical` functions,
  consistent with `architecture.md` §"Salsa purity rule (analysis)". The profile itself is
  assembled from that report by a single-owner builder in `smelt-runtime`
  (`build_model_diagnostics`), which both the single-version report and the diff call — there is
  no second assembly path. The diff is a pure function over two profile maps.
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

**Stories are a rendering of the changes, not a second diff.** Presenting the verdict table
alone — one row per changed profile field, `old`/`new` as JSON, a direction word — was
rejected: a two-line SQL edit yields six rows a reviewer cannot act on, because
`source_bound … P1D → P7D` states a fact about the proof and leaves the consequence ("every run
reads 7 days either side of its window instead of 1") to the reader, and a widened row key
yields twelve rows (grain, row identity, and every functional dependency removed and re-added
under the new key) for what is one sentence. Two further alternatives were rejected. Deriving
consequences inside `diff_profiles` would
give the diff SQL knowledge it is deliberately denied (§Design "Reasons are quoted"). Letting
each surface compose its own sentences would fork the wording three ways (CLI, comment,
editor) and break surface parity. Stories are therefore a pure fold over the change list in
`smelt-logical`, single-owned like the direction table, and every surface renders the same
`Story` values. The verdict table survives, collapsed, because it is the cross-reference to
`smelt explain <model>` that the JSON form's `old`/`new` encodings exist for.

**Severity is derived from direction, never assigned.** A story rule could have carried its own
severity, but then a rule and the direction table could disagree — a `risk` story with no
downgrade behind it, or a downgrade no story warns about — and `--fail-on`, the editor's
diagnostics, and the comment would drift apart. Deriving severity from the folded changes'
directions and a fixed dimension class keeps the three surfaces provably consistent
(§Constraints item 11).

**A new dependency is a cost, not an upgrade.** Grading every `cell_added` an upgrade, on the
reasoning that a model with more maintenance cells is more maintained, was rejected. The common
way a cell appears is a new `JOIN`, and when the joined source has no clock the new cell reads
that source in full on every run and rebuilds the whole model whenever it changes — the exact
silent cost the feature exists to surface, and a case a `--fail-on downgrade` gate would wave
through under that grading. The table grades that case a downgrade, a partition-local new cell
neutral, and reserves the upgrade for `maintenance_gained`. Symmetrically, a cell that
disappears because its source was dropped is neutral rather than a "lost route".

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
10. **Narration single ownership.** Stories, their severities, the headline, and the lens title
    are produced by one pure function in `smelt-logical` over a shifted model's `changes`; the
    text, Markdown, JSON, lens, hover, and diagnostic surfaces render those values and never
    compose a sentence, humanise a duration, or grade a severity of their own. The story
    function has no SQL knowledge: it reads change values and quotes reason texts.
11. **Story coverage totality.** Every change of a shifted model is folded by exactly one story;
    a story of severity `risk` or `cost` folds at least one downgrade; every downgrade is folded
    by a `risk` or `cost` story. Standing gate: a property test over generated change lists,
    `cargo test -p smelt-logical --test story_coverage`, asserts all three and that
    `narrate` never panics on any combination of dimensions.

## Limitations

- **Renames are removal plus addition.** Detecting a renamed column or model would require a
  similarity heuristic the fail-loud posture forbids; the reader sees both lines and judges.
- **Working tree only on the new side.** `--diff <a> <b>` between two refs is not supported;
  the second side is always the working tree. Two-ref diffs are trivially composed by a caller
  that checks out `b`.
- **One project per invocation.** The CLI diffs `--project-dir`; a multi-project workspace runs
  it once per project (the CI job loops).

## Known Divergences / Open Questions

- Story sentences are English templates with no localisation hook; whether the `lead`/`detail`
  split is enough for a downstream tool to re-word them, or a message catalogue is needed, is
  open. Tracked in `docs/plans/20260906-property-diff-stories.md`.
- Whether `column_added` on a *maintained* model should be neutral (as specified) or should
  inherit the direction of the `cell_added` it usually accompanies is open; the current answer
  keeps it neutral so schema change stays owned by `smelt diff`.
- Three admission refusals (`ReachNotDerivable`, `RepairKeysNotDiscoverable`,
  `RepairSliceUnbounded`) raise no diagnostic through the ordinary diagnostics pipeline today, so
  their profile `code` is absent. Whether each deserves its own `DiagnosticCode` catalogue entry
  is open; tracked in `docs/outcomes/20260905-property-diff/outcome.md`.
- A source file with no `smelt.` marker that is not actually a model definition (a DDL or setup
  script, say) is classified as `EntityKind::Model` by the resolver and genuinely fails
  `PropertySet::derive`. Today this is harmless: it lands in the derivation-failure set
  symmetrically on both sides of a diff, never enters `diff.models`, and never reaches
  user-facing output. It would stop being harmless if such a file were edited on only one side of
  a diff — that would surface as a spurious `added`/`removed` entry carrying the derivation
  failure text as its reason, rather than being recognised as "not a model." Tracked in
  `docs/outcomes/20260905-property-diff/outcome.md`.
- The editor treats every baseline-resolution failure (not a git work tree, no `main`/`master` to
  default against, an unresolvable ref, git itself unavailable) uniformly as the "no lens, no
  diagnostic, logged at info" case. A narrower treatment — distinguishing "genuinely not
  versioned" from "git is broken in a way the user should probably know about" — is possible but
  not yet made; both still satisfy "never a user diagnostic for an unresolved baseline." Tracked
  in `docs/outcomes/20260905-property-diff/outcome.md`.
- Executing the lens opens the text report for that model in the editor's output channel, but no
  editor extension yet registers the command the lens emits, so today executing it is a no-op in
  every editor. Tracked in `docs/outcomes/20260905-property-diff/outcome.md`.
- Hovering the lens shows nothing: no hover provider serves the per-model block the Editor table
  promises, so the stories-then-verdicts block is reachable today only through `smelt explain
  --diff`. Tracked in `docs/outcomes/20260905-property-diff/outcome.md`.
- `CellVerdict.state_downgrade` has a live producer only for a model whose target dialect makes
  it fire; no model in `examples/timeseries` or `examples/retail_analytics` exercises it, so the
  diff's `state_downgrade` dimension is proven only by a dual-target unit fixture, not by an
  example workspace. Tracked in `docs/outcomes/20260905-property-diff/outcome.md`.
- `examples/timeseries` has no fixture demonstrating a *combiner-driven* downgrade (swapping a
  cell's combiner without otherwise changing row identity). Its own combiner-sensitive cells are
  `NewData` folds over append-only sources, which never need an invertible combiner (see the
  outcome's Decision log); a fixture demonstrating this case needs a model whose driving source
  is declared mutable. Tracked in `docs/outcomes/20260905-property-diff/outcome.md`.
- The LSP refresh coalescer's `pending`-trailing-rerun path (a refresh trigger that arrives while
  a refresh is already running schedules exactly one more pass on completion) has no test that
  reliably induces the race — the coalescing gate covers the same-notification burst case, not
  the cross-notification-concurrent-trigger case. Tracked in
  `docs/outcomes/20260905-property-diff/outcome.md`.

## Future Extensions

- **Deployed-snapshot baseline** (`--diff --against deployed`): the same diff over the profile
  derived from the recorded `model_sql` in the deployed-schema snapshot. Motivated by the
  release-manager question "what will the next run do differently?"; deferred until a concrete
  case shows the migration plan does not already answer it.
- **Cost estimates on downgrades**: annotate a technique downgrade with the row-count delta a
  `smelt bakeoff` measurement would show. Depends on the cost-aware planner direction in
  `docs/research/20260905-ten-directions.md`.

## References

- **Code**: `crates/smelt-logical/src/analysis/profile.rs` (`PropertyProfile`), `crates/smelt-logical/src/analysis/diff.rs` (`diff_profiles`, the direction table), `crates/smelt-logical/src/analysis/diff_stories.rs` (`narrate`, the story fold, severity, headline, lens title, duration humanisation), `crates/smelt-logical/src/analysis/diff_render.rs` (text/Markdown rendering, shared by the CLI and the editor), `crates/smelt-core/src/baseline.rs` (ref/merge-base resolution, `git archive` materialisation, cleanup), `crates/smelt-core/src/workspace.rs` (`load_workspace`, consumed by both sides), `crates/smelt-runtime/src/profile.rs` and `crates/smelt-runtime/src/property_diff.rs` (`build_model_diagnostics`, `profiles_for_workspace`, the `work_side`/`baseline_side`/`report` pipeline shared by the CLI and the LSP), `crates/smelt-cli/src/commands/explain_diff.rs` (`smelt explain --diff`), `crates/smelt-lsp/src/property_diff.rs` (`ProjectDiffState`, `anchor_for`, `diagnostics_for_model`, `refresh`)
- **Tests**: `crates/smelt-logical/tests/story_coverage.rs`, `crates/smelt-cli/tests/property_profile_parity.rs`, `crates/smelt-cli/tests/property_diff_cli.rs`, `crates/smelt-cli/tests/property_diff_ci_docs.rs`, `crates/smelt-logical/tests/diff_purity.rs`, `crates/smelt-core/tests/baseline.rs`, `crates/smelt-runtime/tests/profile_workspace.rs`, `crates/smelt-lsp/tests/property_diff_parity.rs`, `crates/smelt-lsp/tests/property_diff_refresh.rs`, `crates/smelt-lsp/tests/property_diff_coalescing.rs`, `crates/smelt-lsp/tests/property_diff_overlay.rs`
- **User docs**: `docs-site/docs/reference/smelt-explain.md`, `docs-site/docs/guide/ci.md`, `docs-site/docs/guide/editor-features.md`
- **Plans (history)**: the feature was driven end-to-end from `docs/outcomes/20260905-property-diff/outcome.md`, whose phase table and decision log record how each part of the surface landed; `docs/plans/20260906-property-diff-stories.md` (stories, the headline, the direction-table corrections for `cell_added`/`cell_removed` and grain widening).
- **Related specs**: `model_properties.md`, `incremental_models.md`, `definition_deltas.md`, `cli.md`, `lsp.md`, `ui_model_diagnostics.md`, `diagnostics.md`, `architecture.md`

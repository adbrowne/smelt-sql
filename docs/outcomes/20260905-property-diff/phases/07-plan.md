# Phase 7 plan — LSP: code lens, `PropertyDowngrade`, baseline cache, `property_diff_parity`

**Outcome:** `docs/outcomes/20260905-property-diff/outcome.md` criterion 7 (+ 9).
**Spec:** `docs/specs/property_diff.md` §Surface "Editor", §Diagnostics, §"Baseline
materialisation", §Constraints 5/6/7; `docs/specs/lsp.md` §"Watched files", §Diagnostics.
**Carry:** `phase67-carry.md` D1 (2.4–3.0 s / 2000 models — shapes the cache), D3 (every test
must be able to say how it fails against a broken implementation).

## Objective

The LSP advertises code lens; a shifted model file gets exactly one lens
`N downgrades, M upgrades vs <short ref>` and one `PropertyDowngrade` warning per downgrade; an
unshifted model gets neither; a non-git workspace gets neither and logs at `info`. The baseline
side is cached per resolved commit, invalidated by re-resolution and by a `.git/HEAD`/ref watch.
Standing gate `cargo test -p smelt-lsp --test property_diff_parity`. `editor-features.md`
documents the surface.

## Dependency-edge verdict (the central question)

**Today `smelt-lsp` cannot reach the code it needs.** `crates/smelt-lsp/Cargo.toml` depends only
on `smelt-core`, `smelt-db`, `smelt-parser`, `smelt-types`. The diff needs
`smelt_runtime::profile::profiles_for_workspace` (criterion 3 puts profile assembly in
`smelt-runtime`, because a profile carries the probe plan `smelt-db` cannot own) and
`smelt_logical::analysis::{diff, diff_render}`.

Minimal legal change: **add `smelt-runtime` (and, transitively re-exported or directly,
`smelt-logical`) to `smelt-lsp`'s `[dependencies]`.** This is acyclic (`smelt-runtime` depends on
core/parser/db/types/planner/logical/backend/dialect/state — never on `smelt-lsp`) and violates
no `CLAUDE.md` invariant:

- *Layered single-ownership* constrains `smelt-db → smelt-planner`, which is untouched
  (`cargo tree -p smelt-db -i smelt-planner` is unaffected by an edge above `smelt-db`).
- *Run-pipeline parity* keeps `SqlCompiler`/`PrintContext`/emitter factories `pub(crate)`; we add
  no new escape from it — `profiles_for_workspace` is already `pub` and already consumed by
  `smelt-cli` outside `execute_project`.
- *Workspace loading parity* is preserved: both sides still load through
  `smelt_core::workspace::load_workspace` (see the open-buffer decision below).
- Build weight is small: `smelt-runtime`'s heaviest deps are `smelt-backend` (traits + arrow) and
  `smelt-state`; **no DuckDB link**, so `smelt-lsp` stays buildable with no system library.
- `smelt-lsp`'s `python` feature must forward: `python = [… , "smelt-runtime/python"]`, otherwise
  `smelt-core/python` is on while `smelt-runtime`'s python-gated code is off in the same build.

## Spec delta (edit `docs/specs/property_diff.md` first, in this phase)

- **Δ1 `<short ref>` is defined.** §Surface "Editor": the lens's `<short ref>` is the baseline's
  `ref` string, except when that string is a full 40-hex commit sha, in which case its 7-character
  abbreviation. Rendered by one primitive in `diff_render`, like every other spelling.
- **Δ2 Refresh triggers, stated.** §Surface "Editor": the editor's diff is recomputed on workspace
  load, on a model file being **saved** or changed outside the editor, and when the resolved
  baseline commit changes — not on every keystroke. Open buffers override on-disk contents for
  **model files** on the working-tree side; an unsaved `smelt.yml` or source-YAML buffer takes
  effect when it is saved.
- **Δ3 In-flight behaviour.** §Surface "Editor": while a derivation is running, the editor shows
  the previously computed diff if one exists and nothing at all on first load. It never shows a
  half-computed diff.
- **Δ4 Anchor scope, made checkable.** §Surface "Editor": a change whose `subject` names a column
  anchors at that SELECT-list item; a change whose `subject` names a source or upstream model
  anchors at that `FROM`/`JOIN` item; every other change (cell addresses `<group>@<trigger>`,
  refusal texts, whole-model verdicts) anchors at the model's first SQL token.

## Design decisions

**D1 — one shared pipeline, three functions, in `smelt-runtime`.** `crates/smelt-cli/src/commands/
explain_diff.rs` currently owns the whole pipeline inline. Extract it to
`crates/smelt-runtime/src/property_diff.rs` as three pure-ish steps so the LSP gets a cache seam
without duplicating the pipeline (Constraint 5, "the editor never runs its own comparison"):

```
pub struct WorkSide  { loaded: LoadedWorkspace, sources: Vec<SourceInfo>, profiles: WorkspaceProfiles, graph: DependencyGraph }
pub struct BaselineSide { resolved: ResolvedBaseline, loaded: LoadedWorkspace, sources: Vec<SourceInfo>, profiles: WorkspaceProfiles }
pub fn work_side(project_dir: &Path, overlays: &BTreeMap<PathBuf, String>) -> Result<WorkSide, PropertyDiffError>
pub fn baseline_side(project_dir: &Path, explicit_ref: Option<&str>) -> Result<BaselineSide, PropertyDiffError>
pub fn report(work: &WorkSide, base: &BaselineSide) -> DiffReport
```

`BaselineCheckout` is created and dropped **inside** `baseline_side` (everything derived from it is
read before it returns), preserving Constraint 8. The CLI calls `work_side` → `baseline_side` →
`report`, in that order, preserving Phase 5's D2 sequencing (a broken working tree fails before any
scratch directory exists). `--select` narrowing stays in the CLI (it needs the CLI's selector
machinery); `narrow_to` is already single-owned.

**D2 — the cache is the baseline side, keyed `(project_root, commit)`.** D1's measurement says
both sides cost ~2.4–3.0 s at 2000 models; caching the baseline halves every refresh and, more
importantly, skips `git archive` + untar. State lives on `Backend`:

```
property_diff: Arc<Mutex<HashMap<PathBuf /*project_root*/, ProjectDiffState>>>
struct ProjectDiffState { baseline: Option<(String /*commit*/, Arc<BaselineSide>)>, report: Option<Arc<DiffReport>>, lenses: HashMap<PathBuf, CodeLens>, diagnostics: HashMap<PathBuf, Vec<DbDiagnostic>>, silent_reason: Option<String>, running: bool }
```

**Invalidation is re-resolution, not the watch.** Every refresh calls `resolve_baseline` (two or
three cheap `git` invocations) and compares the commit to the cached one; equal ⇒ reuse, different
⇒ re-derive. The `.git/HEAD` + ref watch is a *trigger* that makes the refresh happen promptly, not
the correctness mechanism — several clients (and VS Code configurations) do not report `.git`
changes, and a design that depended on the watch would silently serve a stale baseline after a
`git checkout`. Watch globs: `<repo_root>/.git/HEAD`, `<repo_root>/.git/refs/**`,
`<repo_root>/.git/packed-refs`. `repo_root` is private on `ResolvedBaseline`; add a
`pub fn repo_root(&self) -> &Path` accessor.

**D3 — project isolation.** The map is keyed on project root, one entry per project; each project
resolves its own baseline (two projects in a workspace folder may sit in different repos, or at
different paths in one repo, and `resolve_baseline` already takes `project_dir`). The cache key is
therefore `(project_root, commit)`, never a workspace-folder-wide commit. A model's lens/diagnostics
are looked up by file path within the project whose root is the path's longest matching prefix —
the same rule `did_open`/`did_change` already use.

**D4 — the open-buffer story (the likeliest place to break).** `load_workspace` is filesystem-based
and stays so (out-of-scope forbids an overlay loader). Instead, add
`smelt_core::workspace::apply_open_buffers(&mut LoadedWorkspace, &BTreeMap<PathBuf, String>)` —
a **post-load patch** that replaces `ModelFile::content` for paths that are open, applied by
`work_side` after `load_workspace` returns. Discovery still happens exactly once, in one place, so
Constraint 7 and the CLI↔LSP loading-parity rule hold; the overlay adds no second discovery path
and can never introduce a file the loader did not find. Buffer text comes from the Salsa DB
(`file_text`) for tracked `.sql` paths, which is where `did_change` already puts it. `smelt.yml`
and source YAML are read from disk by `Config::load`/`discover_source_infos` inside
`profiles_for_workspace` and are **not** overlaid — Δ2 says so out loud.

**D5 — anchoring.** Implemented as a pure function
`anchor_for(change: &Change, ast: &AstFile) -> TextRange` in `crates/smelt-lsp/src/property_diff.rs`:
1. `Dimension::{ColumnAdded, ColumnRemoved, Determinism, Comparability, Discriminant,
   LiteralColumn}` and `Grain` — subject is a column name. Walk `SelectStmt::select_list().items()`
   and take the first `SelectItem` whose `alias()` equals the subject, else whose
   `expression_source_text()` equals it; anchor on `alias_range()` if present, else the item's
   `syntax().text_range()`.
2. `Dimension::SourceBound` — subject is a source/model name. Walk `FromClause::table_refs()` and
   `joins()` and match a `TableRef` whose text contains the subject's last dotted segment; anchor
   on that node's range.
3. Everything else — cell subjects are `<group>@<trigger>` (`diff.rs::cell_key`), refusal subjects
   are free prose, several kinds have an empty subject; **none of these is derivable to a narrower
   range and the plan does not pretend otherwise**. Anchor at the model's first SQL token: the
   first non-trivia token of `AstFile::select_stmt()`, falling back to `TextRange::empty(0.into())`.
Ranges stay `rowan::TextRange` until the existing `to_lsp_diagnostic` boundary converter (the
diagnostic-range-encoding invariant).

**D6 — diagnostics merge, never a second publish.** `publish_diagnostics(uri)` replaces a file's
whole diagnostic set. A separate publish for `PropertyDowngrade` would clobber, or be clobbered by,
the Salsa set (the bug `python_diagnostics` already skirts). So `publish_diagnostics` gains one
step: after building `lsp_diagnostics`, append the cached `Vec<DbDiagnostic>` for that path from
`ProjectDiffState`, converted through the same `to_lsp_diagnostic`. Message = `change_line(change)`
plus `reason_line(change)` when present — `diff_render`'s primitives, never a new spelling
(Constraint 5 / the surface-parity rule).

**D7 — the derivation runs off the request path.** Refresh is `tokio::task::spawn_blocking`
(returns plain `Send` data; no rowan node crosses the boundary), guarded by `running` so
concurrent triggers coalesce. `code_lens` and `publish_diagnostics` only *read* the cached state,
so no LSP request can ever block for 3 s.

**D8 — non-git silence.** `PropertyDiffError::Baseline(BaselineError::NotAGitWorkTree { .. })` (and
`NoBaseBranch`, `NoProjectAtRef`) set `silent_reason` and emit `tracing::info!` + one
`client.log_message(MessageType::INFO, …)`. No lens, no diagnostic, no `showMessage`. Fail-loud
discipline is satisfied by the log, not by a user diagnostic — the spec says an un-versioned
workspace is not an error.

## TDD test list

Each test states how it fails against a broken implementation (D3).

1. **`anchor_column_subject_hits_the_select_item`** (unit, `property_diff.rs`) — a model
   `SELECT a, b AS renamed FROM t`; a `Determinism` change with `subject = "renamed"` anchors on
   `renamed`'s alias range. *Fails against a broken impl:* if anchoring falls back to the first SQL
   token, the returned range is the `SELECT` keyword's, and the asserted `(start, end)` offsets
   differ by the whole select list.
2. **`anchor_cell_subject_falls_back_to_first_sql_token`** (unit) — a `CellTechnique` change with
   `subject = "amount@new_data"` anchors at the first token. *Fails:* an implementation that
   substring-matches the subject anywhere would match nothing and (if it returned `Option::None`
   unwrapped, or an empty range at the wrong offset) return a range ≠ the `SELECT` token's.
3. **`lens_title_matches_the_summary_counts`** (unit) — a hand-built `ModelDiff` with 2 downgrades,
   1 upgrade renders `2 downgrades, 1 upgrades vs abc1234`. *Fails:* swapping the two counts (the
   `apply_failure_reasons` swap class from D3.5) makes the string `1 downgrades, 2 upgrades`; the
   asserted literal is asymmetric so the swap cannot pass.
4. **`short_ref_abbreviates_a_sha_and_leaves_a_named_ref_alone`** (unit, `diff_render`) — Δ1.
   *Fails:* an implementation that always truncates to 7 chars turns `merge-base(main)` into
   `merge-b`, which the second assertion rejects.
5. **`shifted_model_gets_one_lens_and_one_diagnostic_per_downgrade`** (e2e, real LSP over duplex
   streams, temp git repo cloned from `examples/timeseries` with Phase 5's `raw.users` join edit) —
   `textDocument/codeLens` on `user_daily_spend.sql` returns exactly one lens whose title starts
   with a non-zero downgrade count; `publishDiagnostics` for that URI contains ≥1 diagnostic with
   code `property-downgrade` and severity `WARNING`. *Fails:* if the state is never populated (a
   trigger not wired) the lens array is empty; if diagnostics are published on their own channel
   they are clobbered by the Salsa publish and the assertion sees none.
6. **`unshifted_model_gets_neither`** (e2e, same fixture) — a model untouched by the edit and not
   downstream of it returns zero lenses and zero `property-downgrade` diagnostics. *Fails:* an
   implementation that puts a lens on every model in the project (or on every file in the report's
   `edited_files` rather than its `models`) returns one here.
7. **`non_git_workspace_is_silent_and_logs_at_info`** (e2e, a temp dir with `smelt.yml` and no
   `.git`) — zero lenses, zero `property-downgrade` diagnostics, and a `window/logMessage`
   notification of type `Info` whose text names "not inside a git work tree". *Fails:* the current
   fail-loud reflex would raise `PropertyDiffBaselineUnavailable` as a diagnostic; the assertion on
   zero diagnostics catches it, and the assertion on the log catches silent swallowing.
8. **`baseline_is_reused_until_head_moves`** (integration, direct on the state module, not over the
   wire) — refresh twice with no git change and assert `baseline_side` ran once (a call counter on
   a test seam, or `Arc::ptr_eq` on the cached `BaselineSide`); then `git commit` on the branch so
   the merge-base is unchanged → still reused; then move the baseline branch → re-derived.
   *Fails:* a cache keyed on project root alone never re-derives (third leg fails); no cache at all
   re-derives every time (first leg fails). Both legs are needed — this is the test that would have
   caught a cache that "looks" keyed on commit but compares the wrong field.
9. **`property_diff_parity`** (the standing gate, `crates/smelt-lsp/tests/property_diff_parity.rs`)
   — see below.

### How the parity gate avoids being the sixth vacuous check

This outcome has produced five "covered but unreachable" defects, and the obvious way to write this
gate — call one function, render it two ways, compare — would be the sixth. Three rules:

- **The two sides are genuinely different code paths.** The CLI side is
  `serde_json::to_value(report)` from `smelt_runtime::property_diff::report(...)` — the exact value
  `smelt explain --diff --json` prints. The LSP side is read **off the wire**: lens titles from a
  real `textDocument/codeLens` response and diagnostics from real `publishDiagnostics`
  notifications, after going through the server's project-root routing, model-name→file-path
  mapping, per-file count aggregation, anchoring, severity mapping and caching. None of those steps
  exists on the CLI side, so agreement is a real claim. (Shelling out to the `smelt` binary was
  considered and rejected: `CARGO_BIN_EXE_*` is not available across packages, and a
  "skip if the binary is missing" leg is exactly the vacuity trap.)
- **Hard-coded non-emptiness.** The gate asserts, with literal expected values before any
  comparison: the fixture yields ≥1 shifted model, ≥1 downgrade, that `user_daily_spend` is among
  the shifted, and that at least one project model is *not* shifted. An empty-vs-empty comparison
  therefore fails at the first assertion, not at the (trivially true) set equality.
- **A sabotage run, recorded in the summary.** Before declaring the gate done, mutate the LSP side
  once (drop the last downgrade from the per-file diagnostic list) and confirm the gate fails; then
  revert. The summary states the observed failure message. If it does not fail, the gate is not a
  gate and the summary says so rather than counting it as coverage.

## Tasks

1. Spec: land Δ1–Δ4 in `docs/specs/property_diff.md` (§Surface "Editor"), and the `PropertyDowngrade`
   cross-reference already present in `lsp.md` needs no change. Timeless-oracle rule applies.
2. `smelt-core`: `ResolvedBaseline::repo_root()` accessor; `git_watch_paths(&ResolvedBaseline) ->
   Vec<PathBuf>`; `workspace::apply_open_buffers`.
3. `smelt-runtime`: new `property_diff` module (`WorkSide`/`BaselineSide`/`work_side`/
   `baseline_side`/`report`/`PropertyDiffError`); refactor `smelt-cli`'s `explain_diff.rs` onto it
   with **no behaviour change** — `cargo test -p smelt-cli --test property_diff_cli` is the
   regression oracle for the refactor and must stay green untouched.
4. `smelt-logical::analysis::diff_render`: `short_ref(&BaselineInfo)` and
   `lens_title(&ModelDiff, &BaselineInfo)` primitives (+ unit tests 3, 4).
5. `smelt-lsp/Cargo.toml`: add `smelt-runtime`, `smelt-logical`; forward the `python` feature.
6. `smelt-lsp/src/property_diff.rs`: `ProjectDiffState`, `anchor_for`, `diagnostics_for_report`,
   `lenses_for_report` — all pure over `(DiffReport, parsed AST, model→path map)` so tests 1–3 need
   no server.
7. `smelt-lsp/src/backend.rs`: `code_lens_provider` in `ServerCapabilities`; `code_lens` handler;
   the append step in `publish_diagnostics`; `refresh_property_diff` (spawn_blocking, coalesced);
   triggers on `initialized`, `did_save`, `did_change_watched_files`; `.git` watcher globs in
   `derive_watch_globs` (needs the resolved repo root per project); the `info` log path.
8. `crates/smelt-lsp/tests/property_diff_parity.rs` (test 9) and the e2e tests 5–8.
9. `docs-site/docs/guide/editor-features.md`: the lens, the warning, the baseline and when it
   refreshes, and the non-git silence.
10. Optional, and untested by cargo — say so in the summary: register a `smelt.showPropertyDiff`
    command in `editors/vscode` that writes the lens's `model_block` argument to the output channel,
    so the emitted lens command is not dead. If skipped, note it as a Known Divergence.

## Risks

- **R1 (highest) — `example_workspaces` / `example_diagnostics` flake.** Those gates run the real
  LSP over `examples/`, which live inside this repo's git tree. On a branch with edited examples the
  server would now publish `property-downgrade` warnings into a suite that asserts zero
  diagnostics. Mitigation: those suites must filter on code, explicitly and with a comment
  (`property-downgrade` is advisory, not a correctness diagnostic), and the implementer must run
  both gates on this branch — where `examples/` *has* been edited relative to `main` — before
  claiming green. Do not "fix" it by suppressing the feature.
- **R2 — `.git` watch never fires.** Handled by design (D2: re-resolution is the correctness
  mechanism), but the implementer must not delete the re-resolve as "redundant".
- **R3 — refresh storms.** A `git rebase` touching hundreds of files fires hundreds of watched-file
  events. The `running` flag coalesces; a trailing re-run must be scheduled once, not per event.
- **R4 — `LoadedWorkspace` overlay drift.** If a future `profiles_for_workspace` starts re-reading
  model text from disk, the overlay silently stops working. Test 5's fixture edits on disk, so it
  would not catch it; add one assertion that an *unsaved* buffer edit (didChange without a disk
  write) changes the lens count.
- **R5 — first-load latency on `examples/huge`.** 2.4–3.0 s in `spawn_blocking` is fine; it becomes
  a problem only if refresh is ever moved onto the request path.

## Verification gate

Split per `shared-context.md` (each stage inside the 10-minute tool timeout):

```
cargo fmt --all -- --check
bash .claude/scripts/clippy-gate.sh 2>&1 | tail -40
CARGO_BUILD_JOBS=4 cargo test -p smelt-logical -p smelt-core --quiet 2>&1 | tail -20
CARGO_BUILD_JOBS=4 cargo test -p smelt-runtime --quiet -- --test-threads=4 2>&1 | tail -20
CARGO_BUILD_JOBS=4 cargo test -p smelt-cli --test property_diff_cli --test property_profile_parity --quiet 2>&1 | tail -20
CARGO_BUILD_JOBS=4 cargo test -p smelt-lsp --quiet -- --test-threads=4 2>&1 | tail -40
cargo test -p smelt-cli --test example_diagnostics --quiet 2>&1 | tail -20
cargo tree -p smelt-db -i smelt-planner    # must still show no production path
mkdocs build --strict                       # editor-features.md
```

Plus the hardening budget (no new unclassified `unwrap`/`expect`, no `println!` in `smelt-lsp`).

## Commit message

```
feat(lsp): property-diff code lens and PropertyDowngrade diagnostics

Advertise codeLens; publish one lens per shifted model
(`N downgrades, M upgrades vs <short ref>`) and one PropertyDowngrade
warning per downgrade, anchored per docs/specs/property_diff.md §Surface
"Editor". The baseline side is cached per (project root, resolved commit),
re-resolved on every refresh and triggered by a .git/HEAD + refs watch; a
non-git workspace is silent and logs at info.

The CLI and LSP now share one pipeline
(`smelt_runtime::property_diff::{work_side, baseline_side, report}`), so the
editor never runs its own comparison (property_diff.md §Constraints 5).
Open model buffers override on-disk contents on the working-tree side via a
post-load patch; load_workspace stays the single discovery path.

Standing gate: cargo test -p smelt-lsp --test property_diff_parity.
```

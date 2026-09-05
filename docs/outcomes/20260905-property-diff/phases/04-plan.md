# Phase 4 plan — baseline materialisation

**Outcome:** `docs/outcomes/20260905-property-diff/outcome.md` (success criterion 3)
**Spec:** `docs/specs/property_diff.md` §"Baseline materialisation", §"Attribution",
§Constraints 6 (fail-loud), 7 (loading parity), 8 (no repository mutation)
**Predecessor:** `phases/03-summary.md` — `diff_profiles(old, new, &DiffGraph)` is pure and
takes `edited` / `project_config_changed` as *inputs*. This phase produces those inputs and both
profile maps. Nothing in this phase may reach into `smelt-logical::analysis::diff`.

## Objective

Ship the git side of the diff, in `smelt-core`, plus the both-sides profile derivation:

1. `smelt_core::baseline` — resolve a baseline ref (explicit, or merge-base with `main`/`master`),
   export the project subtree at that commit with `git archive` into a scratch directory, hand back
   the extracted project root, and delete the scratch on every exit path.
2. `smelt_core::baseline::edited_set` — the §Attribution edited set, derived by comparing the two
   *loaded* workspaces (not by asking git for a file list).
3. `smelt_runtime::profile::profiles_for_workspace(&LoadedWorkspace) -> BTreeMap<String, PropertyProfile>`
   — one function both sides call, lifting the ~110 lines of glue currently living only in
   `crates/smelt-cli/tests/property_profile_parity.rs::build_diagnostics_for`.
4. `BaselineError` classified as exit `2` by `smelt_cli::errors::exit_code_for`.

Out of scope (Phase 5): any `smelt explain` flag, any rendering, calling `diff_profiles`,
building the `DiffGraph` itself.

## Spec delta (required — write it first, in this phase)

**Δ1 — the "thin `smelt-db` query" claim is not implementable as written.**
`docs/specs/property_diff.md` §Interactions → "Salsa purity" says the profile derivation "is
wrapped by a thin `smelt-db` query on each side". It cannot be: a `PropertyProfile` carries
`probes`, and the probe plan is `smelt_runtime::probe_plan::probe_plan_for_model`
(`crates/smelt-runtime/src/probe_plan.rs:45`); `smelt-runtime` depends on `smelt-db`, not the
reverse (`crates/smelt-db/Cargo.toml` has no `smelt-runtime`). Rewrite the bullet to: the
per-model maintenance-plan derivation stays a thin `smelt-db` query (`maintenance_plan_report`)
over pure `smelt-logical` functions; the profile is *assembled* from it by the single-owner
builder in `smelt-runtime` (`build_model_diagnostics`), which both the report and the diff call
— no second assembly path. The Salsa-purity rule is untouched (nothing new is added to a query).

**Δ2 — the edited set must include a model's frontmatter.**
§Attribution defines the edited set as "*frontmatter-stripped* SQL text differs, plus `smelt.yml`
model override differs, plus source `.yml` declaration changed". `smelt_parser::strip_frontmatter`
(`crates/smelt-parser/src/lib.rs:159`) blanks each frontmatter line to `--` + spaces, so a
frontmatter edit is invisible unless it changes a line's *byte length* — i.e. the rule as written
both misses real edits (`unique_key: [a]` → `unique_key: [b]`) and fires on whitespace ones. A
model whose frontmatter changed and which is therefore missed would shift with no edited ancestor
and be reported as `downstream`/`of: []`/"project configuration changed" — a wrong attribution.
Amend to: "whose frontmatter-stripped SQL text **or parsed frontmatter metadata** differs".

Both deltas are behaviour statements, not phase notes — timeless-oracle rule applies.

## Design decisions

**D1 — module placement.** New `crates/smelt-core/src/baseline.rs`, `pub mod baseline;` in
`lib.rs`. `smelt-core` already owns `workspace::load_workspace`, `discover_source_infos`,
`config::Config` — everything the edited set compares — and is the crate the phase table names.
No `smelt-core` crate today shells out to git (`rg 'Command::new("git")'` finds only
`crates/smelt-bench/src/metrics.rs:50`, an unrelated build-stamp helper); there is no shared
process helper to reuse, so this module owns its own.

**D2 — the git surface.** One private `fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>, BaselineError>`
using `std::process::Command` — no shell, no string interpolation of user input into a command
line, `.current_dir(repo)`, `.env("GIT_OPTIONAL_LOCKS", "0")` (so no invocation can refresh/write
`.git/index`, which is half of Constraint 8 for free), stdout+stderr captured, non-zero status
mapped to a typed error carrying the trimmed stderr. Exactly four subcommands are used:
`rev-parse --show-toplevel`, `rev-parse --verify <rev>^{commit}`, `merge-base HEAD <base>`,
`cat-file -e <commit>:<rel>/smelt.yml`, plus `archive`. No `checkout`, no `worktree`, no `stash`,
no `read-tree` — Constraint 8 is upheld by the *absence* of those, and asserted by a test.

**D3 — ref resolution.**
```rust
pub enum ResolvedAs { Explicit, MergeBase }
pub struct ResolvedBaseline { pub requested: String, pub commit: String, pub resolved_as: ResolvedAs }
pub fn resolve_baseline(project_dir: &Path, explicit: Option<&str>) -> Result<ResolvedBaseline, BaselineError>
```
Explicit: `rev-parse --verify <ref>^{commit}`. Default: `main`, else `master` (spec §Surface names
exactly these two; adding `origin/main` would be a spec change — do not), then
`merge-base HEAD <base>`. Then verify the project exists at that commit: `cat-file -e
<commit>:<rel>/smelt.yml`, falling back to `smelt.yaml`. `requested` is the string the JSON
`baseline.ref` field prints (`<ref>` as given, or `merge-base(main)`), so Phase 5 needs no
second derivation.

**D4 — the scratch guard.** `pub struct BaselineCheckout { scratch: tempfile::TempDir, project_root: PathBuf }`
with `pub fn project_root(&self) -> &Path`. `tempfile` moves from `[dev-dependencies]` to
`[dependencies]` in `crates/smelt-core/Cargo.toml` (already a dependency of 12 other crates;
no new lock entry). `tar = "0.4"` is added — already in `Cargo.lock` at 0.4.45 transitively, so
no new external surface. Cleanup is `TempDir`'s own `Drop`, and the `TempDir` is created
**first**, before any `?` that can fail, so every error path after creation unwinds through it.
Honest limits, to be stated in the module doc and the test's comment: `Drop` does not run under
`panic = "abort"`, `std::process::abort`, or SIGKILL. What the test can honestly assert is
(a) the scratch path does not exist after the value is dropped on the happy path, (b) it does not
exist after a *failing* materialisation, and (c) the repository is byte-unchanged. A leaked
scratch after SIGKILL is a documented limitation (the OS temp dir is the backstop), not a
tested guarantee.

**D5 — extraction.** `git archive --format=tar <commit> -- <rel>` with `Stdio::piped()`, unpacked
streaming via `tar::Archive::new(child.stdout)` into the scratch dir, then `child.wait()` checked
(never `output()` — a large project would deadlock the pipe). `rel` is `project_dir` relative to
the `--show-toplevel` root, both canonicalised; when the project *is* the repo root, `rel` is
empty and the pathspec is omitted. Extracted project root = `scratch/<rel>`. The `tar` crate
already refuses `..` entries, so a hostile committed path cannot escape the scratch.

**D6 — `.smelt/` scrub.** Spec: "Nothing under `.smelt/` at the baseline is read even if it is
committed". `smelt_db::workspace_ingest::ingest_loaded_workspace` calls
`register_deployed_schemas_from_disk` (`crates/smelt-db/src/workspace_ingest.rs:68`), which reads
`.smelt/`. So `materialize` removes `<extracted project root>/.smelt` after unpacking, before
returning. Test with a fixture repo that commits a `.smelt/` file.

**D7 — where the edited set is computed, and how (asked question 1).** Here, in
`smelt_core::baseline`, as a **content comparison over the two loaded workspaces** — never
`git diff --name-only`. Signature:
```rust
pub struct EditedSet { pub names: BTreeSet<String>, pub files: Vec<String>, pub project_config_changed: bool }
pub fn edited_set(base: &LoadedWorkspace, base_sources: &[SourceInfo],
                  work: &LoadedWorkspace, work_sources: &[SourceInfo]) -> EditedSet
```
Rationale: `git diff` yields *paths*, but `DiffGraph.edited` is keyed by **model and source
names** (`phases/03-summary.md`), and the spec's three edit predicates are semantic
(frontmatter-stripped SQL, the `smelt.yml` *model override*, the source declaration) — none is a
path-level fact. A content comparison also means the working-tree side is whatever
`load_workspace` sees, which is exactly the uncommitted-edits behaviour question 2 asks about.
`files` (project-relative paths, sorted) is carried alongside for the JSON `edited_files` field
and the text form's "N files changed"; it is derived from the same comparison, so the two can
never disagree.
Per model, edited iff any of: `strip_frontmatter(content)` differs; `metadata` differs (Δ2);
`config.models.get(name)` differs (compared as `serde_json::Value`, since `ModelConfig` has no
`PartialEq`). Per source, edited iff its `SourceInfo` differs with `path` zeroed
(`SourceInfo` is `PartialEq + Eq`, `crates/smelt-core/src/sources.rs:180`; only `path` is
absolute and therefore side-dependent — zeroing it and comparing the whole struct means a field
added later is compared automatically, unlike a hand-written field list).
**One-sided files:** a model or source present on exactly one side is edited. That is deliberate
and harmless: `diff_profiles` already codes those models `Added`/`Removed` and never asks
`attribute()` about them, but a *downstream* model must still be able to attribute to the
added/removed node it references.
Source names use the same convention `DiffGraph::from_dependency_graph` uses — `address_segments`
with the leading `sources` segment stripped, dot-joined — so `edited` and `upstream` key against
each other. Model names are `ModelFile::canonical_path()`.
`project_config_changed`: `serde_json::to_value(&config)` on each side with the `models` key
removed from the object, compared. (`Config` is `Serialize` but not `PartialEq`;
`crates/smelt-core/src/config.rs:268`.)

**D8 — uncommitted working-tree edits (asked question 2).** The new side is the working tree, not
`HEAD`, and D7 makes that automatic: the working side is a `load_workspace(project_dir)` of the
real directory, so an uncommitted edit is simply content that differs from the archived baseline.
Nothing compares two commits anywhere in this phase — `HEAD` appears only as the *left* argument
of `merge-base`. A test asserts it explicitly: commit a model, edit it *without committing*,
assert the model is in `edited_set.names`.

**D9 — profile derivation, both sides.** New `crates/smelt-runtime/src/profile.rs`:
`pub fn profiles_for_workspace(loaded: &LoadedWorkspace) -> BTreeMap<String, PropertyProfile>`.
It builds a fresh `smelt_db::Database`, calls `ingest_loaded_workspace` + `set_workspace` (the
loading-parity path; `init_db` in `smelt-cli` is the *other* consumer of the same discovery, not
a second discovery), then per model: `maintenance_plan_report` → `probe_plan_for_model` →
`build_model_diagnostics` → its `.profile`. Models with no maintenance plan are absent from the
map (they have no profile to diff; consistent with `property_profile_parity`'s own skip).
`smelt_cli::explain::build_bound_context` (`crates/smelt-cli/src/explain.rs:141`) **moves** to
`smelt_runtime::diagnostics` with a `pub use` left in `smelt-cli` — moved, not copied, so no
second copy exists. `smelt-cli`'s `property_profile_parity` harness is then rewritten to call
`profiles_for_workspace`, which is the red-green proof that the lift preserved behaviour: that
gate is byte-exact against the real `smelt explain --json` binary.

**D10 — error taxonomy.** `#[derive(Debug, thiserror::Error)] pub enum BaselineError` in
`baseline.rs`, one variant per spec trigger: `NotAGitWorkTree { dir }`, `UnknownRef { r#ref, stderr }`,
`NoBaseBranch` (neither `main` nor `master`), `MergeBaseFailed { base, stderr }`,
`NoProjectAtRef { commit, rel }`, `GitUnavailable(std::io::Error)`, `Archive { stderr }`,
`Scratch(std::io::Error)`. Every message names what the user must do. Fail-loud: no variant is
recoverable into an empty diff, and `resolve_baseline` never returns a `None`-shaped success.
`smelt_cli::errors::exit_code_for` (`crates/smelt-cli/src/errors.rs:76`) gains a
`downcast_ref::<smelt_core::baseline::BaselineError>()` arm returning `2`, alongside the existing
`ProjectError`/`ConfigError` arms — wired and tested here even though the flag arrives in Phase 5.
No new `unwrap`/`expect` in production code (hardening ratchet).

## TDD test list (red before green, in this order)

`crates/smelt-core/tests/baseline.rs` (new; `tempfile` is available, and a shared
`fn fixture_repo() -> TempDir` helper `git init`s, writes `smelt.yml`/`models/*.sql`, and commits
with `-c user.email=... -c user.name=...` so it works on a bare CI box):

1. `resolve_baseline_rejects_non_git_directory` — a plain temp dir ⇒ `Err(NotAGitWorkTree)`.
   *(red first: module does not exist.)*
2. `resolve_baseline_explicit_ref_resolves_to_commit` — `resolve_baseline(dir, Some("HEAD"))` ⇒
   `commit == git rev-parse HEAD`, `resolved_as == Explicit`.
3. `resolve_baseline_unknown_ref_is_an_error` — `Some("nope/zzz")` ⇒ `Err(UnknownRef)`, **not** an
   empty result (Constraint 6).
4. `resolve_baseline_defaults_to_merge_base_with_main` — repo on a branch off `main` with a commit
   on each ⇒ `commit == git merge-base HEAD main`, `resolved_as == MergeBase`.
5. `resolve_baseline_falls_back_to_master` — same with the default branch named `master`.
6. `resolve_baseline_errors_when_project_absent_at_ref` — project subdir added only in the working
   tree ⇒ `Err(NoProjectAtRef)`.
7. `materialize_exports_project_subtree_at_ref` — model edited after the commit; the extracted
   `project_root()/models/m.sql` holds the **committed** text, and `project_root()/smelt.yml` exists.
8. `materialize_of_a_subdirectory_project` — repo root ≠ project dir; extracted root is the
   project dir's content, not the repo's.
9. `materialize_drops_committed_dot_smelt` — `.smelt/x.json` committed ⇒ absent from the extract (D6).
10. `checkout_scratch_is_deleted_on_drop` — capture `project_root().to_path_buf()`, drop, assert
    `!exists()`.
11. `checkout_scratch_is_deleted_when_materialization_fails` — force a failure after scratch
    creation (bogus commit passed to `materialize`), assert no `smelt-baseline-*` entry was left in
    `std::env::temp_dir()` (snapshot the dir listing before/after).
12. **`diff_leaves_no_repository_state`** (criterion 3's named test) — snapshot
    `git status --porcelain`, `git worktree list`, `git stash list`, `git for-each-ref`, and the
    mtime+len of `.git/index`; run `resolve_baseline` + `materialize` + `load_workspace` +
    `edited_set`; assert all five are byte-identical afterwards.
13. `git_surface_uses_no_mutating_subcommand` — structural: read `baseline.rs`'s own source
    (`include_str!`) and assert it contains no `"checkout"`, `"worktree"`, `"stash"`, `"read-tree"`,
    `"update-ref"`, `"commit"` literal. Cheap standing guard for Constraint 8 as the module grows.
14. `edited_set_flags_uncommitted_sql_edit` (D8) — commit, edit a model on disk without
    committing ⇒ its canonical name in `names`, its path in `files`.
15. `edited_set_ignores_a_formatting_only_edit` — a trailing-newline/whitespace-only change
    outside frontmatter that leaves the stripped text differing *is* an edit (be precise: use an
    edit that leaves the file byte-identical after strip — e.g. a frontmatter comment reflow — and
    assert not edited). Pairs with criterion 4's "no models shifted" case.
16. `edited_set_flags_a_frontmatter_only_edit` (Δ2) — change `unique_key:` in frontmatter only ⇒
    edited. **Red against the spec's literal rule**, green after Δ2.
17. `edited_set_flags_a_smelt_yml_model_override` — `models: {m: {materialization: table}}` added ⇒
    `m` edited, and `project_config_changed == false`.
18. `edited_set_flags_a_project_level_config_change` — a project-level key changed with no model
    override touched ⇒ `project_config_changed == true`, `names` empty.
19. `edited_set_flags_a_changed_source_declaration` — a source `.yml` column added ⇒ the **bare
    dotted source name** (leading `sources` stripped) in `names`, so it keys against
    `DiffGraph::upstream`.
20. `edited_set_flags_one_sided_files` — a model present only in the working tree ⇒ edited.

`crates/smelt-runtime/tests/profile_workspace.rs` (new):

21. `profiles_for_workspace_covers_every_maintained_model` — over `examples/timeseries`: the map's
    key set equals the set of models whose `maintenance_plan_report` is `Some`, and is non-empty.
22. `profiles_for_workspace_matches_the_report_builder` — for one model, the profile equals the
    one `build_model_diagnostics` produces through the existing per-model path (guards the lift).

`crates/smelt-cli/tests/` (extend, do not add a file):

23. `exit_code_for_baseline_error_is_2` — in the existing exit-code test module; red before the
    `downcast_ref` arm exists.
24. `property_profile_parity` — unchanged assertions, harness rewritten onto
    `profiles_for_workspace`; it must stay green (regression oracle for D9).

## Tasks (numbered, independently reviewable)

1. **Spec first.** Apply Δ1 and Δ2 to `docs/specs/property_diff.md`. Commit alone.
2. `smelt-core`: `tempfile` → `[dependencies]`, add `tar = "0.4"`; `pub mod baseline;`; the
   `git()` helper + `BaselineError` (D2, D10). Tests 1–3.
3. `resolve_baseline` (D3). Tests 4–6.
4. `materialize` + `BaselineCheckout` + `.smelt` scrub (D4, D5, D6). Tests 7–13.
5. `edited_set` + `EditedSet` (D7, D8). Tests 14–20.
6. Move `build_bound_context` to `smelt_runtime::diagnostics`, `pub use` in `smelt-cli` (D9).
   No test of its own; the workspace must still build and `explain` tests stay green.
7. `smelt_runtime::profile::profiles_for_workspace` (D9). Tests 21–22.
8. Rewrite `property_profile_parity`'s harness onto `profiles_for_workspace`; test 24 green.
9. `exit_code_for` arm (D10). Test 23.
10. Full gate; write `phases/04-summary.md` (≤40 lines) naming the exact signatures Phase 5
    consumes, and any deviation.

## Risks

- **R1 (highest) — the D9 lift is bigger than it looks.** `build_diagnostics_for` in the parity
  test threads 17 arguments and reaches into `smelt-cli` for `build_bound_context`, `init_db`, and
  target/schema/ephemeral-resolver setup. If moving it out of `smelt-cli` starts pulling more
  `smelt-cli` helpers down into `smelt-runtime`, **stop at task 7**, land tasks 1–5 + 9 (the
  criterion-3 half), and hand D9 to Phase 5 with the blocker recorded. The git module is the
  phase's contract; the profile lift is the convenience.
- **R2 — `git archive` needs `git` on PATH in CI.** Every gate box already runs git (the repo is
  a git checkout and `verify-phase.sh` runs in it), but `GitUnavailable` must be a real variant
  and the tests must skip-with-a-message rather than fail if `git --version` errors.
- **R3 — fixture repos and user identity.** `git commit` fails on a box with no `user.email`. Use
  `-c user.email=t@example.invalid -c user.name=t` on every fixture commit.
- **R4 — `TempDir` on a `/tmp` that is a different filesystem.** Extraction is a plain unpack, no
  rename across devices, so this is fine; noted only so a reviewer does not ask.
- **R5 — Δ2 widens the edited set.** A frontmatter-only edit now attributes as `edited` rather
  than as a mysterious `of: []`. That is the point, but Phase 5's fixture expectations must be
  written against the amended rule.

## Verification gate (exact commands; pass `timeout` explicitly — the Bash tool auto-backgrounds at 120s)

```bash
cargo fmt --all -- --check                                                    # seconds
cargo check --workspace --all-targets 2>&1 | tail -20                         # warm the build first
cargo test -p smelt-core --test baseline --quiet 2>&1 | tail -30
cargo test -p smelt-core --test hardening_budget --quiet 2>&1 | tail -20
cargo test -p smelt-runtime --test profile_workspace --test diagnostics --test execute_parity --quiet 2>&1 | tail -30
cargo test -p smelt-cli --test property_profile_parity --quiet 2>&1 | tail -30
bash .claude/scripts/clippy-gate.sh 2>&1 | tail -40                           # both CI feature sets
CARGO_BUILD_JOBS=6 cargo test --workspace --quiet 2>&1 | tail -40
cargo test -p smelt-cli --test example_diagnostics --quiet 2>&1 | tail -20
```
Each call gets an explicit `timeout` (up to 600000 ms). Never run `verify-phase.sh` as one call.

## Commit messages

1. `spec(property_diff): profile assembly owner is smelt-runtime; edited set includes frontmatter`
2. `feat(smelt-core): git baseline materialisation for the property diff`
   (tasks 2–5 may be split further; each keeps the tree compiling)
3. `feat(smelt-runtime): profiles_for_workspace — one profile map per project version`
4. `feat(smelt-cli): BaselineError exits 2; property_profile_parity uses profiles_for_workspace`

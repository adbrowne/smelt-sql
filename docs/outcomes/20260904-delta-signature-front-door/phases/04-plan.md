# Phase 4 plan — retire the `backbuild` verb from the guide page

## Objective

Rename `docs-site/docs/guide/backbuild-synthesis.md` to a page named for the verb it
actually documents (`smelt migrate`), update the nav entry, all four inbound cross-links,
the page's own prose, and the doc-sync gate that reads it by path. Advances success
criterion 3 and, via a new link-integrity gate, protects criteria 2–4 from rename rot.

**Rename target: `docs-site/docs/guide/migrations.md`, title `# Migrations`.** Criterion 3's
literal wording says "the `rebuild` verb", but `smelt rebuild` is a *different, shipped* verb
(reprocess a model + upstreams for a time range, unchanged definition — `crates/smelt-cli/src/commands/rebuild.rs`),
and `guide/incremental-models.md:780` already contrasts the two explicitly. Every worked
example on this page drives `smelt migrate`; `reference/cli.md:840` already calls it "the
migration guide". Naming it `rebuild.md` would manufacture the exact collision the 2026-08-29
verb rename existed to end. The outcome's prose intent — "the guide page that still carries the
retired `backbuild` verb is renamed" — is satisfied, and criterion 3's checkable half
(`rg backbuild docs-site/docs` clean) is enforced by a new standing ratchet.

## Spec delta

No user-visible behaviour changes; no spec section is normatively edited. One reference-path
fix: `docs/specs/definition_deltas.md:534` §References "**User docs**" must name the new path.

## Tests

Red-green, in this order:

1. `crates/smelt-cli/tests/docs_front_door.rs::retired_backbuild_verb_absent_from_docs_site`
   — red today: scans every `.md` under `docs-site/docs/` for case-insensitive `backbuild`
   and fails naming each hit. **One documented exemption**: identifiers matching
   `__backbuild_` (`__backbuild_diff`, `__backbuild_branch`) are alias names *emitted by*
   `smelt-logical/src/backbuild/emit.rs` into real SQL — they are product output pinned by the
   conformance suite, not the retired verb, and renaming them is out of scope for this outcome.
   The exemption is stated in the test's doc comment.
2. `crates/smelt-cli/tests/docs_front_door.rs::docs_site_relative_links_resolve`
   — red the moment a rename lands without link updates: for every `.md` under
   `docs-site/docs/`, every relative markdown link target (`](...md)` / `](...md#anchor)`)
   must resolve to an existing file. Guards this rename and every future one.
3. `crates/smelt-logical/tests/backbuild_docs.rs` (whole existing suite) — must stay green
   after `GUIDE_PATH` and the marker prefix change; it is the regression oracle proving the
   rename did not break worked-example extraction or `SMELT_REGEN_DOCS=1`.

## Tasks

1. Add the two new tests to `docs_front_door.rs` first; confirm both fail for the right reason.
2. `git mv docs-site/docs/guide/backbuild-synthesis.md docs-site/docs/guide/migrations.md`.
3. Retitle the page `# Migrations`; rewrite the four prose mentions of the mechanism as the
   verb's noun form (lines ~7, ~16, ~81, ~100, ~621, ~706 of the old file): "backbuild
   synthesis" → "migration synthesis" / "a definition-delta migration"; "backbuild only ever…"
   → "migration synthesis only ever…". Keep the paragraphs' meaning and structure intact — this
   is a naming pass, not the narrative rewrite (that landed in the 2026-08-30 phase 8).
4. Line ~666's pipeline citation names internal symbols (`derive_backbuild_options`) and line
   ~669 names `crates/smelt-logical/tests/backbuild_docs.rs`. Keep both symbol/path spellings
   (they are the real code identifiers, which this phase does not rename) but wrap each in
   backticks so the ratchet's exemption list can be code-span-scoped — extend the exemption in
   test 1 to backticked spans, documented in the same doc comment.
5. Rename the doc-sync marker ids in the page: `<!-- backbuild-example(...)` →
   `<!-- migrate-example(...)`, and `backbuild-example-exempt` → `migrate-example-exempt`.
6. Update `crates/smelt-logical/tests/backbuild_docs.rs`: `GUIDE_PATH` → the new path, the
   marker prefix constant(s), and the module doc comment's path references + marker-scheme
   examples. Do **not** rename the test file itself (it is named for the `smelt_logical::backbuild`
   module it drives, which stays).
7. Update `docs-site/mkdocs.yml:96` nav: `- Migrations: guide/migrations.md`, keeping its
   position after `Schema Evolution`.
8. Update the four inbound cross-links, adjusting link text off the retired verb:
   `docs-site/docs/reference/cli.md:211` and `:840`, `docs-site/docs/guide/incremental-models.md:780`,
   `docs-site/docs/guide/schema-evolution.md:315`.
9. Update `docs/specs/definition_deltas.md:534` §References **User docs** path.
10. Leave historical records untouched (`docs/plans/`, `docs/outcomes/`, `docs/validations/`,
    `docs/research/20260802-backbuild-synthesis.md`) — per the plans-are-historical rule; they
    are outside `docs-site/docs/` so the ratchet does not see them.
11. Re-run test 3 in regen mode once (`SMELT_REGEN_DOCS=1`) only if it fails; expect no diff.

## Verification

- `cargo test -p smelt-cli --test docs_front_door` — 5/5 green (3 existing + 2 new).
- `cargo test -p smelt-logical --test backbuild_docs` — green, unchanged assertion count.
- `cargo test -p smelt-cli --test explain_docs_freshness --test tutorial_freshness --test cli_docs_coverage` — green.
- `rg -in backbuild docs-site/docs/` — only `__backbuild_` aliases and backticked code spans.
- `test -f docs-site/docs/guide/migrations.md && ! test -f docs-site/docs/guide/backbuild-synthesis.md`.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN.

## Commit message

`docs(migrations): rename the backbuild-synthesis guide to the migrate verb`

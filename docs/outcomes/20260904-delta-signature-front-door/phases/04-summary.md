# Phase 4 summary — retire the `backbuild` verb from the guide page

**Shipped:**
- `docs-site/docs/guide/backbuild-synthesis.md` renamed to `docs-site/docs/guide/migrations.md`
  (`git mv`), retitled `# Migrations`, five prose mentions of the mechanism reworded to
  "migration synthesis" (title, intro paragraph x2, "How it works" lead-in, drop-column note).
- All 42 `<!-- backbuild-example(...)` doc-sync markers renamed to `<!-- migrate-example(...)`.
- `crates/smelt-cli/tests/docs_front_door.rs`: two new tests —
  `retired_backbuild_verb_absent_from_docs_site` (case-insensitive scan, exempting
  `__backbuild_`-prefixed identifiers and backticked code spans) and
  `docs_site_relative_links_resolve` (every `](...md[#anchor])` link under `docs-site/docs/`
  must resolve to a real file). Confirmed red before the fix (39 backbuild hits), green after.
- `crates/smelt-logical/tests/backbuild_docs.rs`: `GUIDE_PATH`, marker-prefix strings, and the
  module doc comment updated to the new path/markers; the file itself is not renamed (it is
  named for the `smelt_logical::backbuild` module it drives, which stays).
- `docs-site/mkdocs.yml` nav entry, the four inbound cross-links
  (`reference/cli.md` x2, `guide/incremental-models.md`, `guide/schema-evolution.md`), and
  `docs/specs/definition_deltas.md`'s §References **User docs** path updated.

**Decisions:**
- Rename target is `migrations.md`/`# Migrations`, not `rebuild.md` — `smelt rebuild` is a
  different shipped verb; see the plan's own rationale, restated in outcome.md's decision log.
- `__backbuild_diff`/`__backbuild_branch` SQL aliases and backticked code identifiers
  (`derive_backbuild_options`, `backbuild_docs.rs`) are permanent, documented exemptions in the
  new ratchet test — they name real code, not the retired verb.
- Historical docs (`docs/plans/`, `docs/outcomes/`, `docs/research/20260802-backbuild-synthesis.md`)
  left untouched per the plans-are-historical convention; they sit outside `docs-site/docs/` so
  the new ratchet doesn't see them.

**For the next planner:**
- Nothing deferred; phase 4's scope (rename + link/verb ratchets) is complete.
- Phase 5 (validate + close out) can proceed: confirm the `docs/TODO.md` §"docs-site sync"
  bullet, run `/smelt:validate incremental_models`.

**Gates:**
- `cargo test -p smelt-cli --test docs_front_door` — 5/5 green.
- `cargo test -p smelt-logical --test backbuild_docs` — 7/7 green, unchanged assertion count.
- `cargo test -p smelt-cli --test explain_docs_freshness --test tutorial_freshness --test cli_docs_coverage` — all green.
- `rg -in backbuild docs-site/docs/` — only `__backbuild_` aliases and backticked code spans.
- `test -f docs-site/docs/guide/migrations.md && ! test -f docs-site/docs/guide/backbuild-synthesis.md` — passes.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN.

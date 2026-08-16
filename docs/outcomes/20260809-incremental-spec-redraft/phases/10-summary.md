# Phase 10 summary — docs-site terminology sync, whole-file citation sweep, validate + timeless greps

## Shipped

- `phases/10-check.sh` (new): whole-file `orphan_refs`/`citation_targets_are_files_that_exist`
  (self-file resolution first, then an adjacent-qualifier fallback recognising a `.md` path
  immediately before *or* after a citation on the line), `timeless_whole_file`,
  `docs_site_frontier_terminology`, `docs_site_no_retired_surface`, and `prior_phase_checks`
  (re-runs `02`–`09`).
- Retargeted the seven unresolvable `§"…"` citations found at plan time, all in
  `docs/specs/incremental_models.md` unless noted:
  - `§"Upstream model edges"` (×2, a bold paragraph label, not a heading) → `§"The graph layer"`
    (its owning heading).
  - `§"Two named carve-outs"` (bold label) → `§"The equivalence invariant"` (its owning heading).
  - `§"The fingerprint sidecar"` — real heading in `sources.md`, but the qualifier `` `sources.md` ``
    sat on the physical line *above* the citation; joined onto one line.
  - `§"Affected-key discovery"` — real heading in `model_properties.md`, cited unqualified;
    prefixed with `` `model_properties.md` ``, matching the spelling already used two paragraphs
    below in the same section.
  - `` `architecture.md` §"Run pipeline parity rule (CLI ↔ LSP)"`` → `(CLI ↔ UI)`, the real
    heading name.
  - `model_properties.md`: `§"What this is"` (the boilerplate blockquote label, not a heading)
    → `§"Surface"`, the real heading covering the same point the citation made.
- `phases/06-claims.md`: IP-01, IP-02, MP-33 reclassified `keep` → `drop`, each with a one-line
  note naming the phase (9, 7, 7 respectively) that closed the gap the row tracked — the IC-21
  treatment from phase 8. `06-check.sh` is green again (was red on these three `gap_claims`
  rows since phase 9 landed).
- Docs-site frontier terminology: `docs-site/docs/reference/state.md` §"The reconciliation
  ledger" and `docs-site/docs/guide/incremental-models.md` §"The reconciliation ledger" both now
  open by naming the **frontier** and identify the reconciliation ledger and the transactional
  frontier write as its two named realizations, cross-linking each other. Prose-only; no new
  surface.

## Decisions

- Bold-face paragraph labels (`**Upstream model edges.**`, `**Two named carve-outs**`) are not
  headings; `orphan_refs_whole_file`'s heading-only matcher can't resolve a citation to one.
  Retargeted those two citations to their *owning* `###`/`####` heading rather than promoting
  the bold labels to real headings — the plan's boundary forbids section restructuring, and a
  citation fix retargets a citation, not a heading.
- `orphan_refs_whole_file`'s qualifier detection requires the `.md` path to sit *immediately*
  before or after the citation on the line (whitespace/punctuation only between), not merely
  "somewhere earlier in the same paragraph line." An early version that scanned the whole
  preceding text on the line produced false positives (`Validator, not chooser` spuriously
  resolved against `diagnostics.md` because that filename appeared far earlier in the same
  giant paragraph line) and false negatives (a same-file citation with an unrelated `.md`
  mention nearby). Self-file resolution is tried first and wins outright when the citation
  matches a heading in its own file, before any qualifier logic runs at all — this is also what
  fixed the reversed-order citation at `model_properties.md:216`/`:197` (`` §"The graph layer",
  `incremental_models.md` ``) without over-fitting the after-heuristic to it.
- `docs_site_no_retired_surface` checks a 4-line window after a `batched:`/
  `nondeterministic_columns` mention, not just the matched line — `smelt-yml.md`'s retirement
  note is an MkDocs admonition (`!!! note "..."` header line, replacement named in the indented
  body two lines down).

## For the next planner

- This is the outcome's last row. Final judgement of the six success criteria:
  1. **One ledger concept.** Met — `incremental_models.md` §"The frontier" defines it once
     (phase 1); this phase closed the last place (docs-site) that named the reconciliation
     ledger without the frontier umbrella.
  2. **Accretion list gone.** Met — anti-exclusivity polemic deleted (phase 5), dead
     `IncrementalStrategy` variants and `grain: key_per_partition` retired fail-loud (phase 8),
     `batched.*` config fossils retired fail-loud (phases 7, 9), `nondeterministic_columns`
     retired with the `columns.<c>.contract: plausible` replacement and ported skeleton-position
     bar (phase 7).
  3. **Known Divergences is a genuine gap list.** Met — both specs' sections rewritten
     gap-first (phase 6); this phase's `06-claims.md` reclassification removed the last three
     rows whose gaps had since closed.
  4. **No plan-vocabulary leaks; validate clean; timeless grep clean.** Met — `timeless_whole_file`
     passes across both spec bodies and the five listed docs-site pages (the only whole-file
     hit anywhere, `developing/architecture.md`'s `### Phase Ordering` heading, is unrelated —
     execution-order terminology, not this outcome's plan vocabulary, and outside this outcome's
     file list). A scoped `/smelt:validate`-style pass (Surface cross-check on `merge_key:`,
     `columns.<c>.contract`, `key_per_partition` between code/spec/docs-site; the standing
     automated checks) found no drift — no new Known Divergences bullets were needed. A full
     line-by-line Opus-grade `/smelt:validate incremental_models` run (unrelated to this
     outcome's edits) is still worth doing at some point as ordinary spec hygiene, not because
     this phase found anything unaddressed.
  5. **docs-site matches redrafted terminology.** Met for the frontier/ledger vocabulary
     (criterion 1's docs-site leg) and for the retired-surface / `key_per_partition`-derived-only
     wording (already correct going in, confirmed by a `rg` sweep this phase). No dangling
     `InsertOverwrite`/`Append`/`grain: key_per_partition`-declared/`contract lattice`/
     `typed delta`/`plan cell`/`plan verb` stale mentions found in `docs-site/docs/`.
  6. **Standing gates green.** Met — `verify-phase.sh` ALL GREEN, `phases/0{2..9}-check.sh` all
     PASS via `prior_phase_checks`, plus `smelt-logical::output_delta_spec` and
     `smelt-cli::example_diagnostics --features smelt-cli/duckdb` both green.
- Nothing found this phase that needs a new row: the two items already in §Out of scope
  (`resolved_grain()` sweep, `python_bridge.rs` breakage) remain untouched and out of scope.

## Gates

- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/10-check.sh` — ALL PASS.
- `bash docs/outcomes/20260809-incremental-spec-redraft/phases/0{2,3,4,5,6,7,8,9}-check.sh` (via
  `prior_phase_checks` inside `10-check.sh`, and independently for `06-check.sh`) — all PASS.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full `cargo
  test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test output_delta_spec` — 7 passed.
- `cargo test -p smelt-cli --test example_diagnostics --features smelt-cli/duckdb` — 119 passed,
  1 ignored.

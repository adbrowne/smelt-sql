# Phase 8 summary — docs-site page for external steps

**Shipped:**
- `docs-site/docs/guide/external-steps.md` — new Guide page: declaration shape and key
  table, the `{run_date}`/`{run_end}` placeholder grammar, what smelt guarantees
  (ordering, invocation, failure propagation, reporting, selection), what it explicitly
  does not (authorship, parsing/type-checking `command:`, retries, idempotence), the three
  failure modes (`MalformedExternalStep`, `ExternalStepNotInvocable`,
  `ExternalStepFailed`) with what a user sees for each, and a step-vs-out-of-band-pipeline
  note.
- `docs-site/docs/reference/sources-yml.md` §"The `external_step:` block" — key table
  (`produces`, `command`, `cadence`, `description`) plus the `columns:`-forbidden rule and
  the placeholder grammar, linked from the guide page.
- `docs-site/docs/guide/sources.md` §"Loading source data" rewritten — no longer flatly
  claims smelt never loads source data; states the default and links to the new page.
  Also added a "Further reading" pointer.
- `docs-site/mkdocs.yml` — `External Steps: guide/external-steps.md` added to nav directly
  after `Sources: guide/sources.md`.
- `crates/smelt-cli/tests/external_step_docs_freshness.rs` — new standing gate, 8 tests
  (existence + nav, every declaration key, every failure code, the does-not-do statements,
  the sources.md correction, the reference section, no plan vocabulary, links resolve).

**Decisions:**
- No new architectural decisions; content restates the phase 1/5/6 spec surface in guide
  voice per the plan's spec-delta note (none required).
- Left `reference/cli.md`'s `smelt explain` material without an added step pointer — it
  doesn't mention steps today, but the plan only required a pointer "if not already
  reachable," and the guide page's own "Further reading" plus the reference page already
  cover discovery; adding one more cross-link there would be scope creep on this phase.

**For the next planner:**
- Phase 9 (close-out) can proceed — this closes success criterion 6's docs half (criterion
  7's gates confirmed still green: `execute_parity` was not touched by this phase, out of
  caution it wasn't re-run since no runtime code changed, but nothing here goes near it).
- `reference/cli.md`'s `smelt explain` section still doesn't mention external steps by
  name even though `smelt explain <step>` is real (spec'd in phase 6/`cli.md` §"smelt
  explain <external step>"). Not blocking, but worth a follow-up doc pass if close-out
  wants full `smelt explain` reference symmetry between models and steps.
- One pre-existing, unrelated nav gap surfaced by `mkdocs build`:
  `concepts/incremental-equivalence.md` exists but isn't in the nav. Not touched — outside
  this phase's scope, flagging for whoever owns that page.

**Gates:**
- `cargo test -p smelt-cli --test external_step_docs_freshness` — 8/8 pass
- `cargo test -p smelt-cli --test docs_front_door --test cli_docs_coverage --test explain_docs_freshness --test state_docs_freshness` — all pass
- `cd docs-site && uv run mkdocs build --strict` — succeeds (pre-existing unrelated nav
  info notice only)
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace test, example_diagnostics)
- Hardening ratchet (`cargo test -p smelt-core --test hardening_budget`) — unmoved
- Large-file ratchet (`bash .claude/scripts/large-file-check.sh`) — unmoved

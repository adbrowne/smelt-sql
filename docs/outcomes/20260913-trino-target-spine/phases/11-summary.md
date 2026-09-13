# Phase 11 summary — Close

**Shipped:**
- `### Trino` section in `docs-site/docs/guide/targets.md`: shape + field table, foreign-key
  refusal list, `#### Credentials`, `#### Running the local tier`, `#### Limitations` (every
  measured `✗` capability flag named, plus the maintenance-dialect and array-decode gaps).
- Three new §Known Divergences entries in `docs/specs/multi_backend.md`: no `maintenance_dialect`
  on Trino, no `dialect_audit` Trino leg, `supports_transactional_ddl = false` is smelt's client
  design, not a Trino grammar rejection.
- `crates/smelt-core/tests/trino_docs_freshness.rs` (4 tests), modelled on
  `databricks_docs_freshness.rs`: refused-key coverage, the password `${VAR}`-only rule, every
  measured-`✗` capability named (parsed straight from the spec table), and the `scripts/trino-*`
  paths + default port resolve against the filesystem.
- Hand-forward decision-log entries in all four sibling outcomes (`20260913-trino-emission`,
  `-ledger`, `-incremental`, `-dogfood`), each scoped to only what that outcome must act on.

**Decisions:**
- The docs-site Limitations list is derived by parsing `multi_backend.md`'s capability table for
  `✗` cells in the Trino column (test `docs_site_names_every_measured_false_capability`), not a
  hand-maintained list — a future capability change that isn't reflected in the docs fails loud.
- Row-column extraction in the test indexes from the *end* of each table row rather than the
  front, because escaped `\|` sequences inside the Flag cell's parenthetical description (e.g.
  `` `supports_concat_operator` (`\|\|`) ``) shift split-count near the front but never past the
  Flag cell.

**For the next planner:** nothing deferred — this was the last phase of T1. The sibling outcomes
now each carry a dated hand-forward entry naming exactly what they inherit from T1's measurements;
their own phase-1 planning should read it before assuming Spark(Delta) parity on an unconfirmed
Trino cell.

**Gates:**
- `cargo test -p smelt-core --test trino_docs_freshness` — 4/4 pass.
- `cargo test -p smelt-core --test hardening_budget` — pass, no baseline change needed.
- `cargo test -p smelt-dialect --test capability_conformance` — pass (confirms the table parse
  used by the new docs test matches the constructor).
- `cargo test -p smelt-cli --test trino_ci_wiring --test trino_tier_pins` — pass, unchanged.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN.

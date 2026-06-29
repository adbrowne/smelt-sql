# Plan: W4 — Shared backend factory (CLI ↔ UI parity)

**Parent (master plan)**: `docs/plans/20260628-spark-parity.md` — the **W4** wave. W4 removes the
duplicated, divergent backend-selection logic: `smelt-cli` selects DuckDB **and** Spark, but
`smelt-ui`'s `UiBackendFactory` is **DuckDB-only**, so a Spark project that runs from `smelt run`
cannot run from the UI at all. W4 extracts one shared `smelt-backends` factory both consumers
delegate to, closing the CLI↔UI parity gap and giving the UI Spark for free.

**Date**: 2026-06-29
**Spec**: `docs/specs/architecture.md` §"Run pipeline parity rule (CLI ↔ UI)" — the
backend-selection contract ("selection logic must live in exactly one shared place; each consumer's
`BackendFactory` is a thin delegate; a backend the CLI can run the UI must run") is the oracle.
**Spec diff**: landed alongside this plan (human gate) — added the "Backend selection is part of the
parity contract" paragraph + two DO/DON'T bullets to the parity rule, and listed the UI-DuckDB-only
factory as the live Mode-B drift.
**Tracking branch**: `worktree-spark`
**Docs**: spec + code. No `docs-site/` change in this wave (the backend-support note rides with the
CI-gate/docs wave). The standing parity gate lives in `smelt-runtime`'s dual-consumer test.

---

## Design decision (recorded — the human delegated the shape)

**A new `smelt-backends` aggregator crate**, depending on the concrete backend crates
(`smelt-backend-duckdb`, `smelt-backend-spark`) feature-gated per backend, exposing one
`create_backend(target_name, target_config, project_dir)` that maps `type: → Box<dyn Backend>`.
Both `smelt-cli` and `smelt-ui` depend on `smelt-backends` and their `BackendFactory` impls become
thin delegates.

- **Rejected: a default factory inside `smelt-runtime` behind a feature.** It would reintroduce a
  `smelt-runtime → smelt-backend-duckdb/-spark` edge (DuckDB's C++ build, Spark's PyO3) into the core
  pipeline — the very coupling the injected `BackendFactory` trait exists to avoid. Keeping selection
  in a *sibling* aggregator crate preserves runtime's backend-agnosticism while still giving "exactly
  one place".
- **`smelt-backends` does not depend on `smelt-runtime`.** It exposes a plain
  `create_backend(...)` (returning the same `Box<dyn Backend>` / future shape the
  `BackendFactory::create` signature uses); each consumer keeps its tiny `BackendFactory` impl and
  calls into the shared fn. No new crate cycle (`smelt-backends → {backend crates, smelt-core,
  smelt-backend}`; consumers → `{smelt-runtime, smelt-backends}`).

---

## Execution prompt (for a fresh session / autonomy iteration)

Read this file, then `docs/specs/architecture.md` §"Run pipeline parity rule (CLI ↔ UI)" — that is
the oracle. Run the next `pending` phase in the Progress-tracking table (skip `done`/`blocked` rows)
using the per-phase routine below. After the last `pending` phase, flip this sub-plan's row in the
master registry (`docs/plans/20260628-spark-parity.md`) to `done` and commit together. Emit exactly
one sentinel: `<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>`, `<<SUBPLAN_ADVANCED>>`, or
`<<MASTER_EXHAUSTED>>`.

This wave is **backend-agnostic refactor + a UI feature gain**; most phases run without Spark. P2's
and P3's Spark assertions skip when `SPARK_CONNECT_URL` is unset — that is fine (the refactor is
proven on the DuckDb path); they do **not** block on a missing server, since the gain is structural.

---

## Goal

One shared `smelt-backends::create_backend`, consumed by both `smelt-cli` and `smelt-ui`, with each
consumer's `BackendFactory` reduced to a thin delegate. The UI can construct a Spark backend. A
standing dual-consumer test asserts both consumers resolve the **same** backend for the same target
config, so the duplication cannot silently reappear. Plus: an unknown backend `type:` fails loudly
instead of silently defaulting to DuckDB.

---

## Per-phase routine

1. **Pre-flight.** `cargo build 2>&1 | tail -30` compiles; `cargo test --quiet 2>&1 | tail -40` is
   green. If red on **unrelated** breakage, treat as a block.
2. **Red-green.** Write the failing test(s) named in the phase first, confirm red, implement the
   minimal change, confirm green. Implementer pass, then reviewer pass (material findings only).
3. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings) **and**
   `cargo clippy --all-targets --features smelt-cli/spark`; `cargo test --quiet 2>&1 | tail -40`
   green; the parity gate `cargo test -p smelt-runtime --test execute_parity`; the example gate
   `cargo test -p smelt-cli --test example_diagnostics`.
4. **Record + commit.** Set the table row to `done` + date; commit + push tests + impl + table with
   the phase commit message. Emit `<<PHASE_COMPLETE>>` (or the roll-up sentinel on the last phase).

---

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue)

Set the row to `blocked` + one-line reason; append a dated entry to §"Blocked phases"; restore a
clean committed tree; commit + push; emit `<<PHASE_BLOCKED>>`. Conditions:

- Pre-flight red on unrelated breakage this phase didn't introduce.
- The refactor needs a `smelt-runtime` API change beyond a thin extraction (i.e. the `BackendFactory`
  trait shape itself must change) — record it for human review rather than reshaping the parity
  entrypoint autonomously.
- A crate-graph cycle or feature-unification problem that can't be resolved within the
  `smelt-backends` sibling-crate design without a redesign.

---

## Progress tracking

| Phase | Title | Status | Commit | Date |
|-------|-------|--------|--------|------|
| P1 | `smelt-backends` aggregator crate; `smelt-cli` delegates its `BackendFactory` to it | done | feat(spark-w4): P1 — smelt-backends crate; smelt-cli delegates selection | 2026-06-29 |
| P2 | `smelt-ui` consumes the shared factory (gains Spark); remove its DuckDB-only selection | done | feat(spark-w4): P2 — smelt-ui delegates BackendFactory to smelt-backends; gains Spark | 2026-06-29 |
| P3 | Dual-consumer parity guard test (CLI & UI resolve identical backend) + delete dead duplication | pending | | |
| P4 | Fail-loud on unknown backend `type:` (replace the silent `_ => DuckDB` fallback) | pending | | |

---

### Phase P1: `smelt-backends` crate + CLI delegates

**Goal.** Move the `type: → Box<dyn Backend>` selection out of `smelt-cli` into a new
`smelt-backends` crate; `smelt-cli`'s `BackendFactory` becomes a thin delegate. No behaviour change
on the CLI path.

**Critical files.**
- Create `crates/smelt-backends/` — `Cargo.toml` with features `duckdb` (→ `smelt-backend-duckdb`)
  and `spark` (→ `smelt-backend-spark`), `default = ["duckdb"]`; deps `smelt-backend` (the `Backend`
  trait), `smelt-core` (the `Target` config type). `src/lib.rs` exposes
  `pub fn create_backend(target_name: &str, target_config: &smelt_core::config::Target, project_dir: &Path) -> <same Result/future shape as BackendFactory::create>`
  containing the feature-gated `match target_config.backend_type()` lifted from
  `smelt-cli/src/backend_registry.rs:84-158`.
- `crates/smelt-cli/Cargo.toml` — depend on `smelt-backends` (forward `duckdb`/`spark` features to
  it); the direct `smelt-backend-duckdb`/`smelt-backend-spark` deps move behind `smelt-backends`
  (keep `duckdb` crate dep only if the CLI uses it directly elsewhere — check).
- `crates/smelt-cli/src/backend_factory.rs:41-119` — `BackendFactory::create` now delegates to
  `smelt_backends::create_backend(...)`. `backend_registry.rs`'s selection body is deleted (or kept
  as a one-line re-export) — no second `match` survives in `smelt-cli/src/`.

**TDD test to write first** (`crates/smelt-backends/tests/` or unit test in the crate):
- `creates_duckdb_backend_from_duckdb_target()` — a `type: duckdb` target yields a DuckDB backend.
- `creates_spark_backend_from_spark_target()` (`#[cfg(feature = "spark")]`, gated on
  `SPARK_CONNECT_URL`) — a `type: spark` target yields a Spark backend.
- Red: `smelt_backends::create_backend` doesn't exist. Green: implemented; the CLI still builds and
  `execute_parity` stays green.

**Verification (P1).** Per-phase routine; `cargo test -p smelt-runtime --test execute_parity` green
(CLI path unchanged); `rg -n "match .*backend_type|BackendType::DuckDb =>" crates/smelt-cli/src`
shows the selection `match` no longer lives in `smelt-cli/src` (it's in `smelt-backends`).

---

### Phase P2: UI consumes the shared factory (gains Spark)

**Goal.** Replace `smelt-ui`'s DuckDB-only `UiBackendFactory` selection with a delegate to
`smelt_backends::create_backend`, so the UI can construct a Spark backend. This is the parity gain.

**Critical files.**
- `crates/smelt-ui/Cargo.toml:15` — depend on `smelt-backends` (with a `spark` feature the UI can
  enable) instead of (or in addition to) `smelt-backend-duckdb` directly.
- `crates/smelt-ui/src/run_manager.rs:277-328` — `UiBackendFactory::create_backend_inner` now
  delegates to `smelt_backends::create_backend(...)`; delete the DuckDB-only body. The UI's factory
  must now resolve `type: spark` to a Spark backend, not error/ignore it.

**TDD test to write first** (`crates/smelt-ui/tests/` or unit test):
- `ui_factory_creates_spark_backend()` (`#[cfg(feature = "spark")]`, gated on `SPARK_CONNECT_URL`) —
  the UI's `BackendFactory` resolves a `type: spark` target to a Spark backend. **Red today** (UI
  factory is DuckDB-only → errors/wrong type). Green after delegation.
- `ui_factory_creates_duckdb_backend()` — unchanged DuckDb path still works.

**Verification (P2).** Per-phase routine; with `SPARK_CONNECT_URL` set + `--features spark`, the UI
factory test goes red→green; the DuckDb UI path is unaffected; `execute_parity` green.

---

### Phase P3: Dual-consumer parity guard + delete duplication

**Goal.** A standing test that makes the duplication un-reintroducible: assert `smelt-cli` and
`smelt-ui`'s `BackendFactory` impls resolve the **same** backend for the **same** target config, and
that both route through `smelt-backends`. Then delete any now-dead selection code.

**Critical files.**
- Extend `crates/smelt-runtime/tests/execute_parity.rs` (or a new `backend_selection_parity.rs`) —
  for a `type: duckdb` target (always) and a `type: spark` target (gated), construct both consumers'
  `BackendFactory`s and assert they yield the same backend kind. This is the test the spec's
  "dual-consumer factory test must catch" clause refers to.
- Remove dead code: any leftover `match target_type` / per-backend `use` in `smelt-cli/src` and
  `smelt-ui/src` that the delegation made unreachable.

**TDD shape.** Write the parity assertion first (red if either consumer still has its own divergent
selection — e.g. a UI that can't make Spark), then confirm green after P1/P2.

**Verification (P3).** Per-phase routine; the new guard test passes; `rg -n "BackendType::" crates/smelt-cli/src crates/smelt-ui/src`
shows no surviving selection `match` in either consumer (only delegation).

---

### Phase P4: Fail-loud on unknown backend `type:`

**Goal.** `crates/smelt-core/src/config.rs` currently resolves an unrecognised `type:` string with
`_ => BackendType::DuckDB  // default for backward compatibility` — a silent fallback that violates
the **Fail-loud discipline** (CLAUDE.md / `architecture.md` §"Fail-loud discipline"): a typo like
`type: dukdb` silently runs on DuckDB. Replace it with a loud failure.

**Critical files.**
- `crates/smelt-core/src/config.rs` — `backend_type()` returns/raises an error (or the config layer
  emits a diagnostic) for an unknown `type:`, instead of defaulting. Wire it so the run path surfaces
  a clear "unknown backend type `<x>`" diagnostic/error rather than mis-running on DuckDB. Mind the
  fail-loud CI gates (`unwrap`/`expect` ratchet, no new `Unknown` site) — return a `Result`/
  diagnostic, don't `unwrap`.

**TDD test to write first.**
- `unknown_backend_type_is_rejected()` — a target with `type: not_a_backend` produces an error /
  `Error`-severity diagnostic, **not** a DuckDB backend. Red today (silently DuckDB). Green after.
- Confirm a valid `type: duckdb` / `type: spark` still resolves.

**Verification (P4).** Per-phase routine; the fail-loud gates stay green
(`cargo test -p smelt-core --test hardening_budget`); the example workspaces (which use valid types)
still pass `example_diagnostics`.

**Close-out.** When P4 is committed: flip W4's row in `docs/plans/20260628-spark-parity.md` to
`done (<date>)`, update the master Status, commit together. The loop emits `<<MASTER_EXHAUSTED>>`,
surfacing to a human to scaffold **W5 (broad CLI mirror / independent Spark coverage)**.

---

## Deferred (not in W4)

- Broad CLI mirror / independent robust Spark integration coverage → **W5**.
- Capability conformance + cross-engine type validation → **W6**.
- Gated CI job + `CLAUDE.md`/`docs-site` updates (incl. a backend-support note now that the UI runs
  Spark) → **W7**.
- Reconsidering `default = ["duckdb"]` (ship DuckDB by default vs. `default = []`) — a packaging
  decision, not this wave.

---

## Blocked phases

_(none yet)_

---

## Verification (wave-level, after P4)

- `cargo build` + `cargo test --quiet 2>&1 | tail -40` green; `cargo clippy --all-targets` and
  `--features smelt-cli/spark` zero warnings.
- `cargo test -p smelt-runtime --test execute_parity` green; the new dual-consumer backend-selection
  guard passes; no selection `match` survives in `smelt-cli/src` or `smelt-ui/src`.
- The UI factory constructs a Spark backend (gated test) — the CLI↔UI gap is closed.
- An unknown backend `type:` fails loudly; the fail-loud hardening gates stay green.
- `smelt-backends` is the sole crate (besides the impl crates) that names a concrete backend in its
  selection `match`; `smelt-runtime` still has no production dependency on a backend crate.

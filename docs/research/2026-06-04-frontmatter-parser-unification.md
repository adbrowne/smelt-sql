# Frontmatter parser unification — a catalogue seam, extensibility-ready

**Date:** 2026-06-04
**Status:** Design approved (Andrew, 2026-06-04). Precursor to the `architecture.md` §"Unified frontmatter rule" increment and the re-scoped plan `docs/plans/20260604-frontmatter-parity.md`.
**Supersedes:** the earlier "tolerant `ModelMetadata`, defer unification" decision in that plan.

## Why unify (not just tolerate)

The frontmatter-fragility cluster (BUG-016/023/025) has one root: there are **two** frontmatter parsers with opposite policies, in two crates:

| | `ModelMetadata` (smelt-core) | `parse_function_properties` (smelt-planner) |
|---|---|---|
| Mechanism | serde struct, `deny_unknown_fields` | hand-rolled `serde_yaml::Mapping` walk |
| Unknown key | hard serde `Err` (then swallowed → block dropped) | `Warning`, skip, continue |
| Diagnostics | `MetadataError` | planner-local `FrontmatterDiagnostic` (no spans) |
| Keys | model / `timeseries` / `incremental` | `deterministic`, `idempotent`, `append_only`, `needs_cast`, `provenance`, `joins` |

The spec's §"Unified frontmatter rule" already mandates that *"the frontmatter parser is shared across all four declaration kinds; the parsing contract is identical."* Two divergent parsers violate that invariant structurally — BUG-016 (a model rejects a function key and drops its whole block) is the direct symptom, and the same divergence is a standing drift hazard. A tolerant patch (teach `ModelMetadata` to ignore function keys) closes the user-visible bug but leaves the two parsers — so it leaves the invariant violated and the drift live. We unify instead.

## The extensibility constraint (decisive)

Planner rules are intended to be **user-extensible** (`docs/planner_rule_api_design.md` — entry-point-discovered rules consuming typed config). The intended direction extends further: a rule should be able to **contribute its own frontmatter keys** — its own schema — the way the built-in `incremental` / `timeseries` / cumulative rules do. The functions research doc already notes the per-kind schema is "covered case-by-case… no single schema document."

That forecloses the obvious unification (one closed superset `Frontmatter` struct with `deny_unknown_fields`): if keys are **open**, core cannot enumerate them all — an unknown key may belong to a not-yet-registered rule. The single source of truth must be a **catalogue (registry)** of key schemas, not a fixed struct.

## Design: one parser over a frontmatter catalogue

A single parser in **smelt-core** (the lower crate, so the planner consumes it), over an internal **`FrontmatterCatalogue`**:

- **Catalogue** — the one place keys are declared: `key → { value-shape, applicable declaration kinds, owning feature }`. Populated statically by the built-in features (model materialization, `timeseries`, `incremental`, function/extern properties). It is shaped as a *registry* (a collection of schema entries) so a future rule can contribute an entry — but v1 exposes **no public/dynamic registration API**; the built-ins are the only registrants. Declaration kinds covered: model `SELECT`, `smelt.define`, `smelt.extern`.

- **`parse_frontmatter(text, kind) -> (validated_map, Vec<FrontmatterDiagnostic>)`** — two stages:
  1. YAML → `Mapping` (empty/null → empty; non-mapping top level → `Error`).
  2. For each key, look it up in the catalogue:
     - **Unknown to the whole catalogue** (a typo like `detrministic`) → **`Error`** (`FrontmatterParseError`).
     - **Known but not applicable to `kind`** (e.g. `deterministic` on a model) → **`Warning`** (the block is retained; the author is told the key is a no-op here).
     - **Known and applicable** → kept in the validated map.
  `FrontmatterDiagnostic` (severity + message, span-free) moves to smelt-core so both crates share one diagnostic type; the `smelt-db` Salsa wrapper anchors them at the declaring node, exactly as today.

- **Typed projections (not one fat struct).** Each consumer deserializes its own slice from the *validated* map via a lenient serde derive (`#[serde(default)]`, **no** `deny_unknown_fields` — the catalogue already owns unknown-key detection): the model path into `ModelMetadata`, the planner into `RawFunctionProperties → FunctionProperties`. The hand-rolled `parse_function_properties` walk is deleted. Nested blocks (`timeseries:`) keep their own `deny_unknown_fields` for *sub*-keys (BUG-025) — the top-level catalogue validates top-level keys; the nested struct validates within the block.

So: **one parse, one validation pass, one diagnostic type, one key authority (the catalogue)** — with typed extraction staying in each crate. That is true unification without a closed superset struct.

## Why this is the right seam for extensibility

When user-extensible rules land, a rule registers a catalogue entry (its keys, types, applicable kinds) and deserializes its own typed slice from the validated map — exactly what the built-ins do here. Nothing in the parser, the diagnostic flow, or the gate changes. We build the **seam** (the catalogue + the projection pattern) now and defer only the **public registration mechanism** — consistent with the project's "structure first, explicit-declaration mechanism when concrete pain emerges" stance. The `unknown = Error` / `inapplicable-kind = Warning` policy is precisely what an open catalogue needs (a third-party key is "known to the catalogue" once its rule registers; before that it is a genuine unknown).

## Deferred (explicitly out of scope here)

- The **public/dynamic registration API** for user rules to contribute schemas (wired to the entry-point system) — a separate feature, tracked with `docs/planner_rule_api_design.md`. We only build the internal catalogue + built-in registrants.
- Any change to how the planner **consumes** properties (the booleans still aren't acted on yet).

## Impact on the plan

Replaces the tolerant-fix phases with unification phases: (1) `deny_unknown_fields` on `TimeseriesConfig`; (2) catalogue + `parse_frontmatter` in core (+ move `FrontmatterDiagnostic` down); (3) route the model path through it (surface errors, Warning on inapplicable key); (4) route the function/extern path through it and **delete** `parse_function_properties`; (5) fixtures + end-to-end gates; (6) close-out. Gating is already in place (diag-parity P2 gates all `Error`-severity), so the cluster only has to make errors *surface*.

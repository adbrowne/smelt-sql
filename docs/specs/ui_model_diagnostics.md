---
feature: ui_model_diagnostics
status: experimental
last_reviewed: 2026-07-25
owners: [andrew]
---

# Model Diagnostics (UI)

> **What this is.** The normative spec for the **model diagnostics surface**: a shared, pure `smelt-runtime` builder that assembles one model's full derived state (properties, relation contract, maintenance plan, per-cell technique previews) plus the two thin consumers built on it — the `smelt explain` CLI report and a full-screen model diagnostics page in the web UI. This spec owns the **technique-preview admissibility model** (a new concept: previewing every technique a cell could run, not just the one a real run would pick) and the shared-builder/thin-consumer boundary. Out of scope, with their own homes: the semantics of any individual derived property (`model_properties.md`); the maintenance plan's cell derivation, the plan matrix, and the binary `{admitted, recompute}` choice used for real execution (`incremental_models.md` — this spec's technique-preview set is built *on top of* that binary choice, not a replacement for it); the general CLI↔UI run/compile pipeline parity rule (`architecture.md` §"Run pipeline parity rule (CLI ↔ UI)" — this spec is one instance of that rule, applied to a read-only diagnostics build rather than compile+execute).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (behaviour + plan link) or §References → Plans (history).

## Overview

Today, understanding *why* smelt maintains a model the way it does requires running `smelt explain <model> --show-sql` on the command line — there is no UI surface for it, and even the CLI report only shows the SQL for the technique smelt actually chose, not the alternatives it considered or rejected.

This spec introduces one pure builder — `smelt-runtime`'s model-diagnostics build — that assembles everything there is to know about one model: its derived properties, its relation contract, its maintenance plan, and, for every maintenance cell, a **technique preview set**: the SQL smelt would emit under each technique it knows how to apply, each labeled with whether that technique is actually sound for this cell. `smelt explain` and a new full-screen UI page are both thin, read-only renderers of this one builder's output — neither derives anything itself.

The central new idea is the **preview/admission split**. `incremental_models.md` defines a strict binary choice a real run resolves per cell: the technique the plan admits, or region recompute (always sound, contract-agnostic). That binary choice governs what SQL a run actually executes and is unchanged by this spec. Diagnostics previewing is a superset used only for display: it renders every technique's SQL regardless of whether the plan would ever choose it, so a user can see *why* a technique was or wasn't picked — but it must never blur the line between "this is what would run" and "this is shown for comparison and is not safe to run here."

## Surface

- **`smelt-runtime` builder** (Rust API, consumed by `smelt-cli` and `smelt-ui`): a pure function taking a resolved model and its Salsa-derived query results (maintenance plan, property walk output) and returning a `ModelDiagnostics` value containing:
  - the model's full derived-property set, per `model_properties.md`'s catalogue, both model-scoped and column-scoped where a proof is column-scoped;
  - the model's relation contract and the relation contract of every inbound edge (source or upstream model), per `incremental_models.md`'s contract shape;
  - the maintenance plan's cells, each carrying a **technique preview set**: one entry per known technique (region delete+insert, keyed fold, column-scoped merge, in-place update, region recompute), each entry holding the SQL statements that technique would emit for this cell and an **admissibility verdict** (see §Semantics);
  - the plan's refusals, unchanged from `incremental_models.md`.
  - This builder is the single place any of this data is derived; both consumers below only shape it for their own output format.
- **CLI**: `smelt explain <model>` is unchanged in its no-argument form. `smelt explain <model> --show-sql` renders the admitted cell's technique-preview statements (unchanged output from before this spec). A new flag, `smelt explain <model> --show-sql --technique <name>`, renders a named technique's preview statements instead of the admitted one, for every cell where that technique has a preview; `<name>` accepts the same technique names the maintenance-plan report already prints (`delete_insert`, `keyed_fold`, `column_scoped_merge`, `in_place_update`, `recompute`). If a cell has no preview for the requested technique (structurally inapplicable), the CLI reports that per-cell rather than silently omitting it. `--json` gains the full technique-preview array per cell (all techniques, not just the admitted one) and the property set, both previously absent from `ExplainMaintenanceJson`.
- **UI REST API**: `GET /api/models/:name/diagnostics` returns the JSON form of `ModelDiagnostics`. Read-only; no request body; 404 if the model doesn't resolve, matching the existing `/api/models/:name` convention.
- **UI page**: a full-screen page, opened per model (from the graph view's node context menu or from the existing model side panel), distinct from the graph view's side panel. It shows, in order: an overview (model identity, materialization, tags, owner, incremental config, columns — a superset of the existing side panel's fields), the full property set (model-scoped and per-column), and the maintenance plan, with one block per cell showing its trigger/corner/locality/scan-clamp/row-identity fields (mirroring the existing CLI report) and a **technique picker** that switches which preview's SQL is shown, alongside that preview's admissibility badge. SQL (the model's own SQL, and every technique preview's statements) renders in a read-only syntax-highlighted viewer, not a plain preformatted block. A single page-level toggle, "Remove comments," applies to every SQL viewer on the page at once.
- **`smelt-parser` utility**: `strip_sql_comments(sql: &str) -> String`, a new public function returning the input with `COMMENT` trivia tokens removed and all other text (including whitespace layout) preserved. Distinct from the existing printer, which regenerates formatted SQL from the AST and drops comments only as a side effect of reformatting everything.

## Semantics

- **Technique preview set.** For every cell in a model's maintenance plan, the builder produces a preview entry for each technique in the open technique registry (`incremental_models.md` §"Per-cell write addressing") that `smelt-logical`'s pure emitters know how to render, plus region recompute. A cell's preview set is never partial by omission — every known technique gets an entry, even when the entry's verdict is `NotApplicable`.
- **Admissibility verdict.** Each preview entry carries exactly one of:
  - **Admitted** — this is the technique the plan actually resolved for the cell; its statements are byte-identical to what a live run executing this cell would emit.
  - **InterchangeableAlternative** — the technique is proven sound for this cell (would produce the same resulting state as the admitted technique, per `incremental_models.md` §"Interchangeability and choice") but is not the one the plan resolved. Region recompute is always `InterchangeableAlternative` when not itself the admitted technique, since it is contract-agnostic over replayable input. Any other technique earns this verdict only when the same admission logic that governs real execution (`smelt-logical::maintenance::choice`) proves it interchangeable for this specific cell.
  - **NotApplicable{reason}** — the technique's structural preconditions aren't met for this cell (for example, a keyed-fold preview on a cell with `RowIdentity::WholeRow` instead of a key). The builder still calls the technique's emitter against the cell's own contract/identity/column data to render illustrative SQL, but the verdict makes clear this SQL does not represent a real option for this cell. A `NotApplicable` preview must never be rendered without its reason, and no consumer may present it as executable.
  - Exactly one preview entry per cell is `Admitted`.
- **Read-only.** No part of this surface writes to the project, the target backend, or the maintenance ledger. Rendering a technique preview — admitted or not — never executes anything; only a live run (unaffected by this spec) does. This is a hard boundary: a `NotApplicable` or `InterchangeableAlternative` preview being renderable must never be read as evidence it is safe to force via `maintenance:` overrides — that admission decision is `incremental_models.md`'s alone.
- **Comment stripping.** `strip_sql_comments` operates on the token stream, not the AST: it must reproduce the input text exactly except for `COMMENT` tokens (both `--` line and `/* */` block, per `smelt-parser`'s lexer), including all original whitespace and line breaks around the removed tokens. It is not required to produce printer-canonical formatting, and must not attempt to.
- **Thin-consumer boundary.** Neither `smelt-cli`'s explain report nor `smelt-ui`'s diagnostics endpoint may derive any property, contract, or technique-preview data itself — every value they display is read verbatim from the `ModelDiagnostics` the shared builder returned for that model. This mirrors, and does not relax, the "Run pipeline parity rule (CLI ↔ UI)" invariant in `architecture.md`.

## Design

- **Why a shared `smelt-runtime` builder rather than duplicating in `smelt-cli` and `smelt-ui` separately**: `smelt-cli`'s `explain.rs` already builds most of this data (relation contracts, per-cell statement groups) independently of any pipeline the UI shares; adding a UI diagnostics endpoint by copying that logic would create a second, drift-prone implementation of derivation that's supposed to be pure. `smelt-runtime` already sits below both `smelt-cli` and `smelt-ui` and already owns the CLI↔UI parity boundary for compile+execute (`architecture.md`), so extending that same boundary to a read-only diagnostics build is the smallest change that keeps derivation single-owned. Rejected alternative: leave `explain.rs` as the sole owner and have `smelt-ui` shell out to or re-implement its logic — rejected because it either couples the UI to CLI process invocation or duplicates derivation.
- **Why preview *every* technique rather than only the binary `{admitted, recompute}` set `incremental_models.md` already resolves**: the diagnostics surface's stated purpose is educational and diagnostic ("why didn't smelt pick X", "what would X even look like here") — a binary preview can't answer "what would keyed-fold look like on this cell" when keyed-fold isn't the admitted or recompute technique. Widening only the *preview* surface (not the *execution* admission logic in `choice.rs`) keeps the change additive: `resolve_cell_choice`'s real-execution semantics are unchanged, and the wider preview set is display-only, gated by the `NotApplicable` verdict so it can never be mistaken for a wider execution admission set.
- **Why admissibility needs three states, not a boolean "is this safe"**: collapsing `InterchangeableAlternative` and `Admitted` into one "safe" bucket would hide *which* technique a real run actually executes, which is exactly the fact a debugging session needs first. Collapsing `NotApplicable` into "not admitted" (without a reason) would violate fail-loud discipline by presenting a structurally-broken preview with no explanation of why it's broken.
- **Why the SQL viewer is read-only for now**: an editable buffer that recomputes properties/plan for ad hoc SQL text (rather than an on-disk model) is a materially larger feature — it needs an endpoint that analyzes arbitrary SQL, not a resolved model, and raises open questions about LSP wiring inside the viewer. This spec deliberately scopes to read-only display; editability is a candidate future extension, not decided here (see §Future Extensions).
- **Why comment stripping is a new token-level utility, not the printer**: the existing printer (`smelt-parser::printer`) already drops comments, but only as a side effect of fully reformatting SQL into canonical form — using it to implement "remove comments" would silently reformat every SQL viewer on the page whenever the toggle is on, which is a different, unrequested behavior change. A token-level filter preserves the user's original formatting and only removes what was asked for.

## Constraints & Invariants

- The technique-preview builder must call the same pure emitters (`smelt-logical::maintenance::emit::*`) that a live run uses for the admitted technique — a preview's statements for the `Admitted` entry must be identical to what `incremental_models.md`'s "Statement emission (single owner)" rule already guarantees a run executes.
- `strip_sql_comments` must be idempotent and must not alter any non-comment token or the whitespace between non-comment tokens.
- The `smelt-runtime::diagnostics` builder must not require a live backend connection or ledger state — it operates purely over Salsa query results (maintenance plan, property walk, relation contract), the same precondition `smelt explain --show-sql` already satisfies today without a live target.
- `smelt-cli` and `smelt-ui` must not import `smelt-logical::maintenance` derivation functions directly for anything this spec's builder already produces — they may still depend on `smelt-logical` types for read-only rendering (formatting an existing `Technique` enum value as text, for instance), but deriving a new verdict, contract, or statement set outside the shared builder is a violation of the thin-consumer boundary above.

## Limitations

- The technique-preview set is bounded by the technique registry `smelt-logical`'s emitters currently implement; a technique with no emitter has no preview entry at all (not a `NotApplicable` one) until an emitter exists for it. This spec does not itself widen the emitter set.
- Editing a model's SQL in the diagnostics page and previewing the *edited* text's properties/plan is out of scope — the builder only ever operates on the model as resolved from the project (see §Design).

## Known Divergences / Open Questions

- **Entire surface is unimplemented at time of writing.** This spec was drafted ahead of any code; every item in §Surface is a target, not current behavior. Tracked by the implementation plan for this spec (path to be added once `docs/plans/` has it — see §References).
- **Open question**: whether `smelt bakeoff`'s cost-measurement machinery should eventually read technique previews from this builder (rather than independently invoking `admitted_family()`/live execution) is undecided; today `bakeoff` measures real cost via live runs and is unaffected by this spec.

## Future Extensions

- An editable SQL buffer in the diagnostics page, with a "preview" action that recomputes properties/plan against the edited (unsaved) text via a new ad hoc-SQL analysis endpoint, and LSP wiring inside the editor itself for live diagnostics as the user types. Explicitly not decided or scoped here — the read-only surface should prove valuable first.

## References

- **Code**: `crates/smelt-runtime/src/diagnostics.rs` (once landed); `crates/smelt-cli/src/explain.rs`; `crates/smelt-ui/src/{build.rs,api.rs,server.rs,types.rs}`; `ui/src/pages/ModelDiagnostics.tsx`; `ui/src/components/SqlViewer.tsx`; `crates/smelt-parser/src/lexer.rs` (comment token kinds); `crates/smelt-logical/src/maintenance/{choice.rs,emit.rs}`
- **Tests**: (once landed) `crates/smelt-runtime` diagnostics builder tests; `crates/smelt-cli` explain snapshot tests; `crates/smelt-parser` `strip_sql_comments` round-trip tests
- **User docs**: (once landed) `docs-site/docs/` page for the model diagnostics UI page
- **Plans (history)**: (to be added once the implementation plan is written)
- **Related specs**: `incremental_models.md` (maintenance plan, cell derivation, the binary execution-time choice this spec's preview set extends for display only); `model_properties.md` (the property catalogue this spec's builder serializes); `architecture.md` (the Run pipeline parity rule this spec's shared-builder pattern follows)

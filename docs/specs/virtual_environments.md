---
feature: virtual_environments
status: experimental
last_reviewed: 2026-06-04
owners: [andrew]
---

# Virtual Environments

> **What this is.** A normative spec for smelt's opt-in **virtual data environments** — cheap, isolated environments (dev branches, CI runs, PR previews) that share physical tables with production whenever a model's output is *provably unchanged*, and rebuild only what provably changed. It defines the `state.mode` posture, environment-suffixed addressing, fingerprint-keyed table reuse, plan categorization (breaking vs non-breaking), the author override hatches, and the promotion ("virtual update") model. Out of scope: the equivalence oracle this is built on (see `output_fingerprint.md`); the persisted state layout (see `run_state.md`); incremental interval execution (see `incremental_models.md`); physical schema migration (see `schema_evolution.md`). This spec is the orchestration layer; `output_fingerprint.md` is the judgement it orchestrates.
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status — most of this layer is unbuilt — lives in §Known Divergences with research links.

## Surface

### `state.mode` — the opt-in posture

A project (in `smelt.yml`) and optionally an individual model (in frontmatter) declare a state posture:

```yaml
# smelt.yml
state:
  mode: stateless        # default — no state store, nothing snapshot-reused
  # mode: intervals      # opt-in: interval ledger for incremental backfill/gap detection
  # mode: environments   # opt-in: full virtual environments (implies intervals)
```

- `stateless` (default) is exactly today's behaviour: no `.smelt/` state store is required, and no model is snapshot-reused. Stateless projects pay nothing for this feature.
- `intervals` enables the persisted interval ledger (see `incremental_models.md`, `run_state.md`).
- `environments` enables environment-suffixed addressing and fingerprint-keyed reuse (this spec). A model may narrow but not widen the project posture.

### Environment-addressed runs

```
smelt plan  --environment <name>     # show the categorized change set vs the environment's current state
smelt apply --environment <name>     # build only what changed; reuse the rest
smelt promote <from> --to <to>       # virtual update: repoint <to> at <from>'s tables, no rebuild
```

Under `state.mode: environments`, physical objects are addressed in an environment-suffixed namespace (e.g. schema `<base>__<environment>`), so two environments never collide on a physical table. An environment that reuses a table points at the *existing* physical object rather than creating its own.

### Author override hatches (frontmatter)

```yaml
# model frontmatter
reuse:
  accept_current: true        # non-deterministic model: reuse the existing materialization across an
                              # output-preserving change rather than re-rolling the dice
  assert_deterministic: true  # assert this model (or a named call) is deterministic-in-practice;
                              # the prover trusts it, the assertion is logged
forward_only: true            # stateful model: apply a breaking change in place, accept no clean revert
```

These are explicit, logged, user-owned overrides. They are the only way to make a non-deterministic or breaking change reuse a table.

## Semantics

### Reuse decision (per model, per environment)

For a model `M` building into environment `E`, smelt may **reuse** an existing physical table `T` (from production or another environment) instead of rebuilding `M` when **all** of:

1. `M` is under `state.mode: environments`. A stateless model is never snapshot-reused.
2. `fingerprint(M_current) == fingerprint(T.source)` — the output fingerprint of `M` equals that of the model version that built `T`, computed fresh on both sides by the current compiler (`output_fingerprint.md`).
3. `M` is **deterministic** (`output_fingerprint.md` §"Determinism signal"), **or** the author set `reuse.accept_current` / `reuse.assert_deterministic`.
4. No physical schema migration is required, or one is applied (see `schema_evolution.md`).

If any condition fails, `M` is rebuilt in `E`'s namespace. Reuse must never produce a table different from what a rebuild would have produced (the soundness invariant of `output_fingerprint.md`, plus the determinism gate of condition 3).

### Plan categorization

`smelt plan --environment E` classifies each model against `E`'s current state as **unchanged** (fingerprint-equal ⇒ reuse), **breaking** (fingerprint differs and the change can alter a column a downstream model consumes ⇒ rebuild it and cascade), or **non-breaking** (the change cannot alter any consumed column ⇒ reuse downstream). The non-breaking class is the "eclipse": a strictly larger provably-safe set than a syntactic edit-script can recognise. Its full form requires cross-model column lineage (see Known Divergences); without lineage, smelt categorizes single-model equivalence only and is conservative (parity) beyond it.

### Promotion (virtual update)

`smelt promote <from> --to <to>` makes `<to>` reference `<from>`'s physical tables without rebuilding — a metadata repoint of the environment→table map. This is sound precisely because the tables were built from fingerprint-identical model versions; promotion never copies or recomputes data.

### Forward-only and unsafe changes

A model with state normally gets safe, revertible, provable-equivalence reuse. `forward_only: true` is an explicit escape hatch on a **stateful** model: apply a genuinely breaking change in place and accept no clean revert — for tables too large to rebuild. (Models with no state are never snapshot-reused, so the clean-revert question does not arise for them.) Both `forward_only` and the `reuse.*` hatches are deliberate, logged overrides of the prover's default verdict.

## Design

**Opt-in layer over an unchanged stateless core.** Everything here sits behind `state.mode`; the default `stateless` path is byte-for-byte today's behaviour. This is the adoption contract: a smelt project pays for environments only when it asks for them, and a half-configured state store can never make a stateless project worse. Rationale: research §6, §0.

**"Stateless" means not snapshot-reused, not "always rebuilt from scratch."** A stateless model is simply outside the snapshot/reuse machinery — it is built the normal way every run. The distinction matters: stateless is not a performance penalty, it is the absence of the reuse *option*. (This corrected an earlier framing; see research and commit history.)

**smelt computes the central judgement better than SQLMesh, and adopts its plumbing.** SQLMesh decides reuse and backfill cascade with a syntactic SQLGlot edit-script that is conservative by necessity (no type system, column lineage on the roadmap). smelt decides the same thing with a *typed, provable equivalence relation* (`output_fingerprint.md`) built on its type system and logical/physical separation. The state store, environment suffixes, and plan/apply *plumbing* are worth adopting wholesale; the *judgement* is where smelt is differentiated. Rationale: research §9.

**Honest staging: worst case parity, typical case eclipse — as coverage is built.** Where smelt's analysis applies (the transparent, deterministic, typed, annotated core), the eclipse is real and provable. Where it goes dark — un-annotated determinism (the planner's `deterministic` defaults to `false`), type-system axes not yet tracked, missing cross-model lineage — smelt degrades to SQLMesh-equivalent conservatism. The day-one win scales with how much of `output_fingerprint.md`'s coverage and the lineage analyser ships, gated at every step by the DuckDB oracle. This spec must not claim eclipse for the un-annotated case. Rationale: research §0, §5.5, §9.

**Determinism overrides mirror the planner's trust model.** `accept_current` and `assert_deterministic` are the same shape as the author-declared `deterministic` function property (`planner_integration.md`): an unproven assertion the prover trusts and logs. They exist because the determinism detector is a conservative floor — it flags inline non-determinism it cannot prove pure — and only the author can supply the knowledge the prover lacks. Rationale: research §5.5.

## Constraints & Invariants

- **Stateless is the default and is untouched.** Enabling this feature must not change behaviour, output, or required on-disk artifacts for a `state.mode: stateless` project.
- **Reuse soundness.** A reused or promoted table is identical to what a rebuild of the current model version would produce — guaranteed by `output_fingerprint.md` soundness plus the determinism precondition. A non-deterministic model is never silently reused; it requires an explicit `reuse.*` override.
- **Overrides are logged.** Every `accept_current`, `assert_deterministic`, and `forward_only` decision is recorded in run state so the trust delegation is auditable.
- **Cross-model categorization is conservative without lineage.** Until the column-lineage analyser exists, the non-breaking (downstream-spared) class is recognised only where single-model equivalence proves it; everything else rebuilds (parity).

## Known Divergences / Open Questions

- **The orchestration layer is unbuilt.** Only the substrate exists today: the equivalence oracle (`output_fingerprint.md`, prototyped in `smelt-fingerprint`, not yet wired into the runtime) and the interval ledger crate (`smelt-state`). The `state.mode` surface, environment-suffixed addressing, `smelt plan/apply --environment`, `smelt promote`, the `reuse.*`/`forward_only` hatches, and fingerprint-keyed snapshots are **specified here but not implemented**. Tracking: `docs/research/20260601-virtual-environments.md` §8.
- **Cross-model column lineage is the gating new work.** The full eclipse (the non-breaking class beyond single-model equivalence) needs a cross-model column-lineage analyser. smelt has the type-inference and per-column-provenance scaffolding to build it, not the finished article. See research §4(b), §5.2.
- **Un-annotated determinism inverts the eclipse.** The planner's `deterministic` property defaults to `false` and is author-declared (`planner_integration.md`); an un-annotated pipeline presents as non-deterministic and rebuilds conservatively. The inline detector (`output_fingerprint.md`) narrows this for structurally-provable cases but cannot derive purity of opaque calls. See research §5.5.
- **User-extensible equivalence rules — open.** Whether a project can register its own "these two forms are output-equivalent" rules (mirroring planner-rule extensibility) is undecided; any such rule must route through the same DuckDB-oracle property gate as the built-in relation, or be confined to *assertions* smelt records-but-trusts. Research §8 Open Question 1.
- **Exact addressing, GC/retention, and typed data-diff surfaces are open.** Environment-suffix scheme, snapshot retention/garbage collection, and `smelt diff` over data (not just schema) are sketched in research §6–§7 but not pinned.

## References

- **Code**: `crates/smelt-fingerprint/` (the equivalence oracle substrate); `crates/smelt-state/` (interval ledger, run manifest, file store) — orchestration layer not yet present
- **Tests**: `crates/smelt-fingerprint/tests/` (oracle and determinism gates)
- **User docs**: none yet
- **Plans (history)**: none yet — predecessor research is `docs/research/20260601-virtual-environments.md`
- **Related specs**: `output_fingerprint.md` (the equivalence oracle), `run_state.md` (persisted `.smelt/` state and snapshots), `incremental_models.md` (interval ledger, the `intervals` posture), `schema_evolution.md` (physical migration when reuse needs a schema change), `architecture.md` (`state.mode` surface, crate responsibilities), `planner_integration.md` (the author-declared `deterministic` property), `cli.md` (`smelt plan`/`apply`/`status`)

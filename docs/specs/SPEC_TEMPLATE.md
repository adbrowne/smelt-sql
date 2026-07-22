---
feature: <feature_slug>
status: experimental
last_reviewed: YYYY-MM-DD
owners: [andrew]
---

<!--
Frontmatter rules:
- `feature`: short slug matching the filename stem (e.g. `incremental_models`).
- `status`: one of `experimental`, `stable`, `deprecated`.
  - `experimental` (default for new specs): the surface may break; pre-1.0.
  - `stable`: the surface is committed; breaking changes require a deprecation cycle.
    Today only `architecture.md` is `stable` — it pins load-bearing invariants the
    rest of the spec set depends on.
  - `deprecated`: the spec describes a feature being phased out. Cross-references
    should mark the replacement.
- `last_reviewed`: YYYY-MM-DD of the last substantive review (drift audit, sweep,
  or rewrite). Bump on any non-trivial edit.
- `owners`: GitHub handles or names of people responsible for keeping the spec
  current.

References blocks (the bottom-of-spec section) use **flat bullets** under the
`- **Code**:` / `- **Tests**: ` / `- **User docs**:` / `- **Plans (history)**:` /
`- **Related specs**:` headings — not nested sub-headings — to keep cross-spec
parsing scripts simple.
-->

# <Feature Title>

> **What this is.** *(Required scope callout — keep this blockquote between the H1 and the first `##` section heading on every spec.)* A one-paragraph statement of what this spec covers and what it does not — naming the adjacent specs that own neighbouring concerns. Readers skim this first; if they're in the wrong file, the callout sends them to the right one. Example shape: "A normative spec for `<feature>`: <one-sentence summary of in-scope surface>. Out of scope: <bullet of adjacent concern> (see `<other_spec>.md`); <another adjacent concern> (see `<another_spec>.md`)."
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. No plan-phase headings (`### Phase A — …`), no inline phase labels (`Meta list (Phase A)`), no plan-vocabulary status callouts (`[deferred to Phase E1]`) in §Surface, §Semantics, §Design, or §Constraints. Implementation status that needs naming goes in §Known Divergences (describe behaviour, link the plan; phase numbers tolerated only when paired with a plan link) or §References → Plans (history) (link plan files; do not describe their phase structure). See the Timeless-oracle rule in `CLAUDE.md` for the full rule and good/bad examples.

## Overview

*(Optional — recommended for large specs.)* A **non-normative** mental-model primer, placed
before `## Surface`. Its job is concept ordering: state the feature's central guarantee first,
then the declared surface in outline, then how the machinery relates — introducing every term
the spec depends on *before* the normative sections use it. Worked examples and a short reading
guide ("which section answers which kind of question") belong here. Nothing in this section may
be the sole home of a rule: every normative statement it previews must also appear, in full, in
§Surface/§Semantics/§Constraints — on conflict, the normative sections win.

## Surface

The contract this spec defines — what callers can observe, depend on, and be told by error messages. For feature specs, "callers" are users; for system specs (`architecture.md`), "callers" can include other crates / components, and the surface includes contracts between them.

If anything in this section is user-visible, you must update the corresponding `docs-site/` page in the same PR.

- Syntax additions or changes
- YAML frontmatter fields, their types and defaults
- CLI flags / subcommands
- Public Rust API (user-facing or cross-crate)
- Error messages and diagnostic codes the user will see
- Wire-format / on-disk artifacts users or downstream tools interact with

## Semantics

Formal behavior. What the system does given the surface. Use precise language ("must", "must not", "if X then Y"). Reference test files for executable specifications where they exist.

- Evaluation rules / type rules / rewrite rules
- Edge cases and how they're handled
- Failure modes (what error fires when, and why)
- Interactions with adjacent features (link to their specs, don't duplicate)

## Design

The rationale — *why* the spec is shaped this way. Captures the load-bearing decisions and the alternatives rejected, so future contributors can tell when a constraint is structural versus when it's open to revisit.

- Why each surface choice (annotation form, default, error code) is what it is
- Alternatives considered and why rejected (briefly — the spec is not a research doc; link to `docs/research/` if there is more)
- Cross-feature interactions that drove the shape (link to other specs)
- Architectural decisions the implementation must respect (these often graduate into Constraints & Invariants below)

Keep this section dense — one paragraph per decision, not an essay. If a decision needs deep justification, write it in `docs/research/` and link.

## Constraints & Invariants

What must always hold. What is explicitly not supported and why. These are the things `smelt:validate` will check against.

- Invariants the implementation must preserve (e.g., "`type_inference.rs` is pure")
- Properties that must hold across all inputs (proptest-style)
- Things explicitly out of scope (and the reason — keeps future plans honest)

## Known Divergences / Open Questions

Where current implementation differs from intent, or where intent itself is undecided. Update as part of any plan that touches this feature.

- Implementation gaps known at `last_reviewed`
- Open design questions with current best-known answer
- Tensions with other specs (link to them)

## Future Extensions

*(Optional — omit if there's nothing speculative to record.)* Ideas for widening the feature's
surface or admission space that are **not decided**, unlike `§Known Divergences` (which tracks a
gap between decided intent and current implementation). Nothing in this section is surface, and
none of it may be relied on or implemented against until it graduates into `§Surface`/`§Semantics`
via its own spec diff and plan.

- A candidate future capability, the concrete case that motivates it, and what's still open
  (what new trigger/diagnostic/composition question it would raise)
- Explicitly out of scope for now, and why now isn't the time

## References

Concrete pointers — kept current, not historical.

- **Code**: primary implementation paths (`crates/...`, `src/...`)
- **Tests**: tests that exercise the spec (especially the spec-invariant tests)
- **User docs**: `docs-site/docs/...` pages that document this feature
- **Plans (history)**: links to past `docs/plans/*` that landed work in this area, ordered oldest → newest
- **Related specs**: other `docs/specs/*.md` files this one depends on or interacts with

---
feature: <feature_slug>
status: experimental
last_reviewed: YYYY-MM-DD
owners: [andrew]
---

# <Feature Title>

> **What this is.** A normative spec for `<feature>`. This is the canonical answer to "how does `<feature>` work?". Behavior changes start here, then propagate to plans, code, and user docs.
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.

## Surface

The user-visible API. Anything a user can observe, type, or be told by an error message. If you change anything in this section, you must update the corresponding `docs-site/` page in the same PR.

- Syntax additions or changes
- YAML frontmatter fields, their types and defaults
- CLI flags / subcommands
- Public Rust API (only if user-facing — internal APIs go in code, not here)
- Error messages and diagnostic codes the user will see
- Wire-format / on-disk artifacts the user can interact with

## Semantics

Formal behavior. What the system does given the surface. Use precise language ("must", "must not", "if X then Y"). Reference test files for executable specifications where they exist.

- Evaluation rules / type rules / rewrite rules
- Edge cases and how they're handled
- Failure modes (what error fires when, and why)
- Interactions with adjacent features (link to their specs, don't duplicate)

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

## References

Concrete pointers — kept current, not historical.

- **Code**: primary implementation paths (`crates/...`, `src/...`)
- **Tests**: tests that exercise the spec (especially the spec-invariant tests)
- **User docs**: `docs-site/docs/...` pages that document this feature
- **Plans (history)**: links to past `docs/plans/*` that landed work in this area, ordered oldest → newest
- **Related specs**: other `docs/specs/*.md` files this one depends on or interacts with

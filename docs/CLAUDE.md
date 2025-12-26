# CLAUDE.md - Documentation Directory

This file provides guidance to Claude Code (claude.ai/code) when working in the `docs/` directory.

## Purpose of This Directory

This directory is dedicated to **planning, design documentation, and architecture decisions** for the smelt project. When working in this directory, Claude should focus exclusively on documentation and planning tasks, not code implementation.

## Important Constraints

**DO NOT access code outside this directory** - The only exception is:
- `/Users/andrewbrowne/code/smelt/README.md` - The main project specification

**DO NOT make code changes** - This session is for documentation only:
- NO reading parser, CLI, or LSP source code
- NO editing implementation files
- NO running code examples or tests

**DO reference existing documentation** freely:
- All files in this `docs/` directory
- `README.md` from the project root (for spec reference)
- `ROADMAP.md` (also in this directory)

## Documentation Structure

### Specification
- **../README.md** - Complete language specification and design decisions
  - Language syntax and semantics
  - Extension design (`smelt.ref()`, `smelt.metric()`)
  - Type system and computation requirements
  - Backend capabilities and optimization philosophy

### Implementation Planning
- **ROADMAP.md** - Implementation status, completed phases, and next steps
  - Tracks what's been implemented (with completion dates)
  - Deferred work with rationale
  - Proposed next-step options
  - Future work backlog

### Architecture Documentation
- **architecture_overview.md** - System design and component interactions
- **lsp_architecture.md** - LSP implementation details and design decisions
- **lsp_quickstart.md** - Getting started with the LSP
- **optimization_rule_api_design.md** - Future optimizer API design

### Analysis and Insights
- **example1_insights.md** - Analysis of common intermediate aggregation optimization
- **example2_insights.md** - Analysis of split large GROUP BY optimization

## Typical Tasks in This Directory

When the user is working in `docs/`, they're typically:

1. **Planning new features**
   - Designing APIs before implementation
   - Thinking through optimization patterns
   - Documenting architectural decisions

2. **Documenting completed work**
   - Updating ROADMAP.md after completing phases
   - Writing architecture docs for new components
   - Capturing insights from examples

3. **Strategic planning**
   - Deciding between implementation options
   - Prioritizing next steps
   - Evaluating trade-offs

4. **Design discussions**
   - Exploring syntax alternatives
   - Designing optimizer APIs
   - Planning LSP features

## Workflow for Documentation Tasks

### When Updating ROADMAP.md

1. Mark completed phases with ✅ and completion date
2. Explain what was implemented and why
3. Document deferred work with clear rationale
4. Propose concrete next-step options
5. Use dates (e.g., "December 26, 2025") instead of commit hashes

### When Creating Architecture Docs

1. Start with "Why" - motivation and goals
2. Explain "What" - the design at a high level
3. Detail "How" - specific implementation patterns
4. Document trade-offs and alternatives considered
5. Include examples to illustrate concepts

### When Analyzing Optimization Patterns

1. Show concrete before/after examples
2. Explain the optimization pattern being demonstrated
3. Identify what makes the optimization safe or unsafe
4. Extract API requirements from the pattern
5. Consider how to generalize the pattern

## Communication Style

When working in `docs/`:
- Be thorough and detailed - documentation should be comprehensive
- Explain rationale and trade-offs, not just decisions
- Use concrete examples to illustrate abstract concepts
- Reference the README.md spec to stay aligned with project vision
- Think strategically about the project direction

## What NOT to Do

- ❌ Read or reference implementation code (parser, CLI, LSP, etc.)
- ❌ Make code changes or propose specific code edits
- ❌ Run commands, tests, or examples
- ❌ Look at test workspaces or example code
- ❌ Access files outside this directory (except README.md)

## Project Context (High-Level Only)

**smelt** is a modern data transformation framework that:
- Separates logical specification from physical execution
- Enables automatic optimization across models
- Supports multi-backend execution
- Uses proper language instead of Jinja templates
- Provides LSP support for great developer experience

For full details, see `../README.md`.

Current project status is tracked in `ROADMAP.md`.

## License

MIT License - Copyright (c) 2025 Andrew Browne

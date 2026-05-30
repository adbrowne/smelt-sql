# crates/smelt-parser/CLAUDE.md

Standalone Rowan-based parser — text → lossless CST with error recovery, typed AST wrappers, and the `SyntaxKind` enum. No Salsa dependency; usable in any context.

## How to test

```bash
cargo test -p smelt-parser
```

Parser tests live inline in `src/parser/tests.rs`.

## Gotchas

- **No position conversion happens here.** `smelt-parser` re-exports `rowan::TextRange` but does not provide `offset_to_position` or `text_range_to_range` helpers. Those conversions belong at the LSP and CLI boundaries. See root `CLAUDE.md` §Architectural invariants — **Diagnostic Range Encoding** is the rule.
- **Adding a new token type:** add a variant to `SyntaxKind` in `src/syntax_kind.rs`, add a lexer case in `src/lexer.rs`, and (if it surfaces in the AST) add a typed wrapper in `src/ast.rs`. `SyntaxKind` is `#[repr(u16)]` — order matters for Rowan's internal representation; append new variants rather than inserting.
- **`src/ast.rs` is ~4000 lines.** It's typed wrappers over the Rowan CST — one struct per node kind. Use `rg 'pub struct <NodeName>'` to find the relevant type rather than reading the file top-to-bottom.
- **Error recovery is intentional.** The parser is designed to produce a usable CST even for partial or invalid SQL. If a new construct fails to parse, check whether the sync point set (`parser/mod.rs`) needs updating before assuming the grammar is wrong.
- **`src/parser/smelt_ext.rs`** handles smelt-specific extensions (`smelt.ref()`, `smelt.fn.*`, `=>` named-parameter syntax). SQL-only grammar changes go in `parser/select.rs` or `parser/expr.rs`.

## Where things live

- `src/syntax_kind.rs` — `SyntaxKind` enum (all token/node variants)
- `src/lexer.rs` — tokenizer; maps source text to `SyntaxKind` tokens
- `src/parser/` — recursive descent parser; `mod.rs` (entry + sync), `select.rs`, `expr.rs`, `smelt_ext.rs`, `meta.rs`
- `src/ast.rs` — typed AST wrappers over Rowan CST (~4000 lines)
- `src/lib.rs` — `find_frontmatter_blocks` and re-exports

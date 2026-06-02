# smelt-fingerprint

**Stage 0 of the virtual-environments work** (`docs/research/20260601-virtual-environments.md`, §8).

A *semantic-equivalence oracle*: given two versions of a model's (function-expanded)
`SELECT`, compute an `output_fingerprint` over a **canonical normal form** of the typed CST.
Two versions with the same fingerprint are proven to compute the same relation — same
multiset of rows, columns matched by name — for the same inputs. That is the table-reuse
key a downstream environment would use to point at an existing physical table instead of
rebuilding.

This crate is single-model only: no state store, no environments, no cross-model lineage.

## The one invariant: soundness

A fingerprint match must **never** be a false positive (that would silently corrupt data).
The gate is `tests/soundness_prop.rs`: a property test asserting, over generated query
pairs, that

```
fingerprint(A) == fingerprint(B)  ⇒  DuckDB confirms A and B are the same relation
```

Only this direction is asserted. *Incompleteness* — failing to recognise a genuine
equivalence — is allowed; the canonicaliser falls back to a verbatim hash whenever it
cannot prove a rewrite output-preserving. Completeness grows rule by rule, each gated by
this oracle, exactly as `smelt-db`'s `type_property_tests.rs` grows type coverage against
DuckDB.

## Equivalences recognised (the eclipse over SQLMesh)

SQLMesh's syntactic edit-script rebuilds on any change other than adding a projection.
smelt recognises these refactors as equivalent and would reuse the table:

| Refactor | Test |
|---|---|
| Whitespace / formatting | `whitespace_reformatting_is_equivalent` |
| Line / block comments | `*_comment_is_ignored` |
| Keyword case | `keyword_case_is_equivalent` |
| Projection **reordering** | `projection_reorder_*` |
| Internal CTE / alias **renaming** | `cte_name_rename_*`, `derived_table_alias_rename_*` |
| Single-use **CTE ≡ derived table** | `single_use_cte_equals_derived_table`, `cte_inline_*` |
| Refactor **inside** an inlined CTE body | `cte_inline_with_inner_refactor` |

The negative corpus (`tests/corpus_negative.rs`) pins the other side: real changes
(filter, expression, column add/remove/rename, `DISTINCT`, `GROUP BY`, `UNION`-branch
reorder) all move the fingerprint.

## Known completeness gaps (sound, deferred)

These are recognised as *not equivalent* today (conservative, never unsound), left for later
stages:

- **Deep subquery flattening** — collapsing a derived table's projection into its parent
  (the monolithic-vs-deeply-nested split in the research §5.3 #1 worked example). Only the
  single-subquery FROM is canonicalised by content.
- **Joins / multi-table FROM, multi-CTE queries** — kept in flat form.
- **Cross-model / column lineage** — dead-column removal and downstream-spared changes
  (§5.3 #2/#3/#5) need the cross-model lineage analyser the research gates to Stage 4.
- **Type-system axes** — decimal precision, collation, nullability are not yet folded
  (§5.5); type-only changes invisible to the current type system stay conservatively
  distinct.
- **`smelt.<path>` resolution / function expansion** — callers pass an already-expanded
  `SELECT`; this crate does not resolve refs itself.

Fallbacks are recorded as `MissedReuse` on the result so a later stage can quantify the gap.

## Running

```bash
# unit (no DuckDB)
cargo test -p smelt-fingerprint --lib
# corpus + oracle + soundness (DuckDB-backed; needs DUCKDB_LIB_DIR + LD_LIBRARY_PATH)
cargo test -p smelt-fingerprint
# deeper soundness coverage
PROPTEST_CASES=1000 cargo test -p smelt-fingerprint --test soundness_prop
```

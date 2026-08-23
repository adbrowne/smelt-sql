<!-- GENERATED FILE — do not edit by hand.
Regenerate with:
     SMELT_REGEN_DOCS=1 cargo test -p smelt-db --test dialect_audit \
       the_coverage_table_matches_the_registry
-->

# Dialect emission coverage

How every built-in smelt recognises is spelled on each backend. Each cell is the
`Emission` verdict the registry carries for that `(entry, dialect)` pair
(`crates/smelt-types/src/signatures.rs`), which is the single place the printer
reads — there is no name-matched dialect arm in `printer.rs`.

Cell vocabulary:

- `native` — same spelling, same semantics; smelt emits the name unchanged.
- `rename:X` — same call shape, emitted as `X`.
- `rewrite:Id` — structurally rewritten by the printer's `RewriteId::Id` arm.
- `unsupported` — the compiler refuses the model (`UnsupportedOnBackend`) rather
  than emitting SQL the engine would reject or misread.
- `(gap #N)` — a live sweep found this pair does not work as claimed, tracked by
  issue #N. The count ratchets down only
  (`.claude/dialect-gaps-baseline.txt`).
- `(gap divergent)` — an accepted, permanent semantic difference no rename or
  rewrite can close.

| Entry | Form | DuckDB | Spark SQL | PostgreSQL | BigQuery |
|---|---|---|---|---|---|
| `%` | infix | native | native | native | rewrite:ModuloCall |
| `**` | infix | native | rewrite:PowerCall | native | rewrite:PowerCall |
| `//` | infix | native | unsupported | unsupported | unsupported |
| `ABS` | call | native | native | native | native |
| `ACOS` | call | native | native | native | native |
| `AGE` | call | native | native (gap #171) | native | native |
| `ANY_VALUE` | call | native | native | native | native |
| `APPROX_COUNT_DISTINCT` | call | native | native | native | native |
| `ARG_MAX` | call | native | native (gap #171) | native | native |
| `ARG_MIN` | call | native | native (gap #171) | native | native |
| `ARRAY_AGG` | call | native | native (gap divergent) | native | native |
| `ASIN` | call | native | native | native | native |
| `ATAN` | call | native | native | native | native |
| `ATAN2` | call | native | native | native | native |
| `AVG` | call | native | native | native | native |
| `BETWEEN` | special | native | native | native | native |
| `BIT_AND` | call | native | native | native | native |
| `BIT_OR` | call | native | native | native | native |
| `BIT_XOR` | call | native | native | native | native |
| `BOOL_AND` | call | native | rename:EVERY | native | rename:LOGICAL_AND |
| `BOOL_OR` | call | native | rename:SOME | native | rename:LOGICAL_OR |
| `CAST` | special | native | native | native | native |
| `CEIL` | call | native | native | native | native |
| `CEILING` | call | native | native | native | native |
| `CHARACTER_LENGTH` | call | native | native | native | native |
| `CHAR_LENGTH` | call | native | native | native | native |
| `COALESCE` | call | native | native | native | native |
| `CONCAT` | call | native | native (gap divergent) | native | native |
| `CORR` | call | native | native (gap divergent) | native | native |
| `COS` | call | native | native | native | native |
| `COSH` | call | native | native | native | native |
| `COUNT` | call | native | native | native | native |
| `COVAR_POP` | call | native | native | native | native |
| `COVAR_SAMP` | call | native | native | native | native |
| `CUME_DIST` | call | native | native | native | native |
| `CURRENT_DATE` | call | native | native | native | native |
| `CURRENT_TIMESTAMP` | call | native | native | native | native |
| `DATE` | call | native | native | native | native |
| `DATE_ADD` | special | native | native (gap #171) | native | native |
| `DATE_PART` | call | native | native | native | native |
| `DATE_SUB` | special | native (gap #171) | native (gap #171) | native | native |
| `DATE_TRUNC` | call | native | native | native | native |
| `DAY` | call | native | native | native | native |
| `DAYOFWEEK` | call | native | native (gap #171) | native | native |
| `DENSE_RANK` | call | native | native | native | native |
| `EVERY` | call | rename:BOOL_AND | native | native | rename:LOGICAL_AND |
| `EXISTS` | special | native | native | native | native |
| `EXP` | call | native | native | native | native |
| `EXPLODE` | table-fn | rename:UNNEST | native | rename:UNNEST | rename:UNNEST |
| `EXTRACT` | call | native | native | native | native |
| `FIRST` | call | native | native | native | native |
| `FIRST_VALUE` | call | native | native | native | native |
| `FLOOR` | call | native | native | native | native |
| `GLOB` | infix | native | native (gap #171) | native | native |
| `GREATEST` | call | native | native | native | native |
| `GROUP_CONCAT` | call | native | native (gap #171) | native | native |
| `IFNULL` | call | native | native | native | native |
| `ILIKE` | infix | native | native | native | native |
| `IN` | special | native | native | native | native |
| `INITCAP` | call | native (gap #171) | native | native | native |
| `IS_NOT_NULL` | postfix | native | native | native | native |
| `IS_NULL` | postfix | native | native | native | native |
| `JSON_ARRAY` | call | native | native (gap #171) | native | native |
| `JSON_ARRAY_LENGTH` | call | native | native (gap #171) | native | native |
| `JSON_CONTAINS` | call | native | native (gap #171) | native | native |
| `JSON_EXTRACT` | call | native | native (gap #171) | native | native |
| `JSON_EXTRACT_TEXT` | call | native (gap #171) | native (gap #171) | native | native |
| `JSON_OBJECT` | call | native | native (gap #171) | native | native |
| `JSON_OBJECT_KEYS` | call | native (gap #171) | native (gap #171) | native | native |
| `LAG` | call | native | native | native | native |
| `LAST` | call | native | native | native | native |
| `LAST_VALUE` | call | native | native | native | native |
| `LEAD` | call | native | native | native | native |
| `LEAST` | call | native | native | native | native |
| `LEFT` | call | native | native | native | native |
| `LENGTH` | call | native | native | native | native |
| `LIKE` | infix | native | native | native | native |
| `LISTAGG` | call | native | native | native | native |
| `LN` | call | native | native | native | native |
| `LOG` | call | native | native (gap #171) | native | native |
| `LOG10` | call | native | native | native | native |
| `LOG2` | call | native | native | native | native |
| `LOWER` | call | native | native | native | native |
| `LPAD` | call | native | native | native | native |
| `LTRIM` | call | native | native | native | native |
| `MAKE_DATE` | call | native | native | native | native |
| `MAKE_TIME` | call | native | native (gap #171) | native | native |
| `MAKE_TIMESTAMP` | call | native | native | native | native |
| `MAKE_TIMESTAMPTZ` | call | native | native (gap #171) | native | native |
| `MAX` | call | native | native | native | native |
| `MD5` | call | native | native | native | native |
| `MEDIAN` | call | native | native (gap #171) | native | rewrite:BigQueryMedian |
| `MIN` | call | native | native | native | native |
| `MOD` | call | native | native | native | native |
| `MODE` | call | native | native | native | native |
| `MONTH` | call | native | native | native | native |
| `NOW` | call | native | native | native | native |
| `NTH_VALUE` | call | native | native | native | native |
| `NTILE` | call | native | native | native | native |
| `NULLIF` | call | native | native | native | native |
| `PERCENTILE_CONT` | call | native (gap #171) | native (gap #171) | native | native |
| `PERCENTILE_DISC` | call | native (gap #171) | native (gap #171) | native | native |
| `PERCENT_RANK` | call | native | native | native | native |
| `PI` | call | native | native | native | native |
| `POSITION` | call | native | native | native | native |
| `POW` | call | native | native | native | native |
| `POWER` | call | native | native | native | native |
| `QUARTER` | call | native | native | native | native |
| `QUOTE_IDENT` | call | native (gap #171) | native (gap #171) | native | native |
| `QUOTE_LITERAL` | call | native (gap #171) | native (gap #171) | native | native |
| `RANDOM` | call | native | native | native | native |
| `RANK` | call | native | native | native | native |
| `REGR_SLOPE` | call | native | native (gap divergent) | native | native |
| `REPEAT` | call | native | native | native | native |
| `REPLACE` | call | native | native | native | native |
| `REVERSE` | call | native | native | native | native |
| `RIGHT` | call | native | native | native | native |
| `ROUND` | call | native | native | native | native |
| `ROW_NUMBER` | call | native | native | native | native |
| `RPAD` | call | native | native | native | native |
| `RTRIM` | call | native | native | native | native |
| `SIGN` | call | native | native | native | native |
| `SIN` | call | native | native | native | native |
| `SINH` | call | native | native | native | native |
| `SPLIT_PART` | call | native | native | native | native |
| `SQRT` | call | native | native | native | native |
| `STDDEV` | call | native | native | native | native |
| `STDDEV_POP` | call | native | native | native | native |
| `STDDEV_SAMP` | call | native | native | native | native |
| `STRING_AGG` | call | native | native | native | native |
| `STRPOS` | call | native | native (gap #171) | native | native |
| `SUBSTR` | call | native | native | native | native |
| `SUBSTRING` | call | native | native | native | native |
| `SUM` | call | native | native | native | native |
| `TAN` | call | native | native | native | native |
| `TANH` | call | native | native | native | native |
| `TO_CHAR` | call | native (gap #171) | native | native | native |
| `TO_JSON` | call | native | native (gap #171) | native | native |
| `TO_SECONDS` | call | native | native (gap #171) | native | native |
| `TRANSLATE` | call | native | native | native | native |
| `TRIM` | call | native | native | native | native |
| `TRUNC` | call | native | native (gap #171) | native | native |
| `TRUNCATE` | call | native (gap #171) | native (gap #171) | native | native |
| `UNNEST` | table-fn | native | rename:EXPLODE | native | native |
| `UPPER` | call | native | native | native | native |
| `VARIANCE` | call | native | native | native | native |
| `VAR_POP` | call | native | native | native | native |
| `VAR_SAMP` | call | native | native | native | native |
| `YEAR` | call | native | native | native | native |
| `^` | infix | native | rewrite:PowerCall | native | rewrite:PowerCall |
| `||` | infix | native | native | native | native |

## Schema-only entries

These are probed for acceptance but never value-compared: they return a different
answer on every run, or on every engine, for reasons that say nothing about
emission.

- `ANY_VALUE` — returns an unspecified row's value; engines may pick different rows
- `CURRENT_DATE` — engines execute at different instants
- `CURRENT_TIMESTAMP` — engines execute at different instants
- `MODE` — ties are broken arbitrarily, and the fixture's values are all distinct within a group
- `NOW` — engines execute at different instants
- `RANDOM` — no stable value: a different draw per engine and per call

## Verification tiers

A verdict in the table is what smelt *claims*. What a live engine has actually
confirmed differs per dialect:

| Dialect | Live leg | Tier |
|---|---|---|
| DuckDB | schema + value | every PR (in-process, no warehouse) |
| Spark SQL | schema + value | nightly, or a PR labelled `run-docker-tests` |
| BigQuery | schema + value | manual sweep only — `scripts/bigquery-dialect-audit.sh`; the value leg executes rather than dry-runs, so it bills |
| PostgreSQL | none | **unverified** — a `SqlDialect` variant with no backend crate and no oracle, so nothing exercises its verdicts |

An untested `native` is reported as *unverified*, never as *passing*: the value leg
exists to test the claim, and a default-passing assumption would recreate exactly the
silent hole this audit was built to close.

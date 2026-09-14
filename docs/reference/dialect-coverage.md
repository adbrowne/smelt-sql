<!-- GENERATED FILE — do not edit by hand.
Regenerate with:
     SMELT_REGEN_DOCS=1 cargo test -p smelt-db --test dialect_audit \
       the_coverage_table_matches_the_registry
-->

# Dialect emission coverage

How every built-in smelt recognises is spelled on each backend. Each cell is the
`Emission` verdict the registry carries for that `(entry, dialect)` pair
(`crates/smelt-types/src/signatures/`), which is the single place the printer
reads — there is no name-matched dialect arm in the printer.

Cell vocabulary:

- `native` — same spelling, same semantics; smelt emits the name unchanged.
- `rename:X` — same call shape, emitted as `X`.
- `template:X` — the target spells this built-in as a fixed shape `X` over the
  call's own positional arguments (`{n}` placeholders), interpreted by one generic
  printer routine that holds no function names; a call carrying a modifier a
  placeholder cannot express (`DISTINCT`, `FILTER`, `WITHIN GROUP`, an argument-list
  `ORDER BY`, `IGNORE`/`RESPECT NULLS`, a named argument, or `*`) is refused at
  compile time rather than silently dropping it.
- `rewrite:Id` — structurally rewritten by the printer's `RewriteId::Id` arm.
- `restructure:Id` — the enclosing query block is restructured around a
  synthesised CTE by the planner's `RestructureId::Id` shape, because the
  backend offers this built-in only in the opposite position from the one the
  author wrote.
- `unsupported` — the compiler refuses the model (`UnsupportedOnBackend`) rather
  than emitting SQL the engine would reject or misread.
- `conditional(guard→verdict | ... | otherwise→verdict)` — the verdict depends on
  the call's own arity and/or operand types; the first arm whose guard the call
  satisfies wins, and `otherwise` always matches last. Settled once per call on
  the compile path by `Signature::settle_at`; the printer only ever sees the
  settled verdict. Every arm is probed by the audit — never claimed from
  documentation.
- `(gap #N)` — a live sweep found this pair does not work as claimed, tracked by
  issue #N. The count ratchets down only
  (`.claude/dialect-gaps-baseline.txt`).
- `(gap divergent)` — an accepted, permanent semantic difference no rename or
  rewrite can close.

A cell holds one verdict when every position the entry can occupy agrees. When they
differ, the cell renders the set instead, one `position:verdict` term per position,
separated by `; ` — `agg` (aggregate, no `OVER`), `win` (an `OVER` clause covering the
whole partition), `run` (a narrower, running `OVER` clause), `scalar` (a row-wise
call). Collapsing to a single verdict would hide exactly the position-dependent
asymmetry the position axis exists to record — GoogleSQL's `PERCENTILE_CONT` is
refused as an aggregate but accepted with a whole-partition `OVER`, while `MAX_BY` is
the exact reverse.

| Entry | Form | DuckDB | Spark SQL | BigQuery | Trino |
|---|---|---|---|---|---|
| `%` | infix | native | native | template:MOD({0}, {1}) (gap #173) | native |
| `**` | infix | native | template:POWER({0}, {1}) | template:POWER({0}, {1}) (gap divergent) | template:POWER({0}, {1}) |
| `//` | infix | native | conditional(a0:integral,a1:integral→template:{0} DIV {1} | a0:floating,a1:floating→template:{0} / {1} | a0:decimal,a1:decimal→template:{0} / {1} | otherwise→unsupported) | unsupported | template:{0} / {1} (gap #209) |
| `ABS` | call | native | native | native | native |
| `ACOS` | call | native | native | native | native |
| `AGE` | call | native | template:{0} - {1} (gap divergent) | native (gap #179) | native (gap #209) |
| `ANY_VALUE` | call | native | native | native | native |
| `APPROX_COUNT_DISTINCT` | call | native | native | agg:native; win:restructure:WindowToCte; run:unsupported (gap #179) | native (gap #209) |
| `ARG_MAX` | call | native | rename:MAX_BY | agg:rename:MAX_BY; win:restructure:WindowToCte; run:unsupported (gap #179) | native (gap #209) |
| `ARG_MIN` | call | native | rename:MIN_BY | agg:rename:MIN_BY; win:restructure:WindowToCte; run:unsupported (gap #179) | native (gap #209) |
| `ARRAY_AGG` | call | native | native (gap divergent) | native (gap divergent) | native (gap #209) |
| `ASIN` | call | native | native | native | native |
| `ATAN` | call | native | native | native | native |
| `ATAN2` | call | native | native | native | native |
| `AVG` | call | native | native | native | native |
| `BETWEEN` | special | native | native | native | native |
| `BIT_AND` | call | native | native | native | native (gap #209) |
| `BIT_OR` | call | native | native | native | native (gap #209) |
| `BIT_XOR` | call | native | native | native | native (gap #209) |
| `BOOL_AND` | call | native | rename:EVERY | rename:LOGICAL_AND | native |
| `BOOL_OR` | call | native | rename:SOME | rename:LOGICAL_OR | native |
| `CAST` | special | native | native | native | native |
| `CEIL` | call | native | native | native | native |
| `CEILING` | call | native | native | native | native (gap #209) |
| `CHARACTER_LENGTH` | call | native | native | native | native (gap #209) |
| `CHAR_LENGTH` | call | native | native | native | native (gap #209) |
| `COALESCE` | call | native | native | native | native (gap #209) |
| `CONCAT` | call | native | native (gap divergent) | native (gap divergent) | native (gap #209) |
| `CORR` | call | native | native (gap divergent) | native (gap divergent) | native |
| `COS` | call | native | native | native | native |
| `COSH` | call | native | native | native | native |
| `COUNT` | call | native | native | native | native |
| `COVAR_POP` | call | native | native | native | native |
| `COVAR_SAMP` | call | native | native | native | native |
| `CUME_DIST` | call | native | native | native | native |
| `CURRENT_DATE` | call | native | native | native | native |
| `CURRENT_TIMESTAMP` | call | native | native | native | native (gap #209) |
| `DATE` | call | native | native | native | native |
| `DATE_ADD` | call | native | template:CAST({0} + {1} AS TIMESTAMP) | native (gap #176, divergent) | native (gap #209) |
| `DATE_PART` | call | native | native | native (gap #179) | native (gap #209) |
| `DATE_SUB` | call | template:{0} - {1} | template:CAST({0} - {1} AS TIMESTAMP) | native (gap #176) | native (gap #209) |
| `DATE_TRUNC` | call | native | native | native (gap #179) | native |
| `DAY` | call | native | native | native (gap #179) | native |
| `DAYOFWEEK` | call | native | template:DAYOFWEEK({0}) - 1 | native (gap #179) | native (gap #209) |
| `DENSE_RANK` | call | native | native | native | native |
| `EPOCH_US` | call | native | template:unix_micros({0}) | template:UNIX_MICROS({0}) | native (gap #209) |
| `EVERY` | call | rename:BOOL_AND | native | rename:LOGICAL_AND | native |
| `EXISTS` | special | native | native | native | native |
| `EXP` | call | native | native | native | native |
| `EXPLODE` | table-fn | rename:UNNEST (gap #176) | native (gap #176) | rename:UNNEST (gap #179) | native (gap #209) |
| `EXTRACT` | call | native | native | native | native |
| `FIRST` | call | native (gap #175) | native (gap #175) | native (gap #179) | native (gap #209) |
| `FIRST_VALUE` | call | native | native | native | native |
| `FLOOR` | call | native | native | native | native |
| `GLOB` | infix | native | unsupported | native (gap #179) | native (gap #209) |
| `GREATEST` | call | native | native | native (gap divergent) | native |
| `GROUP_CONCAT` | call | native | unsupported | rename:STRING_AGG | native (gap #209) |
| `IFNULL` | call | native | native | native | native (gap #209) |
| `ILIKE` | infix | native | native | native (gap #179) | native (gap #209) |
| `IN` | special | native | native | native | native |
| `INITCAP` | call | unsupported | native | native | native (gap #209) |
| `IS_NOT_NULL` | postfix | native | native | native | native |
| `IS_NULL` | postfix | native | native | native | native |
| `JSON_ARRAY` | call | native | unsupported | native | native |
| `JSON_ARRAY_LENGTH` | call | native | native (gap divergent) | native (gap #179) | native |
| `JSON_CONTAINS` | call | native | unsupported | native (gap #179) | native (gap #209) |
| `JSON_EXTRACT` | call | native | rename:GET_JSON_OBJECT | native | native (gap #209) |
| `JSON_EXTRACT_TEXT` | call | rename:JSON_EXTRACT_STRING | rename:GET_JSON_OBJECT | rename:JSON_VALUE | native (gap #209) |
| `JSON_OBJECT` | call | native | unsupported | native | native (gap #209) |
| `JSON_OBJECT_KEYS` | call | rename:JSON_KEYS | native (gap divergent) | native (gap #179) | native (gap #209) |
| `LAG` | call | native | rewrite:ElideWindowFrame | native | native |
| `LAST` | call | native (gap #175) | native (gap #175) | native (gap #179) | native (gap #209) |
| `LAST_VALUE` | call | native | native | native | native |
| `LEAD` | call | native | rewrite:ElideWindowFrame | native | native |
| `LEAST` | call | native | native | native (gap divergent) | native |
| `LEFT` | call | native | native | native | native (gap #209) |
| `LENGTH` | call | native | native | native | native |
| `LIKE` | infix | native | native | native | native |
| `LISTAGG` | call | native | native | rename:STRING_AGG | native (gap #209) |
| `LN` | call | native | native | native | native |
| `LOG` | call | native | conditional(arity=1→rename:LOG10 | otherwise→native) | native (gap #174) | conditional(arity=1→unsupported | otherwise→native) |
| `LOG10` | call | native | native | native | native |
| `LOG2` | call | native | native | native (gap #179) | native |
| `LOWER` | call | native | native | native | native |
| `LPAD` | call | native | native | native | native |
| `LTRIM` | call | native | native | native | native |
| `MAKE_DATE` | call | native | native | rename:DATE | native (gap #209) |
| `MAKE_TIME` | call | native | unsupported | rename:TIME | native (gap #209) |
| `MAKE_TIMESTAMP` | call | native | native | rename:DATETIME | native (gap #209) |
| `MAKE_TIMESTAMPTZ` | call | native | unsupported | native (gap #179) | native (gap #209) |
| `MAX` | call | native | native | native | native |
| `MD5` | call | native | native | native (gap #179, divergent) | native (gap #209) |
| `MEDIAN` | call | native | agg:native; win:restructure:WindowToCte; run:unsupported | agg:rewrite:BigQueryMedian; win:rewrite:BigQueryMedian; run:unsupported (gap #179) | native (gap #209) |
| `MIN` | call | native | native | native | native |
| `MOD` | call | native | native | native | native |
| `MODE` | call | native | native | native (gap #179) | native (gap #209) |
| `MONTH` | call | native | native | native (gap #179) | native |
| `NOW` | call | native | native | rename:CURRENT_TIMESTAMP | native (gap #209) |
| `NTH_VALUE` | call | native | native | native | native |
| `NTILE` | call | native | native | native | native |
| `NULLIF` | call | native | native | native | native |
| `PERCENTILE_CONT` | call | agg:native; win:restructure:WindowToCte; run:unsupported | agg:native; win:restructure:WindowToCte; run:unsupported | agg:restructure:AnalyticToCte; win:rewrite:WithinGroupToAnalytic; run:unsupported (gap #179) | native (gap #209) |
| `PERCENTILE_DISC` | call | agg:native; win:restructure:WindowToCte; run:unsupported | agg:native; win:restructure:WindowToCte; run:unsupported | agg:restructure:AnalyticToCte; win:rewrite:WithinGroupToAnalytic; run:unsupported (gap #179) | native (gap #209) |
| `PERCENT_RANK` | call | native | native | native | native |
| `PI` | call | native | native | native (gap #179) | native |
| `POSITION` | call | native | native | native (gap #179) | native |
| `POW` | call | native | native | native | native |
| `POWER` | call | native | native | native (gap divergent) | native |
| `QUARTER` | call | native | native | native (gap #179) | native |
| `QUOTE_IDENT` | call | unsupported | unsupported | native (gap #179) | native (gap #209) |
| `QUOTE_LITERAL` | call | unsupported | unsupported | native (gap #179) | native (gap #209) |
| `RANDOM` | call | native | native | rename:RAND | native |
| `RANK` | call | native | native | native | native |
| `REGR_SLOPE` | call | native | native (gap divergent) | native (gap #179) | native |
| `REPEAT` | call | native | native | native | native (gap #209) |
| `REPLACE` | call | native | native | native | native |
| `REVERSE` | call | native | native | native | native |
| `RIGHT` | call | native | native | native | native (gap #209) |
| `ROUND` | call | native | native | native | native |
| `ROW_NUMBER` | call | native | native | native | native |
| `RPAD` | call | native | native | native | native |
| `RTRIM` | call | native | native | native | native |
| `SIGN` | call | native | native | native (gap #179) | native (gap #209) |
| `SIN` | call | native | native | native | native |
| `SINH` | call | native | native | native | native |
| `SPLIT_PART` | call | native | native | native (gap #179) | native |
| `SQRT` | call | native | native | native | native |
| `STDDEV` | call | native | native | native | native |
| `STDDEV_POP` | call | native | native | native | native |
| `STDDEV_SAMP` | call | native | native | native | native |
| `STRING_AGG` | call | native | native | native | native (gap #209) |
| `STRPOS` | call | native | rename:INSTR | native | native |
| `SUBSTR` | call | native | native | native | native |
| `SUBSTRING` | call | native | native | native | native |
| `SUM` | call | native | native | native | native |
| `TAN` | call | native | native | native | native |
| `TANH` | call | native | native | native | native |
| `TO_CHAR` | call | unsupported | native | native (gap #179) | native (gap #209) |
| `TO_JSON` | call | native | conditional(a0:composite→native | otherwise→unsupported) | native (gap divergent) | native (gap #209) |
| `TO_SECONDS` | call | native | template:make_interval(0, 0, 0, 0, 0, 0, {0}) (gap divergent) | native (gap #179) | native (gap #209) |
| `TRANSLATE` | call | native | native | native | native |
| `TRIM` | call | native | native | native | native |
| `TRUNC` | call | conditional(arity=2,a0:temporal,a1:string→unsupported | otherwise→native) | conditional(arity=2,a0:temporal,a1:string→native | otherwise→unsupported) | native (gap #179) | native (gap #209) |
| `TRUNCATE` | call | rename:TRUNC | unsupported | rename:TRUNC (gap #179) | native (gap #209) |
| `UNNEST` | table-fn | native (gap #176) | rename:EXPLODE (gap #176) | native (gap #179) | native (gap #209) |
| `UPPER` | call | native | native | native | native |
| `VARIANCE` | call | native | native | native | native |
| `VAR_POP` | call | native | native | native | native |
| `VAR_SAMP` | call | native | native | native | native |
| `YEAR` | call | native | native | native (gap #179) | native |
| `^` | infix | native | template:POWER({0}, {1}) | template:POWER({0}, {1}) (gap divergent) | template:POWER({0}, {1}) |
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
| Trino | schema only | every PR, or a PR labelled `run-docker-tests` — the value leg (`20260913-trino-emission` phase 6) is not live yet |

An untested `native` is reported as *unverified*, never as *passing*: the value leg
exists to test the claim, and a default-passing assumption would recreate exactly the
silent hole this audit was built to close.

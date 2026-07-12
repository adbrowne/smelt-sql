# External SQL corpus — attribution and license notices

This directory vendors a filtered, SELECT-only subset of SQL statements
extracted from two upstream test suites by `scripts/extract-sql-corpus.py`.
The extraction script is documented and re-runnable; it is **not** run in
CI — the extracted files below are committed and used directly by
`crates/smelt-parser-compat/tests/external_corpus.rs`.

## DuckDB (v1.5.0) — `duckdb.sql` (750 statements)

Source: https://github.com/duckdb/duckdb, `test/sql/**/*.test` sqllogictest
files (`statement ok` / `query <types>` blocks only).

DuckDB is licensed under the MIT License:

> Copyright 2018-2025 Stichting DuckDB Foundation
>
> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to
> deal in the Software without restriction, including without limitation the
> rights to use, copy, modify, merge, publish, distribute, sublicense, and/or
> sell copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the following conditions: the above
> copyright notice and this permission notice shall be included in all copies
> or substantial portions of the Software. THE SOFTWARE IS PROVIDED "AS IS",
> WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED.

## PostgreSQL (REL_16_4) — `postgres.sql` (750 statements)

Source: https://github.com/postgres/postgres,
`src/test/regress/sql/*.sql` regression-test scripts.

PostgreSQL is distributed under the PostgreSQL License, a liberal
MIT/BSD-style license:

> PostgreSQL Database Management System
> (formerly known as Postgres, then as Postgres95)
>
> Portions Copyright (c) 1996-2025, PostgreSQL Global Development Group
> Portions Copyright (c) 1994, The Regents of the University of California
>
> Permission to use, copy, modify, and distribute this software and its
> documentation for any purpose, without fee, and without a written agreement
> is hereby granted, provided that the above copyright notice and this
> paragraph and the following two paragraphs appear in all copies.

## Regenerating

```bash
python3 scripts/extract-sql-corpus.py
```

Bumps the pinned tags by editing `DUCKDB_TAG` / `POSTGRES_TAG` at the top of
the script first. After regenerating, re-run
`cargo test -p smelt-parser-compat --test external_corpus` and triage any
new failures into `../external_ledger.toml`.

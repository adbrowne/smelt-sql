window.BENCHMARK_DATA = {
  "lastUpdate": 1784112672097,
  "repoUrl": "https://github.com/adbrowne/smelt-sql",
  "entries": {
    "Smelt Latency Benchmarks": [
      {
        "commit": {
          "author": {
            "email": "brownie@brownie.com.au",
            "name": "Andrew Browne",
            "username": "adbrowne"
          },
          "committer": {
            "email": "brownie@brownie.com.au",
            "name": "Andrew Browne",
            "username": "adbrowne"
          },
          "distinct": true,
          "id": "564740540dd7cdda9c2d3a4252fb9d8d0664bd66",
          "message": "docs(research): conditional maintenance without a change feed\n\nResearch paper exploring change-suppressed writes (conditional MERGE /\nconditional DELETE+INSERT), delta-restricted enrichment compute, and\nderived change feeds via snapshot-diff fingerprinting — the properties\nand transforms needed, correctness against the equivalence invariant,\nspec tensions, and a prior-art survey (industry + academic IVM).\n\nCo-Authored-By: Claude Fable 5 <noreply@anthropic.com>",
          "timestamp": "2026-07-15T20:35:28+10:00",
          "tree_id": "1c47075a2dcd60c727d184b9022a4d56b2fa4f34",
          "url": "https://github.com/adbrowne/smelt-sql/commit/564740540dd7cdda9c2d3a4252fb9d8d0664bd66"
        },
        "date": 1784111900085,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 47.518199,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 45.534559,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.91041,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.546062,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.261498,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 878.321794,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.330664,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.3517680000000003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.105218,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.815834,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 737.9006019999999,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.1903,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.86556,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 14.37472,
            "unit": "ms"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "brownie@brownie.com.au",
            "name": "Andrew Browne",
            "username": "adbrowne"
          },
          "committer": {
            "email": "brownie@brownie.com.au",
            "name": "Andrew Browne",
            "username": "adbrowne"
          },
          "distinct": true,
          "id": "6a103bca0b6b7031d9e49f2d9fec6709adb25d4a",
          "message": "fix(ci): install DuckDB system lib in gap-report job\n\nThe gap-report job builds smelt-parser-compat, which links libduckdb,\nbut was missing the setup-duckdb action and DUCKDB_LIB_DIR env that\nevery other job in this workflow has, causing a linker failure\n(unable to find -lduckdb).",
          "timestamp": "2026-07-15T20:47:47+10:00",
          "tree_id": "4d4b6a1329a14f02bf021905230d637f816cd11a",
          "url": "https://github.com/adbrowne/smelt-sql/commit/6a103bca0b6b7031d9e49f2d9fec6709adb25d4a"
        },
        "date": 1784112667206,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 43.332231,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 41.601191,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.756898,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.498854,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.225055,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 712.430153,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.1267500000000004,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 1.647185,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 1.659353,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.5569200000000001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 588.560233,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.236909999999999,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 25.7647,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 10.695861,
            "unit": "ms"
          }
        ]
      }
    ],
    "Smelt Throughput Benchmarks": [
      {
        "commit": {
          "author": {
            "email": "brownie@brownie.com.au",
            "name": "Andrew Browne",
            "username": "adbrowne"
          },
          "committer": {
            "email": "brownie@brownie.com.au",
            "name": "Andrew Browne",
            "username": "adbrowne"
          },
          "distinct": true,
          "id": "564740540dd7cdda9c2d3a4252fb9d8d0664bd66",
          "message": "docs(research): conditional maintenance without a change feed\n\nResearch paper exploring change-suppressed writes (conditional MERGE /\nconditional DELETE+INSERT), delta-restricted enrichment compute, and\nderived change feeds via snapshot-diff fingerprinting — the properties\nand transforms needed, correctness against the equivalence invariant,\nspec tensions, and a prior-art survey (industry + academic IVM).\n\nCo-Authored-By: Claude Fable 5 <noreply@anthropic.com>",
          "timestamp": "2026-07-15T20:35:28+10:00",
          "tree_id": "1c47075a2dcd60c727d184b9022a4d56b2fa4f34",
          "url": "https://github.com/adbrowne/smelt-sql/commit/564740540dd7cdda9c2d3a4252fb9d8d0664bd66"
        },
        "date": 1784111903939,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 23.98140624652167,
            "unit": "MB/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "brownie@brownie.com.au",
            "name": "Andrew Browne",
            "username": "adbrowne"
          },
          "committer": {
            "email": "brownie@brownie.com.au",
            "name": "Andrew Browne",
            "username": "adbrowne"
          },
          "distinct": true,
          "id": "6a103bca0b6b7031d9e49f2d9fec6709adb25d4a",
          "message": "fix(ci): install DuckDB system lib in gap-report job\n\nThe gap-report job builds smelt-parser-compat, which links libduckdb,\nbut was missing the setup-duckdb action and DUCKDB_LIB_DIR env that\nevery other job in this workflow has, causing a linker failure\n(unable to find -lduckdb).",
          "timestamp": "2026-07-15T20:47:47+10:00",
          "tree_id": "4d4b6a1329a14f02bf021905230d637f816cd11a",
          "url": "https://github.com/adbrowne/smelt-sql/commit/6a103bca0b6b7031d9e49f2d9fec6709adb25d4a"
        },
        "date": 1784112671273,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 32.22985040661991,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}
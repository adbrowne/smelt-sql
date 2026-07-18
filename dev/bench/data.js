window.BENCHMARK_DATA = {
  "lastUpdate": 1784353955200,
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
          "id": "7ba0e99de870fac0678496d089c5f9c212401b3a",
          "message": "fix(test): register ROW_NUMBER/RANK Spark type divergence\n\nDuckDB's ROW_NUMBER() etc. return BIGINT (matches smelt's inference),\nbut Spark returns INT. The type-property Spark oracle was failing on\nthis genuine backend difference; register it in divergences.rs per\nthe type-oracle strictness gate instead of silently tolerating it.\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-07-18T01:20:50+10:00",
          "tree_id": "022a3ad0400b63e30e8bcbde15154246cd990c49",
          "url": "https://github.com/adbrowne/smelt-sql/commit/7ba0e99de870fac0678496d089c5f9c212401b3a"
        },
        "date": 1784301921420,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 55.595873,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 53.283277,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.075042,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.616515,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.312498,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 932.675618,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.870301,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.3039039999999997,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.1985550000000003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.706489,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 771.375295,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 7.03154,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 32.87338,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.618203,
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
          "id": "25a34a152bc59c6f95fefbb05f2807860d5a5b94",
          "message": "fix(test): register cast_float_as_double divergence for Spark oracle\n\nCI's dual-target property test failed against Spark: CAST(DECIMAL AS\nFLOAT) returns FLOAT there too (smelt normalizes to DOUBLE), but the\nexisting cast_float_as_double divergence only listed a duckdb_type.\nAdd the matching spark_type and a regression test.",
          "timestamp": "2026-07-18T15:48:10+10:00",
          "tree_id": "35058021b526be54fb85b802fb4b3cad9b470eed",
          "url": "https://github.com/adbrowne/smelt-sql/commit/25a34a152bc59c6f95fefbb05f2807860d5a5b94"
        },
        "date": 1784353953807,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 56.144268,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 53.802209,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.093524,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.645479,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.296871,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 933.833557,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.840084,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.226969,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.171546,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.780679,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 762.525123,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.66941,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 32.843,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.638866,
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
          "id": "7ba0e99de870fac0678496d089c5f9c212401b3a",
          "message": "fix(test): register ROW_NUMBER/RANK Spark type divergence\n\nDuckDB's ROW_NUMBER() etc. return BIGINT (matches smelt's inference),\nbut Spark returns INT. The type-property Spark oracle was failing on\nthis genuine backend difference; register it in divergences.rs per\nthe type-oracle strictness gate instead of silently tolerating it.\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-07-18T01:20:50+10:00",
          "tree_id": "022a3ad0400b63e30e8bcbde15154246cd990c49",
          "url": "https://github.com/adbrowne/smelt-sql/commit/7ba0e99de870fac0678496d089c5f9c212401b3a"
        },
        "date": 1784301924479,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 25.313618837962686,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}
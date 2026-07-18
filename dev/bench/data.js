window.BENCHMARK_DATA = {
  "lastUpdate": 1784367448225,
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
          "id": "9dec8f69845856f4cae43aa0a1ecab4a25ccd9f5",
          "message": "fix(test): register date_plus_interval Spark divergence, fix interval oracle parsing\n\nLocal soak run (PROPTEST_CASES beyond CI's 256) surfaced two more issues in\nthe type_property_tests dual-target oracle:\n\n- DATE + INTERVAL infers Timestamp in smelt (matches DuckDB), but Spark\n  returns DATE for a day/year-month-granularity interval. Register as\n  date_plus_interval.\n- spark_type_to_smelt only exact-matched the string \"interval\", but Spark's\n  typeof() always qualifies granularity (e.g. \"interval day to second\"),\n  so TIMESTAMP - TIMESTAMP misreported as Unknown(Dynamic) instead of\n  Interval. Fixed to prefix-match \"interval*\".\n\nEach fix includes a regression test.",
          "timestamp": "2026-07-18T16:53:36+10:00",
          "tree_id": "43713079368705677f0ab94beebb4a71923bc58d",
          "url": "https://github.com/adbrowne/smelt-sql/commit/9dec8f69845856f4cae43aa0a1ecab4a25ccd9f5"
        },
        "date": 1784357864589,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 58.47537,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 56.394572,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.848664,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.62312,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.31426099999999996,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 918.241821,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.167801,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.124981,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.127927,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.64426,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 753.038507,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.85227,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 34.035830000000004,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.864434,
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
          "id": "61b4673dc6323a6baaf8a08c9ca683ff9559b919",
          "message": "fix(test): register median_decimal Spark type divergence\n\n10k-case local soak run caught MEDIAN(DECIMAL) diverging against Spark:\nsmelt/DuckDB preserve the input Decimal type, but Spark's MEDIAN is\nimplemented via percentile_cont and always returns DOUBLE.",
          "timestamp": "2026-07-18T19:20:52+10:00",
          "tree_id": "ecc81a026d48dfd3b944e2b28ff2eed2710a1c03",
          "url": "https://github.com/adbrowne/smelt-sql/commit/61b4673dc6323a6baaf8a08c9ca683ff9559b919"
        },
        "date": 1784366715887,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 58.62426,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 56.49168,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.857765,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.634907,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.350606,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 923.298117,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.062917,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.568486,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.431549,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.712272,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 759.281246,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.888190000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 34.38162,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 14.371056,
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
          "id": "036775a57e668b65bc7f6b76ae74294b058c3162",
          "message": "fix(test): register decimal_add_overflow known-unknown, unwrap Array in find_divergence\n\n10k-case soak run found two more genuine gaps:\n\n- `up_0 + up_0` (Decimal(38,10), already at max precision from a prior SUM)\n  overflows past precision 38 and smelt correctly returns Unknown\n  (fail-loud, spec §15) — but only *´ and / had known_unknowns.rs entries,\n  not +. Register decimal_add_overflow, mirroring the existing pattern.\n- ARRAY_AGG(dec_col * dec_col) diverges against DuckDB the same way bare\n  decimal multiplication does (decimal_arithmetic_model), but the\n  divergence matcher didn't unwrap the Array wrapping ARRAY_AGG adds.\n  find_divergence now unwraps one level of Array before matching, so\n  existing element-level entries apply under aggregation without needing\n  duplicate Array-of-X entries.",
          "timestamp": "2026-07-18T19:32:43+10:00",
          "tree_id": "c85be82069bd9e54bdd4e00f2b3ce07fef44a425",
          "url": "https://github.com/adbrowne/smelt-sql/commit/036775a57e668b65bc7f6b76ae74294b058c3162"
        },
        "date": 1784367445982,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 60.00886,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 57.411898,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.241405,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.661548,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.385531,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 925.366556,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.5045520000000003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.254401,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.158782,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.642213,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 761.7589879999999,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.17115,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 34.15525,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.944558,
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
        "date": 1784353956881,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 25.275268486397625,
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
          "id": "9dec8f69845856f4cae43aa0a1ecab4a25ccd9f5",
          "message": "fix(test): register date_plus_interval Spark divergence, fix interval oracle parsing\n\nLocal soak run (PROPTEST_CASES beyond CI's 256) surfaced two more issues in\nthe type_property_tests dual-target oracle:\n\n- DATE + INTERVAL infers Timestamp in smelt (matches DuckDB), but Spark\n  returns DATE for a day/year-month-granularity interval. Register as\n  date_plus_interval.\n- spark_type_to_smelt only exact-matched the string \"interval\", but Spark's\n  typeof() always qualifies granularity (e.g. \"interval day to second\"),\n  so TIMESTAMP - TIMESTAMP misreported as Unknown(Dynamic) instead of\n  Interval. Fixed to prefix-match \"interval*\".\n\nEach fix includes a regression test.",
          "timestamp": "2026-07-18T16:53:36+10:00",
          "tree_id": "43713079368705677f0ab94beebb4a71923bc58d",
          "url": "https://github.com/adbrowne/smelt-sql/commit/9dec8f69845856f4cae43aa0a1ecab4a25ccd9f5"
        },
        "date": 1784357868701,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.86405142828045,
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
          "id": "61b4673dc6323a6baaf8a08c9ca683ff9559b919",
          "message": "fix(test): register median_decimal Spark type divergence\n\n10k-case local soak run caught MEDIAN(DECIMAL) diverging against Spark:\nsmelt/DuckDB preserve the input Decimal type, but Spark's MEDIAN is\nimplemented via percentile_cont and always returns DOUBLE.",
          "timestamp": "2026-07-18T19:20:52+10:00",
          "tree_id": "ecc81a026d48dfd3b944e2b28ff2eed2710a1c03",
          "url": "https://github.com/adbrowne/smelt-sql/commit/61b4673dc6323a6baaf8a08c9ca683ff9559b919"
        },
        "date": 1784366719158,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 23.98752047170368,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}
window.BENCHMARK_DATA = {
  "lastUpdate": 1784023426517,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "2be3033b072fb5c2117b94e7e8092d365334abe0",
          "message": "Merge pull request #160 from adbrowne/web-analytics-tutorial",
          "timestamp": "2026-07-14T19:24:44+10:00",
          "tree_id": "15b0a9eff84b5272428b690b90320f44c4f6c13e",
          "url": "https://github.com/adbrowne/smelt-sql/commit/2be3033b072fb5c2117b94e7e8092d365334abe0"
        },
        "date": 1784021331993,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 60.276031,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 57.807465,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.036002,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.648535,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.375564,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 930.144885,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.37103,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.819406,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.595036,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.690995,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 767.312961,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.82579,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.787150000000004,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.900651,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "feda5e1b480cef9aa110da0570e0f8f7cbd3434d",
          "message": "Merge pull request #157 from adbrowne/worktree-incremental_2",
          "timestamp": "2026-07-14T19:24:26+10:00",
          "tree_id": "41c62ed138153c0bedd17428be22b38a69eb8da6",
          "url": "https://github.com/adbrowne/smelt-sql/commit/feda5e1b480cef9aa110da0570e0f8f7cbd3434d"
        },
        "date": 1784021342447,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 54.619574,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 52.259350000000005,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.131956,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.624665,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.29373499999999997,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 936.965983,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 4.004378,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.367925,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.282079,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.799113,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 771.7269759999999,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.778440000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.064949999999996,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.752961,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "6a169e00536aaec36acceaae8781b8440072afba",
          "message": "fix(test): parse Spark ARRAY<...> types in type-property Spark oracle (#162)\n\n* fix(test): parse Spark ARRAY<...> types in the type-property Spark oracle\n\nDESCRIBE QUERY output for ARRAY_AGG(BOOLEAN) reports \"array<boolean>\",\nwhich the oracle's type mapper didn't recognize and fell back to\nUnknown(Dynamic), causing prop_type_inference to fail against smelt's\ncorrect Array(Boolean) inference. Recurse into the element type instead.\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>\n\n* fix(core): prune nested-project subtrees from a parent project's file walk\n\nproject_root_files_by_dir walked a project's entire subtree unbounded,\nso a parent project's discovery also claimed files belonging to a\nnested project (one with its own smelt.yml/smelt.yaml). The same\nabsolute path then got registered under two projects, corrupting\nworkspace-wide checks keyed on file identity — e.g. the LSP's\nduplicate-function-name diagnostic firing against the file itself when\na nested project's function collided by path with its own double\nregistration (examples/web_analytics/tutorial_stages/05_enrichment,\nadded by the web-analytics-tutorial PR, first exposed this).\n\nPer the \"Project isolation rule\", a nested project owns its files\nexclusively, so its directory is now a walk boundary alongside the\nexisting dotfile/target exclusions.\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-07-14T20:01:47+10:00",
          "tree_id": "818d551e1f4493b8e5f8f3daa85c399acd4606c0",
          "url": "https://github.com/adbrowne/smelt-sql/commit/6a169e00536aaec36acceaae8781b8440072afba"
        },
        "date": 1784023422900,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 55.277039,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 52.641833,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.3859860000000002,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.63124,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.29159900000000005,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 931.508788,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 4.275896,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.6739960000000003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.1405119999999997,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.734155,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 768.830066,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.941549999999999,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 32.90673,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.669736,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "2be3033b072fb5c2117b94e7e8092d365334abe0",
          "message": "Merge pull request #160 from adbrowne/web-analytics-tutorial",
          "timestamp": "2026-07-14T19:24:44+10:00",
          "tree_id": "15b0a9eff84b5272428b690b90320f44c4f6c13e",
          "url": "https://github.com/adbrowne/smelt-sql/commit/2be3033b072fb5c2117b94e7e8092d365334abe0"
        },
        "date": 1784021335799,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.799270192453573,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "feda5e1b480cef9aa110da0570e0f8f7cbd3434d",
          "message": "Merge pull request #157 from adbrowne/worktree-incremental_2",
          "timestamp": "2026-07-14T19:24:26+10:00",
          "tree_id": "41c62ed138153c0bedd17428be22b38a69eb8da6",
          "url": "https://github.com/adbrowne/smelt-sql/commit/feda5e1b480cef9aa110da0570e0f8f7cbd3434d"
        },
        "date": 1784021345208,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 25.065584058589277,
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
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "6a169e00536aaec36acceaae8781b8440072afba",
          "message": "fix(test): parse Spark ARRAY<...> types in type-property Spark oracle (#162)\n\n* fix(test): parse Spark ARRAY<...> types in the type-property Spark oracle\n\nDESCRIBE QUERY output for ARRAY_AGG(BOOLEAN) reports \"array<boolean>\",\nwhich the oracle's type mapper didn't recognize and fell back to\nUnknown(Dynamic), causing prop_type_inference to fail against smelt's\ncorrect Array(Boolean) inference. Recurse into the element type instead.\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>\n\n* fix(core): prune nested-project subtrees from a parent project's file walk\n\nproject_root_files_by_dir walked a project's entire subtree unbounded,\nso a parent project's discovery also claimed files belonging to a\nnested project (one with its own smelt.yml/smelt.yaml). The same\nabsolute path then got registered under two projects, corrupting\nworkspace-wide checks keyed on file identity — e.g. the LSP's\nduplicate-function-name diagnostic firing against the file itself when\na nested project's function collided by path with its own double\nregistration (examples/web_analytics/tutorial_stages/05_enrichment,\nadded by the web-analytics-tutorial PR, first exposed this).\n\nPer the \"Project isolation rule\", a nested project owns its files\nexclusively, so its directory is now a walk boundary alongside the\nexisting dotfile/target exclusions.\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>\n\n---------\n\nCo-authored-by: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-07-14T20:01:47+10:00",
          "tree_id": "818d551e1f4493b8e5f8f3daa85c399acd4606c0",
          "url": "https://github.com/adbrowne/smelt-sql/commit/6a169e00536aaec36acceaae8781b8440072afba"
        },
        "date": 1784023425881,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 25.218190022104306,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}
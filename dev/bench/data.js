window.BENCHMARK_DATA = {
  "lastUpdate": 1788899676136,
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
          "id": "af60af612e0aafd34c043ed4c6e571b7fd029da5",
          "message": "Merge pull request #186 from adbrowne/outcome-loop-20260904-programme-hygiene\n\noutcome: 2026-09-04 programme hygiene + backlog",
          "timestamp": "2026-09-07T20:02:38+10:00",
          "tree_id": "ecb8feaeb7fe48dc2761b8634599433c2b672181",
          "url": "https://github.com/adbrowne/smelt-sql/commit/af60af612e0aafd34c043ed4c6e571b7fd029da5"
        },
        "date": 1788775581197,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 43.924268000000005,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 42.067083,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.889564,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.482323,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.242709,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 930.160153,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.2341990000000003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 1.770286,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 1.709947,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.536173,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 780.8629860000001,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.611689999999999,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 25.81785,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 10.740861,
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
          "id": "01a328b3b6386d8e75daed67f98b47767ad459d2",
          "message": "Merge pull request #199 from adbrowne/fix-round-trip-line-comment-fuzz-crash\n\nfix(smelt-parser): preserve line-comment newline when printing raw source spans",
          "timestamp": "2026-09-09T06:29:38+10:00",
          "tree_id": "d7b32c65004aa073a49dc85a12bdd8599fcbd9a2",
          "url": "https://github.com/adbrowne/smelt-sql/commit/01a328b3b6386d8e75daed67f98b47767ad459d2"
        },
        "date": 1788899668372,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 60.057936,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 57.838049000000005,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.949841,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.6093999999999999,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.35591,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1194.2778890000002,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.31689,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.2202960000000003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.153072,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.6366310000000001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 991.755549,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 6.103620000000001,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 33.4965,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.804744,
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
          "id": "af60af612e0aafd34c043ed4c6e571b7fd029da5",
          "message": "Merge pull request #186 from adbrowne/outcome-loop-20260904-programme-hygiene\n\noutcome: 2026-09-04 programme hygiene + backlog",
          "timestamp": "2026-09-07T20:02:38+10:00",
          "tree_id": "ecb8feaeb7fe48dc2761b8634599433c2b672181",
          "url": "https://github.com/adbrowne/smelt-sql/commit/af60af612e0aafd34c043ed4c6e571b7fd029da5"
        },
        "date": 1788775585043,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 32.094819959033074,
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
          "id": "01a328b3b6386d8e75daed67f98b47767ad459d2",
          "message": "Merge pull request #199 from adbrowne/fix-round-trip-line-comment-fuzz-crash\n\nfix(smelt-parser): preserve line-comment newline when printing raw source spans",
          "timestamp": "2026-09-09T06:29:38+10:00",
          "tree_id": "d7b32c65004aa073a49dc85a12bdd8599fcbd9a2",
          "url": "https://github.com/adbrowne/smelt-sql/commit/01a328b3b6386d8e75daed67f98b47767ad459d2"
        },
        "date": 1788899674571,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 24.97156050123059,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}
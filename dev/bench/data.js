window.BENCHMARK_DATA = {
  "lastUpdate": 1788344007742,
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
          "id": "982240372ab537806ce94b77fa4f10bcb2087438",
          "message": "Merge pull request #184 from adbrowne/annes-words\n\nAnnes words",
          "timestamp": "2026-09-02T20:10:49+10:00",
          "tree_id": "7868874ada8805faeb9271c9f2f858f9a85ba382",
          "url": "https://github.com/adbrowne/smelt-sql/commit/982240372ab537806ce94b77fa4f10bcb2087438"
        },
        "date": 1788344003240,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 43.415678,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 41.598918999999995,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.853001,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.461288,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.248606,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 847.4799370000001,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.210218,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 1.713152,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 1.668825,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.576242,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 701.166891,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 4.69421,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 24.92301,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 10.537015,
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
          "id": "982240372ab537806ce94b77fa4f10bcb2087438",
          "message": "Merge pull request #184 from adbrowne/annes-words\n\nAnnes words",
          "timestamp": "2026-09-02T20:10:49+10:00",
          "tree_id": "7868874ada8805faeb9271c9f2f858f9a85ba382",
          "url": "https://github.com/adbrowne/smelt-sql/commit/982240372ab537806ce94b77fa4f10bcb2087438"
        },
        "date": 1788344007133,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 32.715716927422044,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}
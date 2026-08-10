window.BENCHMARK_DATA = {
  "lastUpdate": 1786350123086,
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
          "id": "96334a84c4a3c02cdc130e0983a81829fe7d9fb8",
          "message": "docs(agents): configure Matt Pocock engineering skills for this repo\n\nWires the issue tracker (GitHub Issues via gh) and domain-docs layout\n(single-context CONTEXT.md + docs/adr/) that mattpocock-skills expects.\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-08-09T12:58:44+10:00",
          "tree_id": "ef0ebf8b0b6d33bd26135f6036acd30f4742d835",
          "url": "https://github.com/adbrowne/smelt-sql/commit/96334a84c4a3c02cdc130e0983a81829fe7d9fb8"
        },
        "date": 1786244579028,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 39.831117,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 38.243412,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.69676,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.464668,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.212003,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 816.616433,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 2.199419,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 1.951928,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 1.833361,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.668063,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 684.495018,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.69086,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 28.45593,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 12.129233,
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
          "id": "ae3293441bdb3324927515374e5e4968a6f19e3c",
          "message": "fix(ci): don't fail Dev Release when TestPyPI publish hits its storage quota\n\nTestPyPI's 10 GB storage limit for the smelt-sql project has been exhausted\nsince mid-July: every push to main uploads a full wheel set under a unique\ndev version and nothing ever ages out, so the publish step has failed on\nevery run since. That's an external quota issue (needs manual cleanup or a\nlimit-increase request on test.pypi.org), not something CI can self-heal, so\nstop letting it fail the whole Dev Release workflow.\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-08-10T18:17:18+10:00",
          "tree_id": "9af5350dc3929e71c458d205aa6853da972b93b3",
          "url": "https://github.com/adbrowne/smelt-sql/commit/ae3293441bdb3324927515374e5e4968a6f19e3c"
        },
        "date": 1786350121261,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 56.094482,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 53.879006,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 0.967892,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.611433,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.321025,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1012.391425,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.671034,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.153953,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.133452,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.740969,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 818.70837,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 5.99515,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 32.25892,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.590204,
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
          "id": "96334a84c4a3c02cdc130e0983a81829fe7d9fb8",
          "message": "docs(agents): configure Matt Pocock engineering skills for this repo\n\nWires the issue tracker (GitHub Issues via gh) and domain-docs layout\n(single-context CONTEXT.md + docs/adr/) that mattpocock-skills expects.\n\nCo-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>",
          "timestamp": "2026-08-09T12:58:44+10:00",
          "tree_id": "ef0ebf8b0b6d33bd26135f6036acd30f4742d835",
          "url": "https://github.com/adbrowne/smelt-sql/commit/96334a84c4a3c02cdc130e0983a81829fe7d9fb8"
        },
        "date": 1786244582560,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 28.421088126512203,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}
window.BENCHMARK_DATA = {
  "lastUpdate": 1788689778168,
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
          "id": "872b8b3ff52f5be66b733e7565ae15e5d6f8d0c3",
          "message": "docs: ext4 has no discard=async; use a daily fstrim timer\n\n`discard=async` is a btrfs mount option. ext4 offers only `discard`\n(synchronous) and `nodiscard` -- `/proc/fs/ext4/nvme0n1p2/options` lists\n`nodiscard`, and `mount -o remount,discard=async /` fails with \"bad\noption\". Synchronous discard is a write-path cost this drive can ill\nafford, so periodic trim is the mechanism; the stock weekly fstrim.timer\nis what starved it. Recommend daily instead.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>",
          "timestamp": "2026-09-06T20:13:38+10:00",
          "tree_id": "cfff81f1cf11542219b7bc33a24ca9dc96e4ee51",
          "url": "https://github.com/adbrowne/smelt-sql/commit/872b8b3ff52f5be66b733e7565ae15e5d6f8d0c3"
        },
        "date": 1788689772248,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "Build / Total",
            "value": 57.827591,
            "unit": "ms"
          },
          {
            "name": "Build / Discovery",
            "value": 55.432552,
            "unit": "ms"
          },
          {
            "name": "Build / Graph Build",
            "value": 1.149644,
            "unit": "ms"
          },
          {
            "name": "Build / Topo Sort",
            "value": 0.627224,
            "unit": "ms"
          },
          {
            "name": "Build / Validation",
            "value": 0.298129,
            "unit": "ms"
          },
          {
            "name": "Salsa / Initial Load",
            "value": 1207.844295,
            "unit": "ms"
          },
          {
            "name": "Salsa / Leaf Edit Diagnostics",
            "value": 3.867769,
            "unit": "ms"
          },
          {
            "name": "Salsa / Mid Edit Diagnostics",
            "value": 2.397242,
            "unit": "ms"
          },
          {
            "name": "Salsa / Root Edit Diagnostics",
            "value": 2.231409,
            "unit": "ms"
          },
          {
            "name": "Salsa / Add File",
            "value": 0.739068,
            "unit": "ms"
          },
          {
            "name": "Salsa / Full Diagnostics",
            "value": 1008.860272,
            "unit": "ms"
          },
          {
            "name": "Parser / Simple SQL",
            "value": 7.40371,
            "unit": "μs"
          },
          {
            "name": "Parser / Complex SQL",
            "value": 32.126039999999996,
            "unit": "μs"
          },
          {
            "name": "Parser / Batch (1000)",
            "value": 13.479512,
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
          "id": "872b8b3ff52f5be66b733e7565ae15e5d6f8d0c3",
          "message": "docs: ext4 has no discard=async; use a daily fstrim timer\n\n`discard=async` is a btrfs mount option. ext4 offers only `discard`\n(synchronous) and `nodiscard` -- `/proc/fs/ext4/nvme0n1p2/options` lists\n`nodiscard`, and `mount -o remount,discard=async /` fails with \"bad\noption\". Synchronous discard is a write-path cost this drive can ill\nafford, so periodic trim is the mechanism; the stock weekly fstrim.timer\nis what starved it. Recommend daily instead.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>",
          "timestamp": "2026-09-06T20:13:38+10:00",
          "tree_id": "cfff81f1cf11542219b7bc33a24ca9dc96e4ee51",
          "url": "https://github.com/adbrowne/smelt-sql/commit/872b8b3ff52f5be66b733e7565ae15e5d6f8d0c3"
        },
        "date": 1788689776967,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Parser / Throughput",
            "value": 25.574071227504376,
            "unit": "MB/s"
          }
        ]
      }
    ]
  }
}
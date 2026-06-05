window.BENCHMARK_DATA = {
  "lastUpdate": 1780639586266,
  "repoUrl": "https://github.com/josongmin/quanta-taskmesh",
  "entries": {
    "taskmesh wall-clock": [
      {
        "commit": {
          "author": {
            "email": "songmin@Joui-MacBookPro.local",
            "name": "Jo"
          },
          "committer": {
            "email": "songmin@Joui-MacBookPro.local",
            "name": "Jo"
          },
          "distinct": true,
          "id": "1941724778df158361b209c364714b5d35480cd0",
          "message": "fix(runtime): honor cancellation before substrate waits",
          "timestamp": "2026-06-05T15:02:58+09:00",
          "tree_id": "d64f87672dfd928b0a6f41cf740cdcc273b971e8",
          "url": "https://github.com/josongmin/quanta-taskmesh/commit/1941724778df158361b209c364714b5d35480cd0"
        },
        "date": 1780639585972,
        "tool": "cargo",
        "benches": [
          {
            "name": "admit_release_success",
            "value": 187,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "admit_unknown_class_reject",
            "value": 27,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/governed",
            "value": 24488,
            "range": "± 1489",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/raw_spawn_blocking",
            "value": 23499,
            "range": "± 1174",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/semaphore_only",
            "value": 23636,
            "range": "± 946",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/tower_concurrency_limit",
            "value": 23665,
            "range": "± 867",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}
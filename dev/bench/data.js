window.BENCHMARK_DATA = {
  "lastUpdate": 1784447056897,
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
      },
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
          "id": "6582935935cae10b93d78b2a8fc642bbe6ac92e1",
          "message": "test: satisfy strict gate on examples and inventory checks",
          "timestamp": "2026-06-06T02:54:53+09:00",
          "tree_id": "e3e3cf6bcbf53fead4d3e8ba3ef21b492838f43d",
          "url": "https://github.com/josongmin/quanta-taskmesh/commit/6582935935cae10b93d78b2a8fc642bbe6ac92e1"
        },
        "date": 1780682245262,
        "tool": "cargo",
        "benches": [
          {
            "name": "admit_release_success",
            "value": 186,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "admit_unknown_class_reject",
            "value": 40,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/governed",
            "value": 24490,
            "range": "± 2297",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/raw_spawn_blocking",
            "value": 23557,
            "range": "± 597",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/semaphore_only",
            "value": 23541,
            "range": "± 692",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/tower_concurrency_limit",
            "value": 23770,
            "range": "± 715",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/governed_pre_submit_cancelled",
            "value": 305,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/governed_substrate_gate_timeout_zero_wait",
            "value": 1085924,
            "range": "± 22137",
            "unit": "ns/iter"
          }
        ]
      },
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
          "id": "91ab0b44b663e57c672f00ad611b46321184b54e",
          "message": "feat: support requested blocking stack size",
          "timestamp": "2026-07-18T19:32:32+09:00",
          "tree_id": "793979900f9bb945b29812f2b72877ce03e0c2e6",
          "url": "https://github.com/josongmin/quanta-taskmesh/commit/91ab0b44b663e57c672f00ad611b46321184b54e"
        },
        "date": 1784370904972,
        "tool": "cargo",
        "benches": [
          {
            "name": "admit_release_success",
            "value": 170,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "admit_unknown_class_reject",
            "value": 25,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/governed",
            "value": 27482,
            "range": "± 2646",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/raw_spawn_blocking",
            "value": 25751,
            "range": "± 2399",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/semaphore_only",
            "value": 26182,
            "range": "± 2319",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/tower_concurrency_limit",
            "value": 27113,
            "range": "± 2442",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/governed_pre_submit_cancelled",
            "value": 336,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/governed_substrate_gate_timeout_zero_wait",
            "value": 1090970,
            "range": "± 42541",
            "unit": "ns/iter"
          }
        ]
      },
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
          "id": "9ae9547216c70458f54f37368a67661321060886",
          "message": "fix(runtime): close cancellation admission races",
          "timestamp": "2026-07-19T16:07:04+09:00",
          "tree_id": "10299bbfdc15910c0d3a5afabaeb679ee3964b75",
          "url": "https://github.com/josongmin/quanta-taskmesh/commit/9ae9547216c70458f54f37368a67661321060886"
        },
        "date": 1784447056568,
        "tool": "cargo",
        "benches": [
          {
            "name": "admit_release_success",
            "value": 184,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "admit_unknown_class_reject",
            "value": 26,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/governed",
            "value": 24542,
            "range": "± 884",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/raw_spawn_blocking",
            "value": 23096,
            "range": "± 697",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/semaphore_only",
            "value": 23379,
            "range": "± 688",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/tower_concurrency_limit",
            "value": 23292,
            "range": "± 541",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/governed_pre_submit_cancelled",
            "value": 313,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/governed_substrate_gate_timeout_zero_wait",
            "value": 1078345,
            "range": "± 23139",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}
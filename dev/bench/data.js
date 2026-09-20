window.BENCHMARK_DATA = {
  "lastUpdate": 1789939362854,
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
      },
      {
        "commit": {
          "author": {
            "email": "songmin.jo@twelvelabs.io",
            "name": "Songmin Jo",
            "username": "songminjo"
          },
          "committer": {
            "email": "songmin.jo@twelvelabs.io",
            "name": "Songmin Jo",
            "username": "songminjo"
          },
          "distinct": true,
          "id": "3da26491997c81435c969bd05b8a20438e360f99",
          "message": "docs: freeze SEP-21 campaign baseline",
          "timestamp": "2026-09-21T04:14:07+09:00",
          "tree_id": "65a9061dcae7c815ab99477f0d98eabec1fcba8e",
          "url": "https://github.com/josongmin/quanta-taskmesh/commit/3da26491997c81435c969bd05b8a20438e360f99"
        },
        "date": 1789931907553,
        "tool": "cargo",
        "benches": [
          {
            "name": "admit_release_success",
            "value": 286,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "admit_unknown_class_reject",
            "value": 66,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/governed",
            "value": 28452,
            "range": "± 2525",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/raw_spawn_blocking",
            "value": 26909,
            "range": "± 2556",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/semaphore_only",
            "value": 27586,
            "range": "± 2191",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/tower_concurrency_limit",
            "value": 27804,
            "range": "± 3010",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/governed_pre_submit_cancelled",
            "value": 342,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/governed_substrate_saturated_shed",
            "value": 430,
            "range": "± 14",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "songmin.jo@twelvelabs.io",
            "name": "Songmin Jo",
            "username": "songminjo"
          },
          "committer": {
            "email": "songmin.jo@twelvelabs.io",
            "name": "Songmin Jo",
            "username": "songminjo"
          },
          "distinct": true,
          "id": "ba643a130dfdaf091099bb11099962198d0e5981",
          "message": "feat(host): enforce bounded dispatch and acquisition authority",
          "timestamp": "2026-09-21T05:59:14+09:00",
          "tree_id": "bf851fe79fa51e4af94741fb0eb14b30512b28ec",
          "url": "https://github.com/josongmin/quanta-taskmesh/commit/ba643a130dfdaf091099bb11099962198d0e5981"
        },
        "date": 1789938089510,
        "tool": "cargo",
        "benches": [
          {
            "name": "admit_release_success",
            "value": 582,
            "range": "± 21",
            "unit": "ns/iter"
          },
          {
            "name": "admit_unknown_class_reject",
            "value": 152,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/governed",
            "value": 18341,
            "range": "± 785",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/raw_spawn_blocking",
            "value": 16678,
            "range": "± 570",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/semaphore_only",
            "value": 16864,
            "range": "± 645",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/tower_concurrency_limit",
            "value": 17181,
            "range": "± 2051",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/governed_pre_submit_cancelled",
            "value": 551,
            "range": "± 11",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/governed_substrate_saturated_shed",
            "value": 744,
            "range": "± 11",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "songmin.jo@twelvelabs.io",
            "name": "Songmin Jo",
            "username": "songminjo"
          },
          "committer": {
            "email": "songmin.jo@twelvelabs.io",
            "name": "Songmin Jo",
            "username": "songminjo"
          },
          "distinct": true,
          "id": "df08a5923474d9a69b4b73ccdd6f3b9968cba84a",
          "message": "proof: isolate and bind mutation campaigns",
          "timestamp": "2026-09-21T06:20:27+09:00",
          "tree_id": "a1febdbccfd71561355d34e6add71d0615fbafd6",
          "url": "https://github.com/josongmin/quanta-taskmesh/commit/df08a5923474d9a69b4b73ccdd6f3b9968cba84a"
        },
        "date": 1789939362086,
        "tool": "cargo",
        "benches": [
          {
            "name": "admit_release_success",
            "value": 622,
            "range": "± 35",
            "unit": "ns/iter"
          },
          {
            "name": "admit_unknown_class_reject",
            "value": 182,
            "range": "± 8",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/governed",
            "value": 31668,
            "range": "± 4951",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/raw_spawn_blocking",
            "value": 31285,
            "range": "± 7092",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/semaphore_only",
            "value": 31548,
            "range": "± 7582",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/tower_concurrency_limit",
            "value": 29560,
            "range": "± 4671",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/governed_pre_submit_cancelled",
            "value": 584,
            "range": "± 25",
            "unit": "ns/iter"
          },
          {
            "name": "governance_tax_blocking_noop/governed_substrate_saturated_shed",
            "value": 732,
            "range": "± 44",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}
# T10 CI Proof Docs Release

## Summary

proof rail, docs compileability, release baseline을 만든다.

## Decisions Frozen

1. docs example는 compile proof를 가진다.
2. CI는 drift detector 역할을 해야 한다.
3. baseline-ratchet는 runtime substrate 규칙을 문서와 같이 간다.
4. release checklist는 first stable API 기준으로 남긴다.

## Files To Touch

1. `Cargo.toml`
2. `README.md`
3. `docs/taskmesh-library-spec.md`
4. `docs/taskmesh-external-interface.md`
5. `.github/workflows/ci.yml` 신규 또는 `docs/plans/.../ci-local.md` 신규
6. `crates/taskmesh-contract/tests/` 보강
7. `crates/taskmesh-core/tests/` 보강
8. `crates/taskmesh-tokio/tests/` 보강
9. `crates/taskmesh-rayon/tests/` 보강

## Implementation

1. 최소 CI/proof matrix를 정의한다.
   - `cargo check`
   - `cargo test`
   - doctest/example compile
2. public docs example를 실제 코드에 맞춘다.
3. baseline-ratchet 전략을 README/spec에 명시한다.
   - raw spawn
   - unbounded competing queue
   - engine-pool pattern
4. first release checklist를 남긴다.
   - semver scope
   - doc sync
   - test matrix
   - known residue
5. package metadata/readme/doc paths가 publishable shape인지 점검한다.

구체 선택:

1. CI runner
   - GitHub Actions 기본
2. actions
   - `actions/checkout@v4`
   - `dtolnay/rust-toolchain@stable`
   - `Swatinem/rust-cache@v2`
3. commands
   - `cargo check --workspace`
   - `cargo test --workspace`
   - `cargo test --doc -p taskmesh`
4. docs compile strategy
   - README snippets는 doctest 또는 compile-only integration example
5. baseline-ratchet artifact
   - `docs/runtime-inventory-baseline.md` 또는 `tests/fixtures/substrate_allowlist.json`
6. release checklist
   - crate version sync
   - README example green
   - docs/spec/interface sync
   - public API review

스니펫:

```yaml
- run: cargo check --workspace
- run: cargo test --workspace
- run: cargo test --doc -p taskmesh
```

## Tests

1. `cargo check`
2. `cargo test`
3. doctest/example compile
4. regression tests for validation/fairness/memory/composite/runtime paths

## Not Done If

1. docs example가 코드와 안 맞는다.
2. CI가 public API drift를 못 잡는다.
3. release checklist가 없다.

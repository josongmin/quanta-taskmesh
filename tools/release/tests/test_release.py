"""R01 release-only negative fixtures. No fixture can publish a release verdict."""

from __future__ import annotations

import json
import subprocess
from copy import deepcopy
from pathlib import Path

import pytest
import yaml

from tools.release import finding_proof, receipt, semver

REPO = Path(__file__).resolve().parents[3]


@pytest.mark.parametrize(
    ("workflow_name", "job_name"),
    [("ci.yml", "gate"), ("release.yml", "release")],
)
def test_baseline_consumers_fetch_immutable_history(workflow_name: str, job_name: str) -> None:
    workflow = yaml.safe_load(
        (REPO / ".github" / "workflows" / workflow_name).read_text(encoding="utf-8")
    )
    checkout = next(
        step
        for step in workflow["jobs"][job_name]["steps"]
        if step.get("uses", "").startswith("actions/checkout@")
    )
    assert checkout.get("with", {}).get("fetch-depth") == 0


def test_baseline_is_the_immutable_020_release_commit() -> None:
    policy = json.loads(semver.POLICY.read_text())
    assert semver.policy_problems(policy) == []
    baseline = policy["baseline_sha"]
    result = subprocess.run(
        ["git", "show", "-s", "--format=%H %s", baseline],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=True,
    )
    assert result.stdout.startswith(baseline + " release: 0.2.0")
    assert policy["candidate_version"] is None
    assert (
        semver.workspace_version(
            subprocess.run(
                ["git", "show", f"{baseline}:Cargo.toml"],
                cwd=REPO,
                capture_output=True,
                text=True,
                check=True,
            ).stdout
        )
        == "0.2.0"
    )


@pytest.mark.parametrize(
    ("field", "bad"),
    [
        ("baseline_sha", "main"),
        ("baseline_sha", "0" * 39),
        ("public_crates", ["taskmesh"]),
        ("features", ["--all-features"]),
        ("release_type", "major"),
        ("semver_tool", "cargo-semver-checks latest"),
    ],
)
def test_semver_policy_rejects_movable_or_incomplete_input(field: str, bad: object) -> None:
    policy = json.loads(semver.POLICY.read_text())
    policy[field] = bad
    assert semver.policy_problems(policy)


def test_semver_commands_pin_all_four_crates_and_baseline() -> None:
    policy = json.loads(semver.POLICY.read_text())
    commands = [semver.command(crate, policy["baseline_sha"]) for crate in policy["public_crates"]]
    assert len(commands) == 4
    assert all("--default-features" in argv for argv in commands)
    assert all(argv[-3:] == ["minor", "--color", "never"] for argv in commands)
    assert all(policy["baseline_sha"] in argv for argv in commands)


@pytest.mark.parametrize("name", ["../secret", "/tmp/file", "target/../../secret", "", [], {}])
def test_artifact_reference_must_be_repo_relative_and_safe(tmp_path: Path, name: object) -> None:
    assert receipt.safe_artifact(tmp_path, name) is None


def test_artifact_identity_rejects_symlink_escape(tmp_path: Path) -> None:
    (tmp_path / "target").mkdir()
    outside = tmp_path.parent / "outside-release-proof.json"
    outside.write_text("{}")
    (tmp_path / "target" / "proof.json").symlink_to(outside)
    assert receipt.identity(tmp_path, "target/proof.json") is None


def test_coverage_does_not_turn_uncollected_branches_into_zero_percent() -> None:
    raw = {
        "data": [
            {
                "totals": {
                    name: {"count": 0 if name in ("branches", "mcdc") else 10, "percent": 0.0}
                    for name in receipt.METRICS
                }
            }
        ]
    }
    reasons: list[str] = []
    report = receipt.metric_report(raw, reasons)
    assert reasons == []
    assert report["branches"] == {"status": "NOT_COLLECTED", "count": 0, "percent": None}
    assert report["mcdc"] == {"status": "NOT_COLLECTED", "count": 0, "percent": None}
    assert report["instantiations"] == {"status": "COLLECTED", "count": 10, "percent": 0.0}


def test_coverage_missing_raw_counts_fail_closed() -> None:
    reasons: list[str] = []
    report = receipt.metric_report({"data": [{"totals": {}}]}, reasons)
    assert report == {}
    assert len(reasons) == 6


def test_adjudication_requires_reviewer_source_and_every_blind_spot() -> None:
    value = json.loads(receipt.ADJUDICATION.read_text())
    reasons = receipt.adjudication_problems(REPO, value, "a" * 40, value["baseline_sha"])
    assert any("candidate SHA" in reason for reason in reasons)
    assert any("reviewer" in reason for reason in reasons)
    assert len([reason for reason in reasons if "decision pending" in reason]) == len(
        receipt.ADJUDICATION_IDS
    )
    bad = deepcopy(value)
    bad["items"][0]["id"] = []
    assert any(
        "item set" in r
        for r in receipt.adjudication_problems(REPO, bad, "a" * 40, bad["baseline_sha"])
    )


def test_semver_manifest_cannot_launder_missing_crate_or_raw_output() -> None:
    policy = json.loads(semver.POLICY.read_text())
    source = {"head": "a" * 40, "tree": "b" * 40, "paths_digest": "c" * 64, "dirty": False}
    value = {"status": "PASS", "results": []}
    reasons = receipt.semver_problems(REPO, value, source, policy)
    assert "semver does not contain exactly four crate results" in reasons


def semver_fixture(tmp_path: Path) -> tuple[dict, dict, dict]:
    (tmp_path / "tools/release").mkdir(parents=True)
    (tmp_path / "tools/release/release-policy.json").write_bytes(semver.POLICY.read_bytes())
    (tmp_path / "CHANGELOG.md").write_text("approved break anchor\n")
    (tmp_path / "evidence.txt").write_text("regression and negative evidence\n")
    policy = json.loads(semver.POLICY.read_text())
    source = {"head": "a" * 40, "tree": "b" * 40, "paths_digest": "c" * 64, "dirty": False}
    results = []
    for crate in policy["public_crates"]:
        base = tmp_path / crate
        stdout = base.with_suffix(".stdout")
        stderr = base.with_suffix(".stderr")
        stdout.write_text(
            "deny-level lint found on public API\n" if crate == "taskmesh" else "clean\n"
        )
        stderr.write_text("")
        results.append(
            {
                "crate": crate,
                "command": semver.command(crate, policy["baseline_sha"]),
                "status": "FINDINGS" if crate == "taskmesh" else "CLEAN",
                "exit_code": 100 if crate == "taskmesh" else 0,
                "timed_out": False,
                "stdout": receipt.identity(tmp_path, stdout.name),
                "stderr": receipt.identity(tmp_path, stderr.name),
            }
        )
    manifest = {
        "schema_version": 1,
        "producer": "taskmesh-semver-release-v1",
        "status": "REPORTED",
        "source": source,
        "source_after": {"paths_digest": source["paths_digest"], "dirty": False},
        "baseline_sha": policy["baseline_sha"],
        "baseline_version": policy["baseline_version"],
        "policy_sha256": semver.sha256(tmp_path / "tools/release/release-policy.json"),
        "tool": policy["semver_tool"],
        "features": policy["features"],
        "release_type": policy["release_type"],
        "candidate_version": policy["candidate_version"],
        "results": results,
    }
    return policy, source, manifest


def test_semver_exit_100_is_reported_findings_but_101_is_tool_failure(tmp_path: Path) -> None:
    policy, source, manifest = semver_fixture(tmp_path)
    assert receipt.semver_problems(tmp_path, manifest, source, policy) == []
    malformed = deepcopy(manifest)
    malformed["results"][0]["exit_code"] = 101
    malformed["results"][0]["status"] = "TOOL_FAILURE"
    assert any(
        "did not finish" in reason
        for reason in receipt.semver_problems(tmp_path, malformed, source, policy)
    )
    malformed = deepcopy(manifest)
    malformed["results"][0]["stdout"]["sha256"] = "0" * 64
    assert any(
        "stdout raw artifact" in reason
        for reason in receipt.semver_problems(tmp_path, malformed, source, policy)
    )


def test_human_adjudication_maps_every_exit_100_to_raw_and_changelog(tmp_path: Path) -> None:
    policy, source, manifest = semver_fixture(tmp_path)
    value = json.loads(receipt.ADJUDICATION.read_text())
    value.update(
        {
            "source_sha": source["head"],
            "reviewer": "release-reviewer",
            "decision": "APPROVED",
            "all_tool_findings_reviewed": True,
        }
    )
    for item in value["items"]:
        item.update(
            {
                "decision": "APPROVED_BREAK",
                "evidence": "evidence.txt",
                "changelog_anchor": "approved break anchor",
            }
        )
    value["semver_outputs"] = [
        {
            "crate": result["crate"],
            "stdout_sha256": result["stdout"]["sha256"],
            "stderr_sha256": result["stderr"]["sha256"],
            "all_findings_reviewed": True,
            "findings": (
                [
                    {
                        "raw_locator": "deny-level lint found on public API",
                        "occurrence": 1,
                        "adjudication_item_id": "facade-reexports",
                    }
                ]
                if result["status"] == "FINDINGS"
                else []
            ),
        }
        for result in manifest["results"]
    ]
    assert (
        receipt.adjudication_problems(
            tmp_path, value, source["head"], policy["baseline_sha"], manifest
        )
        == []
    )
    for mutation, expected in (
        (lambda x: x["semver_outputs"][2].update(findings=[]), "unmapped"),
        (lambda x: x["semver_outputs"][2].update(stdout_sha256="0" * 64), "raw-bound"),
        (
            lambda x: x["semver_outputs"][2]["findings"][0].update(
                raw_locator="missing raw line 123456"
            ),
            "locator",
        ),
        (lambda x: x["items"][0].update(changelog_anchor=None), "CHANGELOG"),
        (lambda x: x.update(reviewer=None), "reviewer"),
    ):
        bad = deepcopy(value)
        mutation(bad)
        assert any(
            expected in reason
            for reason in receipt.adjudication_problems(
                tmp_path, bad, source["head"], policy["baseline_sha"], manifest
            )
        ), expected


def test_23_findings_link_once_and_unverified_tickets_stay_open() -> None:
    plan = json.loads(receipt.PLAN.read_text())
    reasons: list[str] = []
    graph = receipt.closure_report(REPO, plan, reasons)
    assert len(graph) == 12
    assert sorted(f for entry in graph for f in entry["findings"]) == [
        f"TM21-{n:03d}" for n in range(1, 24)
    ]
    assert any("SEP21-V02" in reason and "not verified" in reason for reason in reasons)
    bad = deepcopy(plan)
    bad["tickets"][0]["findings"] = [[]]
    reasons = []
    receipt.closure_report(REPO, bad, reasons)
    assert reasons


def test_missing_release_evidence_is_never_qualified() -> None:
    verdict = receipt.evaluate({"schema_version": 1, "source": {"dirty": False}}, current=None)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("ordinary" in reason for reason in verdict["reasons"])
    assert any("semver" in reason for reason in verdict["reasons"])


@pytest.mark.parametrize("bad", [None, [], 0, "PASS", {"verdict": {"status": "QUALIFIED"}}])
def test_malformed_release_receipt_never_qualifies(bad: object) -> None:
    assert receipt.evaluate(bad, current=None)["status"] == "NOT_QUALIFIED"


def test_finding_spec_maps_actual_exact_tests_and_all_23_ticket_findings() -> None:
    spec = json.loads(finding_proof.SPEC.read_text())
    plan = json.loads(finding_proof.PLAN.read_text())
    ordinary = json.loads((REPO / "tools/gates/required.json").read_text())
    assert finding_proof.spec_problems(REPO, spec, plan, set(ordinary["required"])) == []
    assert [row["id"] for row in spec["witnesses"]] == finding_proof.IDS
    for change, expected in (
        (lambda x: x["witnesses"][0].update(ticket="SEP21-R01"), "ticket mapping"),
        (lambda x: x["witnesses"][0].update(test="not_a_real_test"), "does not exist"),
        (lambda x: x["witnesses"][0].update(required_gate="not-a-gate"), "unknown"),
        (lambda x: x["witnesses"].pop(), "exactly 23"),
    ):
        bad = deepcopy(spec)
        change(bad)
        assert any(
            expected in reason
            for reason in finding_proof.spec_problems(REPO, bad, plan, set(ordinary["required"]))
        )


def finding_fixture(tmp_path: Path) -> tuple[dict, dict, dict, dict]:
    spec = json.loads(finding_proof.SPEC.read_text())
    plan = json.loads(finding_proof.PLAN.read_text())
    required = json.loads((REPO / "tools/gates/required.json").read_text())
    for name, original in (
        ("tools/release/finding-proof-spec.json", finding_proof.SPEC),
        ("docs/bugbash/sep-21/tickets/plan.json", finding_proof.PLAN),
        ("tools/gates/required.json", REPO / "tools/gates/required.json"),
    ):
        path = tmp_path / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(original.read_bytes())
    source = {"head": "a" * 40, "tree": "b" * 40, "paths_digest": "c" * 64, "dirty": False}
    rows = []
    for witness in spec["witnesses"]:
        path = tmp_path / witness["path"]
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes((REPO / witness["path"]).read_bytes())
        proof_root = tmp_path / "target/release/finding-proof"
        proof_root.mkdir(parents=True, exist_ok=True)
        stdout = proof_root / f"{witness['id']}.stdout.log"
        stderr = proof_root / f"{witness['id']}.stderr.log"
        if witness["kind"] == "rust":
            stdout.write_text(
                f"running 1 test\ntest {witness.get('selector', witness['test'])} ... ok\n"
                "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
            )
        else:
            stdout.write_text("1 passed in 0.01s\n")
        stderr.write_text("")
        rows.append(
            {
                **witness,
                "command": finding_proof.test_command(witness),
                "test_source": receipt.identity(tmp_path, witness["path"]),
                "exit_code": 0,
                "timed_out": False,
                "selected": 1,
                "executed": 1,
                "status": "PASS",
                "stdout": receipt.identity(tmp_path, str(stdout.relative_to(tmp_path))),
                "stderr": receipt.identity(tmp_path, str(stderr.relative_to(tmp_path))),
            }
        )
    manifest = {
        "schema_version": 1,
        "producer": "taskmesh-finding-proof-v1",
        "status": "PASS",
        "source": source,
        "source_after": {"paths_digest": source["paths_digest"], "dirty": False},
        "spec": receipt.identity(tmp_path, "tools/release/finding-proof-spec.json"),
        "plan": receipt.identity(tmp_path, "docs/bugbash/sep-21/tickets/plan.json"),
        "results": rows,
    }
    ordinary = {
        "gates": {"results": [{"id": gate, "status": "PASS"} for gate in required["required"]]}
    }
    return manifest, spec, plan, ordinary


def test_finding_manifest_binds_all_23_nonvacuous_raw_witnesses(tmp_path: Path) -> None:
    manifest, spec, plan, ordinary = finding_fixture(tmp_path)
    source = manifest["source"]
    assert receipt.finding_proof_problems(tmp_path, manifest, spec, plan, ordinary, source) == []
    for mutation, expected in (
        (lambda x: x["results"][0].update(executed=0), "exactly once"),
        (lambda x: x["results"][0].update(status="PASS", exit_code=None), "exactly once"),
        (lambda x: x["results"][0]["stdout"].update(sha256="0" * 64), "raw digest"),
        (lambda x: x["results"][0].update(test_source=None), "test source digest"),
        (lambda x: x["results"][0].update(command=["true"]), "command differs"),
        (lambda x: x["source"].update(head="d" * 40), "source differs"),
        (lambda x: x["results"].pop(), "exactly 23"),
    ):
        bad = deepcopy(manifest)
        mutation(bad)
        assert any(
            expected in reason
            for reason in receipt.finding_proof_problems(
                tmp_path, bad, spec, plan, ordinary, source
            )
        ), expected
    raw = tmp_path / "target/release/finding-proof/TM21-001.stdout.log"
    raw.write_text("running 0 tests\ntest result: ok. 0 passed; 0 failed\n")
    assert any(
        "raw digest" in reason and "TM21-001" in reason
        for reason in receipt.finding_proof_problems(
            tmp_path, manifest, spec, plan, ordinary, source
        )
    )
    laundered = deepcopy(manifest)
    laundered["results"][0]["stdout"] = receipt.identity(
        tmp_path, "target/release/finding-proof/TM21-001.stdout.log"
    )
    assert any(
        "raw output has zero or wrong test count" in reason and "TM21-001" in reason
        for reason in receipt.finding_proof_problems(
            tmp_path, laundered, spec, plan, ordinary, source
        )
    )


@pytest.mark.parametrize(
    ("field", "replacement", "expected"),
    [
        ("attestation", None, "hosted main workflow_dispatch"),
        ("semver_manifest", None, "semver manifest"),
        ("adjudication", None, "human adjudication"),
        ("finding_manifest", None, "finding proof manifest"),
        ("raw_artifacts", {}, "release raw artifact set incomplete"),
    ],
)
def test_full_release_validator_rejects_missing_authorities(
    field: str, replacement: object, expected: str
) -> None:
    value = {"schema_version": 1, "source": {"dirty": False}, field: replacement}
    verdict = receipt.evaluate(value, current=None)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any(expected in reason for reason in verdict["reasons"])


def test_full_release_validator_rejects_historical_wrong_sha_receipt(tmp_path: Path) -> None:
    historical = tmp_path / "receipt.json"
    historical.write_text(json.dumps({"source": {"head": "c" * 40}}))
    source = {"head": "a" * 40, "tree": "b" * 40, "paths_digest": "d" * 64, "dirty": False}
    value = {
        "schema_version": 1,
        "source": source,
        "ordinary_receipt": receipt.identity(tmp_path, "receipt.json"),
    }
    verdict = receipt.evaluate(value, root=tmp_path, current=source)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("ordinary receipt is not the exact release source" in r for r in verdict["reasons"])


@pytest.mark.parametrize("version", ["garbage", "0.3", "0.x.0", "0.3.0-rc1", "1e1.0.0"])
def test_full_release_validator_rejects_malformed_candidate_version_without_throwing(
    tmp_path: Path,
    version: str,
) -> None:
    policy = json.loads(receipt.POLICY.read_text())
    policy["candidate_version"] = version
    disk = tmp_path / "tools/release/release-policy.json"
    disk.parent.mkdir(parents=True)
    disk.write_text(json.dumps(policy))
    (tmp_path / "Cargo.toml").write_text(f'[workspace.package]\nversion = "{version}"\n')
    value = {
        "schema_version": 1,
        "source": {"dirty": False},
        "policy": receipt.identity(tmp_path, "tools/release/release-policy.json"),
    }
    verdict = receipt.evaluate(value, root=tmp_path, current=None)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("candidate version" in reason for reason in verdict["reasons"])


@pytest.mark.parametrize("failed_check", ["workflow_dispatch", "main_ref", "source_clean"])
def test_full_release_validator_rejects_pr_local_or_dirty_attestation(
    failed_check: str,
) -> None:
    source = {"head": "a" * 40, "tree": "b" * 40, "paths_digest": "c" * 64, "dirty": False}
    checks = {name: True for name in receipt.release_attestation(source)["checks"]}
    checks[failed_check] = False
    value = {
        "schema_version": 1,
        "source": source,
        "attestation": {
            "kind": "github-actions-release",
            "eligible": True,
            "actor": "reviewer",
            "checks": checks,
        },
    }
    verdict = receipt.evaluate(value, current=source)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("hosted main workflow_dispatch" in r for r in verdict["reasons"])


def test_full_release_validator_cannot_substitute_curated_for_generated(tmp_path: Path) -> None:
    source = {"head": "a" * 40, "tree": "b" * 40, "paths_digest": "c" * 64, "dirty": False}
    ordinary_path = tmp_path / "receipt.json"
    ordinary_path.write_text(
        json.dumps(
            {
                "source": source,
                "mutations": {"status": "PASS"},
                "generated_mutation_sweep": {"status": "REPORTED"},
            }
        )
    )
    value = {
        "schema_version": 1,
        "source": source,
        "ordinary_receipt": receipt.identity(tmp_path, "receipt.json"),
    }
    verdict = receipt.evaluate(value, root=tmp_path, current=source)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("generated mutation is not quality PASS" in r for r in verdict["reasons"])


def test_full_release_validator_rejects_approved_break_without_changelog(
    tmp_path: Path,
) -> None:
    template = json.loads(receipt.ADJUDICATION.read_text())
    approved = deepcopy(template)
    approved.update(
        source_sha="a" * 40,
        reviewer="release-reviewer",
        decision="APPROVED",
        all_tool_findings_reviewed=True,
    )
    for item in approved["items"]:
        item["decision"] = "APPROVED_BREAK"
        item["changelog_anchor"] = "missing exact migration anchor"
    template_path = tmp_path / "tools/release/adjudication.json"
    approval_path = tmp_path / "target/release/input/adjudication.json"
    template_path.parent.mkdir(parents=True)
    approval_path.parent.mkdir(parents=True)
    template_path.write_text(json.dumps(template))
    approval_path.write_text(json.dumps(approved))
    (tmp_path / "CHANGELOG.md").write_text("# Unreleased\n")
    source = {"head": "a" * 40, "tree": "b" * 40, "paths_digest": "c" * 64, "dirty": False}
    value = {
        "schema_version": 1,
        "source": source,
        "adjudication_template": receipt.identity(tmp_path, "tools/release/adjudication.json"),
        "adjudication": receipt.identity(tmp_path, "target/release/input/adjudication.json"),
    }
    verdict = receipt.evaluate(value, root=tmp_path, current=source)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("lacks a concrete CHANGELOG anchor" in r for r in verdict["reasons"])

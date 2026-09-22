from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[3]
MODULE = REPO / "tools" / "qualification" / "evidence.py"
EXAMPLES = MODULE.parent / "examples"
_spec = importlib.util.spec_from_file_location("evidence", MODULE)
assert _spec and _spec.loader
evidence = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(evidence)


def load(name: str) -> dict:
    return json.loads((EXAMPLES / name).read_text(encoding="utf-8"))


def test_v02_and_v03_examples_satisfy_the_shared_contract() -> None:
    for name in ("v02-mutation-valid.json", "v03-fuzz-valid.json"):
        assert evidence.envelope_problems(load(name)) == [], name


def test_missing_raw_artifact_example_is_invalid() -> None:
    problems = evidence.envelope_problems(load("invalid-missing-raw-artifact.json"))
    assert problems == ["artifacts must contain at least one raw artifact"]


def test_canonical_digest_is_order_independent_and_rejects_nan() -> None:
    assert evidence.canonical_digest({"b": 1, "a": 2}) == evidence.canonical_digest(
        {"a": 2, "b": 1}
    )
    with pytest.raises(ValueError):
        evidence.canonical_digest({"bad": float("nan")})


def test_command_digest_is_recomputed_not_trusted() -> None:
    envelope = load("v02-mutation-valid.json")
    envelope["command"]["argv"].append("--forged")
    assert "command.sha256 does not match argv/cwd/environment" in evidence.envelope_problems(
        envelope
    )


@pytest.mark.parametrize(
    "missing", ["source", "command", "tools", "configs", "action", "artifacts", "result"]
)
def test_missing_required_identity_fails_closed(missing: str) -> None:
    envelope = load("v02-mutation-valid.json")
    envelope.pop(missing)
    assert evidence.envelope_problems(envelope), missing


@pytest.mark.parametrize("status", ["FAIL", "NOT_RUN", "SKIPPED", "TIMEOUT", "BASELINE_CREATED"])
def test_required_non_pass_status_never_qualifies(tmp_path: Path, status: str) -> None:
    envelope = materialized_envelope(tmp_path)
    envelope["result"]["status"] = status
    problems = evidence.qualification_problems(envelope, artifact_root=tmp_path)
    assert any("required producer status" in problem for problem in problems)


def materialized_envelope(root: Path) -> dict:
    envelope = copy.deepcopy(load("v02-mutation-valid.json"))
    raw = root / "raw" / "mutations.json"
    raw.parent.mkdir(parents=True)
    raw.write_text("raw mutation output\n", encoding="utf-8")
    summary = root / "receipt.mutations.json"
    summary.write_text("summary\n", encoding="utf-8")
    for config in envelope["configs"]:
        path = root / config["path"]
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("config\n", encoding="utf-8")
        config["sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
    workflow = root / envelope["action"]["workflow"]
    workflow.parent.mkdir(parents=True, exist_ok=True)
    workflow.write_text("name: fixture\n", encoding="utf-8")
    envelope["action"]["workflow_sha256"] = hashlib.sha256(workflow.read_bytes()).hexdigest()
    for artifact in envelope["artifacts"]:
        path = root / artifact["path"]
        data = path.read_bytes()
        artifact["size"] = len(data)
        artifact["sha256"] = hashlib.sha256(data).hexdigest()
    return envelope


def test_artifact_size_and_digest_are_verified_from_disk(tmp_path: Path) -> None:
    envelope = materialized_envelope(tmp_path)
    assert evidence.qualification_problems(envelope, artifact_root=tmp_path) == []
    (tmp_path / "raw" / "mutations.json").write_text("forged\n", encoding="utf-8")
    problems = evidence.qualification_problems(envelope, artifact_root=tmp_path)
    assert any("size mismatch" in problem for problem in problems)
    assert any("digest mismatch" in problem for problem in problems)


@pytest.mark.parametrize("path", ["/tmp/outside.log", "../outside.log"])
def test_unsafe_artifact_path_is_rejected_without_dereference(tmp_path: Path, path: str) -> None:
    envelope = materialized_envelope(tmp_path)
    envelope["artifacts"][0]["path"] = path
    problems = evidence.envelope_problems(envelope, artifact_root=tmp_path)
    assert any("safe relative path" in problem for problem in problems)


@pytest.mark.parametrize("field", ["config", "workflow", "artifact"])
def test_evidence_files_cannot_escape_through_symlinks(tmp_path: Path, field: str) -> None:
    envelope = materialized_envelope(tmp_path)
    outside = tmp_path.parent / f"outside-{field}.txt"
    outside.write_text("outside\n", encoding="utf-8")
    link = tmp_path / f"escape-{field}"
    link.symlink_to(outside)
    if field == "config":
        envelope["configs"][0]["path"] = link.name
    elif field == "workflow":
        envelope["action"]["workflow"] = link.name
    else:
        envelope["artifacts"][0]["path"] = link.name
    problems = evidence.envelope_problems(envelope, artifact_root=tmp_path)
    assert any("must not be a symlink" in problem for problem in problems)


@pytest.mark.parametrize(("selected", "executed"), [(0, 0), (2, 1), (1, 2)])
def test_pass_requires_positive_selected_executed_parity(selected: int, executed: int) -> None:
    envelope = load("v02-mutation-valid.json")
    envelope["result"].update(selected_count=selected, executed_count=executed)
    problems = evidence.envelope_problems(envelope)
    assert any(problem.startswith("PASS requires") for problem in problems)


def test_source_and_action_identity_mismatch_are_rejected() -> None:
    envelope = load("v02-mutation-valid.json")
    expected = dict(envelope["source"])
    envelope["source"]["tree_digest"] = "9" * 64
    envelope["action"]["actions"] = ["actions/checkout@v4"]
    problems = evidence.envelope_problems(envelope, expected_source=expected)
    assert "source.tree_digest does not match the expected source" in problems
    assert any("not pinned to a full commit SHA" in problem for problem in problems)


def test_hosted_action_ref_and_source_sha_are_bound() -> None:
    envelope = load("v02-mutation-valid.json")
    envelope["action"]["workflow_ref"] = (
        "taskmesh/taskmesh/.github/workflows/other.yml@refs/heads/main"
    )
    envelope["action"]["source_sha"] = "9" * 40
    problems = evidence.envelope_problems(envelope)
    assert "hosted action.workflow_ref does not match action.workflow" in problems
    assert "action.source_sha does not match source.head" in problems


def test_runtime_action_reads_the_hosted_environment(tmp_path: Path, monkeypatch) -> None:
    workflow = tmp_path / ".github" / "workflows" / "ci.yml"
    workflow.parent.mkdir(parents=True)
    workflow.write_text("name: ci\n", encoding="utf-8")
    head = "1" * 40
    monkeypatch.setenv("GITHUB_ACTIONS", "true")
    monkeypatch.setenv(
        "GITHUB_WORKFLOW_REF", "taskmesh/taskmesh/.github/workflows/ci.yml@refs/heads/main"
    )
    monkeypatch.setenv("GITHUB_JOB", "qualification")
    monkeypatch.setenv("GITHUB_EVENT_NAME", "push")
    monkeypatch.setenv("GITHUB_SHA", head)
    action = evidence.runtime_action(
        root=tmp_path,
        source_head=head,
        local_workflow="ignored.json",
        local_job="ignored",
    )
    assert action["context"] == "github-actions"
    assert action["workflow"] == ".github/workflows/ci.yml"
    assert action["job"] == "qualification"
    assert action["source_sha"] == head


def test_runtime_action_rejects_missing_or_wrong_hosted_identity(
    tmp_path: Path, monkeypatch
) -> None:
    workflow = tmp_path / ".github" / "workflows" / "ci.yml"
    workflow.parent.mkdir(parents=True)
    workflow.write_text("name: ci\n", encoding="utf-8")
    monkeypatch.setenv("GITHUB_ACTIONS", "true")
    monkeypatch.setenv(
        "GITHUB_WORKFLOW_REF", "taskmesh/taskmesh/.github/workflows/ci.yml@refs/heads/main"
    )
    monkeypatch.setenv("GITHUB_JOB", "qualification")
    monkeypatch.setenv("GITHUB_EVENT_NAME", "push")
    monkeypatch.delenv("GITHUB_SHA", raising=False)
    with pytest.raises(RuntimeError, match="missing"):
        evidence.runtime_action(
            root=tmp_path,
            source_head="1" * 40,
            local_workflow="ignored.json",
            local_job="ignored",
        )
    monkeypatch.setenv("GITHUB_SHA", "2" * 40)
    with pytest.raises(RuntimeError, match="does not match"):
        evidence.runtime_action(
            root=tmp_path,
            source_head="1" * 40,
            local_workflow="ignored.json",
            local_job="ignored",
        )


@pytest.mark.parametrize(
    ("started", "finished", "reason"),
    [
        ("not-a-time", "2026-09-21T00:01:00Z", "RFC3339 UTC"),
        ("2026-09-21T00:00:00+09:00", "2026-09-21T00:01:00Z", "RFC3339 UTC"),
        (
            "2026-09-21T00:02:00Z",
            "2026-09-21T00:01:00Z",
            "must not precede",
        ),
    ],
)
def test_result_interval_requires_ordered_rfc3339_utc(
    started: str, finished: str, reason: str
) -> None:
    envelope = load("v02-mutation-valid.json")
    envelope["result"].update(started_at=started, finished_at=finished)
    assert any(reason in problem for problem in evidence.envelope_problems(envelope))


@pytest.mark.parametrize("version", ["stable", "latest", "nightly", "custom-build"])
def test_mutable_or_ambiguous_tool_version_is_rejected(version: str) -> None:
    envelope = load("v02-mutation-valid.json")
    envelope["tools"][0]["version"] = version
    assert any("mutable or ambiguous" in p for p in evidence.envelope_problems(envelope))


def test_tool_identity_digest_is_required() -> None:
    envelope = load("v02-mutation-valid.json")
    envelope["tools"][0].pop("identity_sha256")
    assert "tools[0].identity_sha256 is required" in evidence.envelope_problems(envelope)


def test_tool_identity_digest_is_recomputed_from_exact_version() -> None:
    envelope = load("v02-mutation-valid.json")
    envelope["tools"][0]["version"] = "cargo 1.96.0 (other-build)"
    assert any("does not match name/version" in p for p in evidence.envelope_problems(envelope))


@pytest.mark.parametrize("field", ["config", "workflow"])
def test_config_and_workflow_paths_are_safe_relative(field: str) -> None:
    envelope = load("v02-mutation-valid.json")
    if field == "config":
        envelope["configs"][0]["path"] = "../outside.json"
    else:
        envelope["action"]["workflow"] = "/tmp/workflow.yml"
    assert any("safe relative path" in p for p in evidence.envelope_problems(envelope))


@pytest.mark.parametrize("field", ["config", "workflow"])
def test_config_and_workflow_digest_are_rehashed_from_disk(tmp_path: Path, field: str) -> None:
    envelope = materialized_envelope(tmp_path)
    if field == "config":
        path = tmp_path / envelope["configs"][0]["path"]
    else:
        path = tmp_path / envelope["action"]["workflow"]
    path.write_text("forged\n", encoding="utf-8")
    assert any(
        field in problem and "digest mismatch" in problem
        for problem in evidence.envelope_problems(envelope, artifact_root=tmp_path)
    )


def test_schema_file_has_a_stable_canonical_digest() -> None:
    schema = json.loads(evidence.SCHEMA.read_text(encoding="utf-8"))
    assert schema["title"] == "EvidenceEnvelopeV1"
    assert "identity_sha256" in schema["properties"]["tools"]["items"]["required"]
    assert {"context", "workflow_ref", "source_sha"} <= set(
        schema["properties"]["action"]["required"]
    )
    assert schema["properties"]["configs"]["items"]["properties"]["path"]["$ref"].endswith(
        "safe_path"
    )
    assert schema["properties"]["result"]["properties"]["started_at"]["$ref"].endswith(
        "utc_timestamp"
    )
    assert len(evidence.canonical_digest(schema)) == 64

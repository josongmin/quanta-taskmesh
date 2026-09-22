#!/usr/bin/env python3
"""Run the curated mutation inventory in an isolated current-source snapshot."""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from collections import Counter
from dataclasses import asdict, dataclass, field
from pathlib import Path, PurePosixPath

sys.path.insert(0, str(Path(__file__).resolve().parent))

from campaign import (  # noqa: E402
    IsolatedCampaign,
    ProcessResult,
    canonical_digest,
    create_isolated_campaign,
    evidence_envelope,
    exact_tool_version,
    execute,
    prepare_output_dir,
    sanitized_campaign_environment,
    sha256_file,
    utc_now,
    write_json,
)

REPO = Path(__file__).resolve().parents[2]
INVENTORY = Path(__file__).resolve().parent / "mutations.json"
TEST_TIMEOUT_SECONDS = 900
GOOD_STATUSES = {"KILLED", "CONTROL_GREEN"}
RUNNERS = {"cargo", "pytest"}
PROFILES = {"debug", "release"}
RUNNER_OWNED_ENV = {
    "CARGO_BUILD_TARGET_DIR",
    "CARGO_TARGET_DIR",
    "OLDPWD",
    "PWD",
    "PYTHONDONTWRITEBYTECODE",
}


@dataclass
class Outcome:
    mutation_id: str
    finding: str
    status: str
    detail: str
    duration_s: float
    expected_failures: list[str] = field(default_factory=list)
    observed_failures: list[str] = field(default_factory=list)
    exit_code: int | None = None
    signal: int | None = None
    timed_out: bool = False
    completed: bool = False
    test_count: int = 0
    command: list[str] = field(default_factory=list)
    command_sha256: str = ""
    environment: dict[str, str] = field(default_factory=dict)
    profile: str = "debug"
    baseline_sha256: str = ""
    stdout_sha256: str = ""
    stderr_sha256: str = ""


@dataclass
class Baseline:
    identity_sha256: str
    definition_sha256: str
    source_head: str
    source_snapshot_sha256: str
    tools_sha256: str
    status: str
    detail: str
    command: list[str]
    command_sha256: str
    environment: dict[str, str]
    profile: str
    exit_code: int | None
    signal: int | None
    timed_out: bool
    completed: bool
    test_count: int
    observed_failures: list[str]
    started_at: str
    finished_at: str
    duration_s: float
    stdout_artifact: str
    stderr_artifact: str
    stdout_sha256: str
    stderr_sha256: str


PYTEST_COUNT = re.compile(r"\b(\d+) (passed|failed|errors?|skipped|xfailed|xpassed)\b")
PYTEST_COMPLETE = re.compile(r"\b\d+ (?:passed|failed|errors?|skipped|xfailed|xpassed)\b.*\bin ")
CARGO_COMPLETE = re.compile(r"^test result: (?:ok|FAILED)\.", re.MULTILINE)
EXACT_TEST_NAME = re.compile(r"[A-Za-z0-9_]+(?:::[A-Za-z0-9_]+)*")


def normalized_repo_path(value: object) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError("file must be a non-empty string")
    if "\\" in value or "\0" in value:
        raise ValueError("file must use normalized repository-relative POSIX syntax")
    path = PurePosixPath(value)
    if path.is_absolute() or ".." in path.parts or "." in path.parts or str(path) != value:
        raise ValueError("file must be a normalized repository-relative path")
    return value


def confined_source_file(source: Path, value: object, *, mutation_id: str) -> Path:
    relative = normalized_repo_path(value)
    target = source / relative
    try:
        resolved = target.resolve(strict=True)
        resolved.relative_to(source.resolve(strict=True))
    except (FileNotFoundError, ValueError) as error:
        raise RuntimeError(
            f"mutation {mutation_id}: file is missing or escapes the isolated source: {relative}"
        ) from error
    if target.is_symlink():
        raise RuntimeError(f"mutation {mutation_id}: refusing symlink target {relative}")
    if not target.is_file():
        raise RuntimeError(f"mutation {mutation_id}: target is not a regular file: {relative}")
    return target


def validate_entry(mutation: dict) -> None:
    mutation_id = mutation.get("id") or "<missing id>"
    for key in ("id", "finding", "file", "find", "replace", "test_target"):
        if not mutation.get(key):
            raise SystemExit(f"mutation {mutation_id}: missing required field {key!r}")
    if mutation["find"] == mutation["replace"]:
        raise SystemExit(f"mutation {mutation_id}: find and replace are identical")
    try:
        normalized_repo_path(mutation["file"])
    except ValueError as error:
        raise SystemExit(f"mutation {mutation_id}: {error}") from error
    runner = mutation.get("runner", "cargo")
    if runner not in RUNNERS:
        raise SystemExit(f"mutation {mutation_id}: unknown runner {runner!r}")
    if runner == "cargo" and not mutation.get("package"):
        raise SystemExit(f"mutation {mutation_id}: cargo runner needs a package")
    if mutation.get("profile", "debug") not in PROFILES:
        raise SystemExit(f"mutation {mutation_id}: unknown profile {mutation.get('profile')!r}")
    env = mutation.get("env", {})
    if not isinstance(env, dict) or not all(
        isinstance(key, str) and isinstance(value, str) for key, value in env.items()
    ):
        raise SystemExit(f"mutation {mutation_id}: env must be a string-to-string map")
    forbidden_env = sorted(set(env) & RUNNER_OWNED_ENV)
    if forbidden_env:
        raise SystemExit(
            f"mutation {mutation_id}: env cannot override runner-owned isolation keys: "
            f"{forbidden_env}"
        )
    cargo_args = mutation.get("cargo_args", [])
    if not isinstance(cargo_args, list) or not all(isinstance(value, str) for value in cargo_args):
        raise SystemExit(f"mutation {mutation_id}: cargo_args must be a list of strings")
    if runner == "pytest" and (env or cargo_args):
        raise SystemExit(f"mutation {mutation_id}: env/cargo_args apply to the cargo runner only")
    cofailures = mutation.get("expect_cofailures", [])
    if not isinstance(cofailures, list) or not all(isinstance(value, str) for value in cofailures):
        raise SystemExit(f"mutation {mutation_id}: expect_cofailures must be a list of strings")
    control = bool(mutation.get("expect_no_failure"))
    if control:
        if mutation["finding"] != "control":
            raise SystemExit(f"mutation {mutation_id}: a control must be labelled finding=control")
        if cofailures:
            raise SystemExit(f"mutation {mutation_id}: a control cannot declare cofailures")
        return
    if mutation["finding"] == "control":
        raise SystemExit(f"mutation {mutation_id}: finding=control without expect_no_failure")
    if not mutation.get("expect_failing_test"):
        raise SystemExit(f"mutation {mutation_id}: missing expect_failing_test")
    if not mutation.get("expect_message"):
        raise SystemExit(f"mutation {mutation_id}: missing expect_message")
    if runner == "cargo":
        if EXACT_TEST_NAME.fullmatch(mutation["expect_failing_test"]) is None:
            raise SystemExit(
                f"mutation {mutation_id}: expect_failing_test must be an exact Rust test name"
            )
        if cofailures:
            raise SystemExit(
                f"mutation {mutation_id}: cargo mutations run one exact primary oracle and "
                "cannot declare cofailures"
            )


def load_inventory(path: Path) -> list[dict]:
    data = json.loads(path.read_text(encoding="utf-8"))
    mutations = data.get("mutations", [])
    if not mutations:
        raise SystemExit("mutation inventory is empty")
    seen: set[str] = set()
    for mutation in mutations:
        validate_entry(mutation)
        if mutation["id"] in seen:
            raise SystemExit(f"duplicate mutation id: {mutation['id']}")
        seen.add(mutation["id"])
    if not any(mutation.get("expect_no_failure") for mutation in mutations):
        raise SystemExit("mutation inventory has no control entry")
    return mutations


def test_command(mutation: dict) -> list[str]:
    if mutation.get("runner", "cargo") == "pytest":
        return [
            sys.executable,
            "-m",
            "pytest",
            "-q",
            "-p",
            "no:cacheprovider",
            "-rfE",
            mutation["test_target"],
        ]
    selector = (
        ["--lib"] if mutation["test_target"] == "lib" else ["--test", mutation["test_target"]]
    )
    profile = ["--release"] if mutation.get("profile") == "release" else []
    harness_args = (
        [mutation["expect_failing_test"], "--exact", "--test-threads", "1"]
        if not mutation.get("expect_no_failure")
        else ["--test-threads", "4"]
    )
    return [
        "cargo",
        "test",
        "-p",
        mutation["package"],
        *profile,
        *mutation.get("cargo_args", []),
        *selector,
        "--",
        *harness_args,
    ]


def command_environment(
    mutation: dict, campaign: IsolatedCampaign
) -> tuple[dict[str, str], dict[str, str]]:
    recorded = {
        **mutation.get("env", {}),
        "CARGO_TARGET_DIR": str(campaign.target),
        "PYTHONDONTWRITEBYTECODE": "1",
    }
    inherited = sanitized_campaign_environment(dict(os.environ))
    return {**inherited, **recorded}, recorded


def command_identity(mutation: dict) -> str:
    return canonical_digest(
        {
            "argv": test_command(mutation),
            "environment": mutation.get("env", {}),
            "profile": mutation.get("profile", "debug"),
            "runner": mutation.get("runner", "cargo"),
        }
    )


def failing_tests(stdout: str, runner: str = "cargo") -> list[str]:
    names: list[str] = []
    for line in stdout.splitlines():
        stripped = line.strip()
        if runner == "pytest":
            for prefix in ("FAILED ", "ERROR "):
                if stripped.startswith(prefix):
                    node = stripped[len(prefix) :].split(" - ", 1)[0]
                    names.append(node.rsplit("::", 1)[-1])
        elif stripped.startswith("test ") and stripped.endswith(" ... FAILED"):
            names.append(stripped[len("test ") : -len(" ... FAILED")])
    return sorted(set(names))


def pytest_errors(stdout: str) -> list[str]:
    errors = []
    for line in stdout.splitlines():
        stripped = line.strip()
        if stripped.startswith("ERROR "):
            node = stripped[len("ERROR ") :].split(" - ", 1)[0]
            errors.append(node.rsplit("::", 1)[-1])
    return sorted(set(errors))


def collected_tests(stdout: str, runner: str = "cargo") -> int:
    if runner == "pytest":
        summaries = [line for line in stdout.splitlines() if PYTEST_COMPLETE.search(line)]
        return sum(
            int(count) for line in summaries[-1:] for count, _kind in PYTEST_COUNT.findall(line)
        )
    total = 0
    for line in stdout.splitlines():
        stripped = line.strip()
        if stripped.startswith("running ") and stripped.endswith(("test", "tests")):
            words = stripped.split()
            if len(words) >= 2 and words[1].isdigit():
                total += int(words[1])
    return total


def completed(stdout: str, runner: str) -> bool:
    return bool(
        PYTEST_COMPLETE.search(stdout) if runner == "pytest" else CARGO_COMPLETE.search(stdout)
    )


def compile_failed(result: ProcessResult | object, runner: str = "cargo") -> bool:
    combined = str(getattr(result, "stdout", "")) + str(getattr(result, "stderr", ""))
    if runner == "pytest":
        return "SyntaxError" in combined or "error during collection" in combined.lower()
    return "could not compile" in combined or "error[E" in combined


def failure_output(combined: str, test_name: str, runner: str) -> str:
    short = test_name.rsplit("::", 1)[-1]
    if runner == "pytest":
        pieces: list[str] = []
        section = re.search(
            rf"^_{{2,}} .*\b{re.escape(short)}\b.* _{{2,}}$\n(.*?)(?=^_{{2,}} |^={{3,}} |\Z)",
            combined,
            re.MULTILINE | re.DOTALL,
        )
        if section:
            pieces.append(section.group(1))
        pieces.extend(
            line
            for line in combined.splitlines()
            if (line.startswith("FAILED ") or line.startswith("ERROR ")) and f"::{short}" in line
        )
        return "\n".join(pieces)
    section = re.search(
        rf"^---- {re.escape(test_name)} stdout ----$\n(.*?)(?=^---- |^failures:$|^test result:)",
        combined,
        re.MULTILINE | re.DOTALL,
    )
    return section.group(1) if section else ""


def _process_fields(result: ProcessResult | object) -> tuple[int | None, int | None, bool]:
    returncode = getattr(result, "returncode", None)
    exit_code = getattr(
        result, "exit_code", returncode if returncode is not None and returncode >= 0 else None
    )
    process_signal = getattr(
        result, "signal", -returncode if returncode is not None and returncode < 0 else None
    )
    return exit_code, process_signal, bool(getattr(result, "timed_out", False))


def evaluate(mutation: dict, result: ProcessResult | object) -> tuple[str, str, list[str]]:
    runner = mutation.get("runner", "cargo")
    stdout = str(getattr(result, "stdout", ""))
    combined = stdout + str(getattr(result, "stderr", ""))
    exit_code, process_signal, timed_out = _process_fields(result)
    failures = failing_tests(stdout, runner)
    count = collected_tests(stdout, runner)
    is_complete = completed(stdout, runner)
    if timed_out:
        return "INVALID_TIMEOUT", "test process exceeded its timeout", failures
    if process_signal is not None:
        return "INVALID_SIGNAL", f"test process terminated by signal {process_signal}", failures
    if compile_failed(result, runner):
        return "INVALID_COMPILE_ERROR", "mutated source did not compile", failures
    if runner == "pytest" and pytest_errors(stdout):
        return "INVALID_HARNESS_ERROR", f"pytest fixture errors: {pytest_errors(stdout)}", failures
    if count == 0:
        return "INVALID_NO_TESTS", "no tests were collected", failures
    if not is_complete:
        return "INVALID_PARTIAL", "test process did not emit its completion summary", failures
    if mutation.get("expect_no_failure"):
        if exit_code != 0:
            return "CONTROL_BAD_EXIT", f"control exited {exit_code}", failures
        if failures:
            return "CONTROL_BROKEN", f"control failed tests: {failures}", failures
        return "CONTROL_GREEN", f"suite stayed green across {count} tests", []
    if exit_code == 0:
        if failures:
            return "INVALID_EXIT_SUCCESS", "failure output was paired with exit code 0", failures
        return "SURVIVED", f"no test rejected this mutation ({count} tests ran)", []
    if exit_code is None:
        return "INVALID_PROCESS", "test process has no exit status", failures
    expected = sorted([mutation["expect_failing_test"], *mutation.get("expect_cofailures", [])])
    if not failures:
        return "INVALID_NO_FAILURE_DETAIL", f"process exited {exit_code} without a test failure", []
    if failures != expected:
        return (
            "UNRELATED_FAILURE_SET",
            f"expected exact failures {expected}; observed {failures}",
            failures,
        )
    if mutation["expect_message"] not in failure_output(
        combined, mutation["expect_failing_test"], runner
    ):
        return "WRONG_REASON", "expected failure reason was absent from the named test", failures
    return "KILLED", f"exact expected failure set observed: {expected}", failures


def baseline_evaluate(mutation: dict, result: ProcessResult) -> tuple[str, str]:
    control = {**mutation, "finding": "control", "expect_no_failure": True}
    status, detail, _failures = evaluate(control, result)
    if status == "CONTROL_GREEN":
        return "PASS", detail
    return "FAIL", f"unmutated baseline rejected: {status}: {detail}"


def apply_mutation(source: Path, mutation: dict) -> tuple[Path, str]:
    target = confined_source_file(source, mutation["file"], mutation_id=mutation["id"])
    original = target.read_text(encoding="utf-8")
    matches = original.count(mutation["find"])
    if matches != 1:
        raise RuntimeError(f"mutation {mutation['id']}: anchor matched {matches} times")
    target.write_text(original.replace(mutation["find"], mutation["replace"], 1), encoding="utf-8")
    return target, original


def _write_raw(raw_dir: Path, name: str, result: ProcessResult) -> tuple[str, str]:
    stdout_path = raw_dir / f"{name}.stdout.log"
    stderr_path = raw_dir / f"{name}.stderr.log"
    stdout_path.parent.mkdir(parents=True, exist_ok=True)
    stdout_path.write_text(result.stdout, encoding="utf-8")
    stderr_path.write_text(result.stderr, encoding="utf-8")
    return sha256_file(stdout_path), sha256_file(stderr_path)


def baseline_identity(record: dict[str, object]) -> str:
    if "identity_sha256" in record:
        raise ValueError("baseline identity payload must not contain its own digest")
    return canonical_digest(record)


def run_baseline(
    mutation: dict,
    campaign: IsolatedCampaign,
    raw_dir: Path,
    tools: list[dict[str, str]],
) -> Baseline:
    argv = test_command(mutation)
    env, recorded_env = command_environment(mutation, campaign)
    result = execute(argv, cwd=campaign.source, env=env, timeout_seconds=TEST_TIMEOUT_SECONDS)
    definition = command_identity(mutation)
    stdout_digest, stderr_digest = _write_raw(raw_dir, f"baseline-{definition}", result)
    stdout_artifact = f"raw/baseline-{definition}.stdout.log"
    stderr_artifact = f"raw/baseline-{definition}.stderr.log"
    status, detail = baseline_evaluate(mutation, result)
    runner = mutation.get("runner", "cargo")
    record: dict[str, object] = {
        "definition_sha256": definition,
        "source_head": campaign.source_head,
        "source_snapshot_sha256": campaign.snapshot_digest,
        "tools_sha256": canonical_digest(tools),
        "status": status,
        "detail": detail,
        "command": argv,
        "command_sha256": canonical_digest(argv),
        "environment": recorded_env,
        "profile": mutation.get("profile", "debug"),
        "exit_code": result.exit_code,
        "signal": result.signal,
        "timed_out": result.timed_out,
        "completed": completed(result.stdout, runner),
        "test_count": collected_tests(result.stdout, runner),
        "observed_failures": failing_tests(result.stdout, runner),
        "started_at": result.started_at,
        "finished_at": result.finished_at,
        "duration_s": result.duration_s,
        "stdout_artifact": stdout_artifact,
        "stderr_artifact": stderr_artifact,
        "stdout_sha256": stdout_digest,
        "stderr_sha256": stderr_digest,
    }
    return Baseline(
        identity_sha256=baseline_identity(record),
        definition_sha256=definition,
        source_head=campaign.source_head,
        source_snapshot_sha256=campaign.snapshot_digest,
        tools_sha256=canonical_digest(tools),
        status=status,
        detail=detail,
        command=argv,
        command_sha256=canonical_digest(argv),
        environment=recorded_env,
        profile=mutation.get("profile", "debug"),
        exit_code=result.exit_code,
        signal=result.signal,
        timed_out=result.timed_out,
        completed=completed(result.stdout, runner),
        test_count=collected_tests(result.stdout, runner),
        observed_failures=failing_tests(result.stdout, runner),
        started_at=result.started_at,
        finished_at=result.finished_at,
        duration_s=result.duration_s,
        stdout_artifact=stdout_artifact,
        stderr_artifact=stderr_artifact,
        stdout_sha256=stdout_digest,
        stderr_sha256=stderr_digest,
    )


def run_one(
    mutation: dict, campaign: IsolatedCampaign, baseline: Baseline, raw_dir: Path
) -> Outcome:
    expected = (
        []
        if mutation.get("expect_no_failure")
        else sorted([mutation["expect_failing_test"], *mutation.get("expect_cofailures", [])])
    )
    if baseline.status != "PASS":
        return Outcome(
            mutation_id=mutation["id"],
            finding=mutation["finding"],
            status="BLOCKED_BASELINE",
            detail=baseline.detail,
            duration_s=0,
            expected_failures=expected,
            baseline_sha256=baseline.identity_sha256,
        )
    target, original = apply_mutation(campaign.source, mutation)
    try:
        argv = test_command(mutation)
        env, recorded_env = command_environment(mutation, campaign)
        result = execute(argv, cwd=campaign.source, env=env, timeout_seconds=TEST_TIMEOUT_SECONDS)
        stdout_digest, stderr_digest = _write_raw(raw_dir, f"mutant-{mutation['id']}", result)
        status, detail, failures = evaluate(mutation, result)
        runner = mutation.get("runner", "cargo")
        return Outcome(
            mutation_id=mutation["id"],
            finding=mutation["finding"],
            status=status,
            detail=detail,
            duration_s=result.duration_s,
            expected_failures=expected,
            observed_failures=failures,
            exit_code=result.exit_code,
            signal=result.signal,
            timed_out=result.timed_out,
            completed=completed(result.stdout, runner),
            test_count=collected_tests(result.stdout, runner),
            command=argv,
            command_sha256=canonical_digest(argv),
            environment=recorded_env,
            profile=mutation.get("profile", "debug"),
            baseline_sha256=baseline.identity_sha256,
            stdout_sha256=stdout_digest,
            stderr_sha256=stderr_digest,
        )
    finally:
        target.write_text(original, encoding="utf-8")


def validate_anchors(source: Path, mutations: list[dict]) -> None:
    drifted = []
    for mutation in mutations:
        path = confined_source_file(source, mutation["file"], mutation_id=mutation["id"])
        count = path.read_text(encoding="utf-8").count(mutation["find"])
        if count != 1:
            drifted.append(f"{mutation['id']} ({count} matches)")
    if drifted:
        raise SystemExit("mutation anchors drifted: " + ", ".join(drifted))


def receipt_scope(mutations: list[dict]) -> dict:
    return {
        "runner_counts": dict(sorted(Counter(m.get("runner", "cargo") for m in mutations).items())),
        "control_entries": sum(bool(m.get("expect_no_failure")) for m in mutations),
        "semantics": "curated-single-edit-only-not-generated-score",
    }


def build_receipt(
    mutations: list[dict],
    outcomes: list[Outcome],
    baselines: list[Baseline],
    campaign: IsolatedCampaign,
    *,
    selection: str,
    source_after: str,
) -> dict:
    bad = [outcome for outcome in outcomes if outcome.status not in GOOD_STATUSES]
    return {
        "schema_version": 3,
        "kind": "curated-single-edit-inventory",
        "generated_at": utc_now(),
        "inventory": str(INVENTORY.relative_to(REPO)),
        "inventory_sha256": sha256_file(INVENTORY),
        "inventory_total": len(load_inventory(INVENTORY)),
        "selection": selection,
        "scope": receipt_scope(mutations),
        "campaign": campaign.identity(),
        "source_after_sha256": source_after,
        "source_unchanged": source_after == campaign.source_before,
        "status": "PASS" if not bad and source_after == campaign.source_before else "FAIL",
        "total": len(outcomes),
        "problems": len(bad),
        "status_counts": dict(sorted(Counter(o.status for o in outcomes).items())),
        "baselines": [asdict(baseline) for baseline in baselines],
        "results": [asdict(outcome) for outcome in outcomes],
    }


def tool_identity(source: Path) -> list[dict[str, str]]:
    values = {
        "python": exact_tool_version([sys.executable, "--version"], cwd=source),
        "cargo": exact_tool_version(["cargo", "--version"], cwd=source),
        "rustc": exact_tool_version(["rustc", "-Vv"], cwd=source),
    }
    return [
        {
            "name": name,
            "version": version,
            "identity_sha256": canonical_digest({"name": name, "version": version}),
        }
        for name, version in values.items()
    ]


def build_baseline_manifest(
    campaign: IsolatedCampaign,
    tools: list[dict[str, str]],
    baselines: list[Baseline],
) -> dict[str, object]:
    payload: dict[str, object] = {
        "schema_version": 1,
        "source": {
            "head": campaign.source_head,
            "snapshot_sha256": campaign.snapshot_digest,
            "dirty": campaign.source_dirty,
        },
        "tools": tools,
        "tools_sha256": canonical_digest(tools),
        "baselines": [asdict(baseline) for baseline in baselines],
    }
    return {**payload, "manifest_sha256": canonical_digest(payload)}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--id", action="append", help="run only this mutation id")
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--output-dir", type=Path, default=REPO / "target/sep21/v02/curated")
    parser.add_argument("--keep-isolation", action="store_true")
    args = parser.parse_args(argv)
    mutations = load_inventory(INVENTORY)
    if args.list:
        for mutation in mutations:
            print(mutation["id"])
        return 0
    selection = "full"
    if args.id:
        selection = "subset"
        wanted = set(args.id)
        unknown = wanted - {mutation["id"] for mutation in mutations}
        if unknown:
            raise SystemExit(f"unknown mutation ids: {sorted(unknown)}")
        mutations = [mutation for mutation in mutations if mutation["id"] in wanted]
    output_dir = prepare_output_dir(REPO, args.output_dir)
    campaign = create_isolated_campaign(REPO)
    try:
        validate_anchors(campaign.source, mutations)
        tools = tool_identity(campaign.source)
        baselines_by_id: dict[str, Baseline] = {}
        for mutation in mutations:
            identity = command_identity(mutation)
            if identity not in baselines_by_id:
                baselines_by_id[identity] = run_baseline(
                    mutation, campaign, output_dir / "raw", tools
                )
        outcomes = [
            run_one(
                mutation, campaign, baselines_by_id[command_identity(mutation)], output_dir / "raw"
            )
            for mutation in mutations
        ]
        source_after = campaign.source_after(REPO)
        receipt = build_receipt(
            mutations,
            outcomes,
            list(baselines_by_id.values()),
            campaign,
            selection=selection,
            source_after=source_after,
        )
        receipt["tools"] = tools
        baseline_path = output_dir / "mutation-baseline-manifest.json"
        receipt_path = output_dir / "receipt.mutations.json"
        baseline_manifest = build_baseline_manifest(campaign, tools, list(baselines_by_id.values()))
        write_json(baseline_path, baseline_manifest)
        receipt["baseline_manifest_sha256"] = sha256_file(baseline_path)
        write_json(receipt_path, receipt)
        raw_paths = sorted(path for path in (output_dir / "raw").iterdir() if path.is_file())
        envelope = evidence_envelope(
            repo=REPO,
            campaign=campaign,
            producer_id="mutation-campaign",
            command=[sys.executable, str(Path(__file__).resolve()), *sys.argv[1:]],
            environment={
                "CARGO_TARGET_DIR": str(campaign.target),
                "PYTHONDONTWRITEBYTECODE": "1",
            },
            tools=receipt["tools"],
            config_paths=[INVENTORY, REPO / "tools/verification/mutation-gate.json"],
            artifact_paths=[
                *[(path, "raw") for path in raw_paths],
                (baseline_path, "replay"),
                (receipt_path, "summary"),
            ],
            job="local-curated-mutations",
            status=receipt["status"],
            exit_code=0 if receipt["status"] == "PASS" else 1,
            started_at=campaign.created_at,
            finished_at=receipt["generated_at"],
            selected_count=len(mutations),
            executed_count=sum(outcome.status != "BLOCKED_BASELINE" for outcome in outcomes),
        )
        write_json(output_dir / "evidence-envelope.json", envelope)
        for outcome in outcomes:
            print(f"{outcome.mutation_id}: {outcome.status}")
        return 0 if receipt["status"] == "PASS" else 1
    finally:
        if not args.keep_isolation:
            campaign.cleanup()


if __name__ == "__main__":
    raise SystemExit(main())

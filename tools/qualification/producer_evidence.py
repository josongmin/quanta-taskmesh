#!/usr/bin/env python3
"""Register and validate producer-owned EvidenceEnvelopeV1 artifacts.

The collector consumes structured summaries and envelopes only. Raw logs are
retained and hashed by the producer envelope, but are never reparsed here.
"""

from __future__ import annotations

import hashlib
import json
from fnmatch import fnmatchcase
from pathlib import Path
from typing import Any

from tools.qualification.evidence import canonical_digest, envelope_problems, file_identity

REQUIRED_REGISTRATIONS = {
    "mutation-campaign-curated-v1",
    "mutation-campaign-generated-v1",
    "taskmesh-fuzz-v03",
    "taskmesh-modelcheck-v03",
}


def _load(path: Path) -> object:
    return json.loads(path.read_text(encoding="utf-8"))


def registrations(inventory_path: Path) -> list[dict[str, Any]]:
    inventory = _load(inventory_path)
    if not isinstance(inventory, dict) or not isinstance(inventory.get("gates"), list):
        raise ValueError("gate inventory is malformed")
    found = []
    for gate in inventory["gates"]:
        if not isinstance(gate, dict) or "producer" not in gate:
            continue
        producer = gate["producer"]
        if not isinstance(producer, dict):
            raise ValueError(f"gate {gate.get('id')!r} has malformed producer registration")
        found.append({"gate_id": gate.get("id"), **producer})
    return found


def clear_registered_outputs(root: Path, specs: list[dict[str, Any]]) -> None:
    """Remove only fixed producer sidecars so a crashed run cannot reuse them."""
    repo_root = root.resolve()
    allowed_root = repo_root / "target"
    if allowed_root.is_symlink():
        raise ValueError("repository target/ must not be a symlink")
    for spec in specs:
        for key in ("summary", "envelope"):
            value = spec.get(key)
            if isinstance(value, str):
                declared = Path(value)
                if declared.is_absolute() or ".." in declared.parts:
                    raise ValueError(f"producer {key} path must be a safe repo-relative path")
                candidate = repo_root / declared
                resolved = candidate.resolve()
                try:
                    relative = resolved.relative_to(allowed_root)
                except ValueError as exc:
                    raise ValueError(
                        f"producer {key} path must stay below repository target/"
                    ) from exc
                if not relative.parts or resolved == allowed_root:
                    raise ValueError(f"producer {key} path must not be the target root")
                if candidate.exists() and not candidate.is_file() and not candidate.is_symlink():
                    raise ValueError(f"producer {key} path must name a file")
                candidate.unlink(missing_ok=True)


def collect_records(root: Path, specs: list[dict[str, Any]]) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    for spec in specs:
        record: dict[str, Any] = {
            "registration": spec.get("registration"),
            "gate_id": spec.get("gate_id"),
            "surface": spec.get("surface"),
        }
        try:
            manifest_path = root / str(spec["manifest"])
            summary_path = root / str(spec["summary"])
            envelope_path = root / str(spec["envelope"])
            manifest = _load(manifest_path)
            summary = _load(summary_path)
            envelope = _load(envelope_path)
            record.update(
                {
                    "manifest": {
                        "identity": file_identity(manifest_path, relative_to=root),
                        "payload_sha256": canonical_digest(manifest),
                        "payload": manifest,
                    },
                    "summary": {
                        "identity": file_identity(summary_path, relative_to=root),
                        "payload_sha256": canonical_digest(summary),
                        "payload": summary,
                    },
                    "envelope": {
                        "identity": file_identity(envelope_path, relative_to=root),
                        "payload_sha256": canonical_digest(envelope),
                        "payload": envelope,
                    },
                }
            )
        except (KeyError, OSError, json.JSONDecodeError, ValueError) as exc:
            record["error"] = str(exc)
        records.append(record)
    return records


def _identity_problems(
    identity: object, *, expected_path: str, name: str, root: Path | None
) -> list[str]:
    if not isinstance(identity, dict):
        return [f"{name} identity is missing or malformed"]
    problems = []
    if identity.get("path") != expected_path:
        problems.append(f"{name} path does not match registration")
    digest = identity.get("sha256")
    if not isinstance(digest, str) or len(digest) != 64:
        problems.append(f"{name} sha256 is missing or malformed")
    if not isinstance(identity.get("size"), int) or identity.get("size", 0) <= 0:
        problems.append(f"{name} size is missing or non-positive")
    if root is not None:
        path = root / expected_path
        if not path.is_file():
            problems.append(f"{name} {expected_path!r} is missing")
        else:
            data = path.read_bytes()
            if identity.get("size") != len(data):
                problems.append(f"{name} size does not match sidecar")
            if identity.get("sha256") != hashlib.sha256(data).hexdigest():
                problems.append(f"{name} digest does not match sidecar")
    return problems


def _embedded_sidecar_problems(*, root: Path, path: str, embedded: object, name: str) -> list[str]:
    try:
        sidecar = _load(root / path)
    except (OSError, json.JSONDecodeError) as exc:
        return [f"cannot parse {name} sidecar: {exc}"]
    if sidecar != embedded:
        return [f"{name} sidecar differs from embedded payload"]
    return []


def _curated_problems(
    summary: dict[str, Any], result: dict[str, Any], inventory_path: Path
) -> list[str]:
    problems = []
    results = summary.get("results")
    statuses = summary.get("status_counts")
    total = summary.get("total")
    if summary.get("schema_version") != 3 or summary.get("kind") != "curated-single-edit-inventory":
        problems.append("curated summary schema/kind mismatch")
    if summary.get("selection") != "full":
        problems.append("curated mutation evidence is not a full-inventory run")
    if not isinstance(results, list) or not results:
        problems.append("curated mutation results are empty")
        results = []
    ids = [item.get("mutation_id") for item in results if isinstance(item, dict)]
    if len(ids) != len(results) or any(not isinstance(value, str) or not value for value in ids):
        problems.append("curated mutation result identity is missing")
    if all(isinstance(value, str) for value in ids) and len(ids) != len(set(ids)):
        problems.append("curated mutation result identities are duplicated")
    try:
        inventory_bytes = inventory_path.read_bytes()
        inventory = json.loads(inventory_bytes)
        expected_ids = [item["id"] for item in inventory["mutations"]]
        if not expected_ids or len(expected_ids) != len(set(expected_ids)):
            raise ValueError("inventory has empty or duplicate IDs")
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as exc:
        problems.append(f"curated mutation inventory cannot be verified: {exc}")
    else:
        if summary.get("inventory") != "tools/verification/mutations.json":
            problems.append("curated mutation inventory path differs from registration")
        if summary.get("inventory_sha256") != hashlib.sha256(inventory_bytes).hexdigest():
            problems.append("curated mutation inventory digest differs from source")
        if ids != expected_ids:
            problems.append("curated mutation inventory IDs/order differ from executed results")
        if summary.get("inventory_total") != len(expected_ids):
            problems.append("curated mutation inventory denominator differs from source")
    counts: dict[str, int] = {}
    for item in results:
        if isinstance(item, dict):
            status = item.get("status")
            counts[str(status)] = counts.get(str(status), 0) + 1
            if not isinstance(status, str) or status not in {"KILLED", "CONTROL_GREEN"}:
                problems.append(f"curated mutation {item.get('mutation_id')} did not pass")
    if statuses != dict(sorted(counts.items())):
        problems.append("curated mutation status_counts drift from results")
    if total != len(results) or summary.get("inventory_total") != total:
        problems.append("curated mutation denominator is partial")
    if result.get("selected_count") != total or result.get("executed_count") != total:
        problems.append("curated envelope count differs from summary denominator")
    if summary.get("problems") != 0 or summary.get("status") != "PASS":
        problems.append("curated summary is not PASS")
    if summary.get("source_unchanged") is not True:
        problems.append("curated campaign changed the source")
    return problems


def _generated_problems(
    summary: dict[str, Any], result: dict[str, Any], envelope: dict[str, Any]
) -> list[str]:
    problems = []
    categories = {"caught", "missed", "unviable", "timeout", "equivalent"}
    counts = summary.get("counts")
    if summary.get("schema_version") != 2 or summary.get("kind") != "generated-cargo-mutants":
        problems.append("generated summary schema/kind mismatch")
    if not isinstance(counts, dict) or set(counts) != categories:
        problems.append("generated mutation categories are incomplete")
        counts = {}
    if any(type(value) is not int or value < 0 for value in counts.values()):
        problems.append("generated mutation counts are invalid")
    denominator = sum(value for value in counts.values() if type(value) is int)
    if summary.get("denominator") != denominator or denominator <= 0:
        problems.append("generated mutation denominator is empty or inconsistent")
    expected_quality = {
        "numerator": counts.get("caught", 0),
        "denominator": denominator - counts.get("unviable", 0),
        "excluded": {"unviable": counts.get("unviable", 0)},
    }
    if summary.get("quality") != expected_quality:
        problems.append("generated mutation quality accounting is inconsistent")
    expected_limitations = ["unviable"] if counts.get("unviable", 0) else []
    if summary.get("limitations") != expected_limitations:
        problems.append("generated mutation limitations are incomplete")
    if result.get("selected_count") != denominator or result.get("executed_count") != denominator:
        problems.append("generated envelope count differs from summary denominator")
    command = summary.get("command")
    argv = command.get("argv") if isinstance(command, dict) else None
    if not isinstance(argv, list) or "--workspace" not in argv or "--package" in argv:
        problems.append("generated mutation evidence is a subset, not the full workspace")
    envelope_command = envelope.get("command")
    if not isinstance(envelope_command, dict) or argv != envelope_command.get("argv"):
        problems.append("generated mutation command differs from envelope execution command")
    outcomes = summary.get("outcomes")
    if not isinstance(outcomes, dict) or set(outcomes) != {
        "caught",
        "missed",
        "unviable",
        "timeout",
    }:
        problems.append("generated mutation outcomes are missing or malformed")
    else:
        seen: set[str] = set()
        for category in ("caught", "missed", "unviable", "timeout"):
            names = outcomes[category]
            if (
                not isinstance(names, list)
                or any(not isinstance(name, str) or not name for name in names)
                or len(names) != len(set(names))
            ):
                problems.append(f"generated mutation outcomes {category} IDs are malformed")
                continue
            if seen.intersection(names):
                problems.append("generated mutation outcomes contain duplicate cross-category IDs")
            seen.update(names)
            if counts.get(category) != len(names):
                problems.append(
                    f"generated mutation outcomes {category} count differs from summary"
                )
        planned = summary.get("planned_mutants")
        if (
            not isinstance(planned, list)
            or not planned
            or any(not isinstance(name, str) or not name for name in planned)
            or len(planned) != len(set(planned))
            or sorted(planned) != sorted(seen)
            or len(planned) != denominator
        ):
            problems.append("generated mutation planned IDs differ from outcomes/denominator")
    baseline = summary.get("baseline_sha256")
    if (
        not isinstance(baseline, str)
        or len(baseline) != 64
        or any(character not in "0123456789abcdef" for character in baseline)
    ):
        problems.append("generated mutation baseline digest is missing or malformed")
    process = summary.get("process")
    if not isinstance(process, dict) or (
        process.get("exit_code") != 0
        or process.get("signal") is not None
        or process.get("timed_out") is not False
    ):
        problems.append("generated mutation process did not complete cleanly")
    if counts.get("caught", 0) <= 0:
        problems.append("generated mutation campaign has no scored caught mutant")
    if any(counts.get(name, 0) != 0 for name in ("missed", "timeout", "equivalent")):
        problems.append("generated mutation campaign has unresolved quality outcomes")
    if summary.get("status") != "PASS" or summary.get("problems") != []:
        problems.append("generated mutation summary is not PASS")
    if summary.get("source_unchanged") is not True:
        problems.append("generated mutation campaign changed the source")
    return problems


def _generated_structured_raw_problems(
    root: Path, spec: dict[str, Any], summary: dict[str, Any], envelope: dict[str, Any]
) -> list[str]:
    """Bind producer-parsed identities to hashed cargo-mutants JSON/category artifacts."""
    problems: list[str] = []
    output_dir = (root / spec["summary"]).parent
    raw_dir = output_dir / "raw" / "mutants.out"
    required = [
        "mutants.json",
        "outcomes.json",
        "caught.txt",
        "missed.txt",
        "unviable.txt",
        "timeout.txt",
    ]
    raw_records = summary.get("raw_artifacts")
    if not isinstance(raw_records, list):
        return ["generated mutation raw artifact identities are missing"]
    raw_root = output_dir / "raw"
    disk_files = sorted(path for path in raw_root.rglob("*") if path.is_file())
    disk_paths = {path.relative_to(raw_root).as_posix() for path in disk_files}
    declared_paths = [item.get("path") for item in raw_records if isinstance(item, dict)]
    if (
        len(declared_paths) != len(raw_records)
        or any(not isinstance(path, str) or not path for path in declared_paths)
        or len(declared_paths) != len(set(declared_paths))
        or set(declared_paths) != disk_paths
    ):
        problems.append("generated mutation raw artifact paths are duplicate or differ from disk")
    raw_by_path = {
        item.get("path"): item
        for item in raw_records
        if isinstance(item, dict) and isinstance(item.get("path"), str)
    }
    envelope_artifacts = envelope.get("artifacts")
    artifact_by_path = (
        {
            item.get("path"): item
            for item in envelope_artifacts
            if isinstance(item, dict) and isinstance(item.get("path"), str)
        }
        if isinstance(envelope_artifacts, list)
        else {}
    )
    for disk in disk_files:
        relative = disk.relative_to(raw_root).as_posix()
        data = disk.read_bytes()
        claimed = raw_by_path.get(relative)
        if not isinstance(claimed, dict) or (
            claimed.get("sha256") != hashlib.sha256(data).hexdigest()
            or claimed.get("size") != len(data)
        ):
            problems.append(f"generated mutation raw artifact {relative} differs from disk")
    documents: dict[str, Any] = {}
    for name in required:
        disk = raw_dir / name
        relative = f"mutants.out/{name}"
        if not disk.is_file():
            problems.append(f"generated mutation structured raw {name} is missing")
            continue
        data = disk.read_bytes()
        digest = hashlib.sha256(data).hexdigest()
        raw_record = raw_by_path.get(relative)
        envelope_path = (output_dir / "raw" / relative).relative_to(root).as_posix()
        artifact = artifact_by_path.get(envelope_path)
        if not isinstance(raw_record, dict) or (
            raw_record.get("sha256") != digest or raw_record.get("size") != len(data)
        ):
            problems.append(
                f"generated mutation structured raw {name} differs from summary identity"
            )
        if data and (
            not isinstance(artifact, dict)
            or artifact.get("sha256") != digest
            or artifact.get("size") != len(data)
            or artifact.get("role") != "raw"
        ):
            problems.append(f"generated mutation structured raw {name} differs from envelope")
        if name.endswith(".json"):
            try:
                documents[name] = json.loads(data)
            except json.JSONDecodeError:
                problems.append(f"generated mutation structured raw {name} is malformed JSON")
        else:
            try:
                documents[name] = [
                    line.strip() for line in data.decode("utf-8").splitlines() if line.strip()
                ]
            except UnicodeDecodeError:
                problems.append(f"generated mutation structured raw {name} is not UTF-8")
    planned_rows = documents.get("mutants.json")
    if isinstance(planned_rows, list):
        planned = [item.get("name") for item in planned_rows if isinstance(item, dict)]
        claimed_planned = summary.get("planned_mutants")
        if (
            len(planned) != len(planned_rows)
            or any(not isinstance(name, str) for name in planned)
            or not isinstance(claimed_planned, list)
            or any(not isinstance(name, str) for name in claimed_planned)
            or sorted(planned) != sorted(claimed_planned)
        ):
            problems.append(
                "generated mutation planned IDs differ from structured raw mutants.json"
            )
    elif "mutants.json" in documents:
        problems.append("generated mutation structured raw mutants.json is not a list")
    outcome_doc = documents.get("outcomes.json")
    rows = outcome_doc.get("outcomes") if isinstance(outcome_doc, dict) else None
    if isinstance(rows, list):
        baseline_rows = [
            row for row in rows if isinstance(row, dict) and row.get("scenario") == "Baseline"
        ]
        if len(baseline_rows) != 1 or baseline_rows[0].get("summary") != "Success":
            problems.append("generated mutation structured raw baseline is not a single success")
        elif canonical_digest(baseline_rows[0]) != summary.get("baseline_sha256"):
            problems.append("generated mutation baseline digest differs from structured raw")
        categories = {
            "CaughtMutant": "caught",
            "MissedMutant": "missed",
            "Unviable": "unviable",
            "Timeout": "timeout",
        }
        executed = {name: [] for name in categories.values()}
        for row in rows:
            if not isinstance(row, dict):
                problems.append("generated mutation structured raw contains non-object outcome")
                continue
            if row.get("scenario") == "Baseline":
                continue
            scenario = row.get("scenario")
            mutant = scenario.get("Mutant") if isinstance(scenario, dict) else None
            name = mutant.get("name") if isinstance(mutant, dict) else None
            outcome_kind = row.get("summary")
            category = categories.get(outcome_kind) if isinstance(outcome_kind, str) else None
            if not isinstance(name, str) or category is None:
                problems.append("generated mutation structured raw contains invalid outcome")
                continue
            executed[category].append(name)
        claimed_outcomes = summary.get("outcomes")
        for category, names in executed.items():
            claimed = claimed_outcomes.get(category) if isinstance(claimed_outcomes, dict) else None
            if not isinstance(claimed, list) or sorted(names) != sorted(claimed):
                problems.append(
                    f"generated mutation {category} IDs differ from structured raw outcomes.json"
                )
            listed = documents.get(f"{category}.txt")
            if not isinstance(listed, list) or sorted(listed) != sorted(names):
                problems.append(
                    f"generated mutation {category} IDs differ from structured raw category file"
                )
    elif "outcomes.json" in documents:
        problems.append("generated mutation structured raw outcomes.json is malformed")
    return problems


def _fuzz_problems(
    summary: dict[str, Any], result: dict[str, Any], manifest: dict[str, Any]
) -> list[str]:
    problems = []
    required = manifest.get("required_targets")
    targets = summary.get("targets")
    if not isinstance(required, dict) or not isinstance(targets, list):
        return ["fuzz manifest/summary target set is malformed"]
    if (
        summary.get("schema_version") != 1
        or summary.get("kind") != "taskmesh-fuzz-evidence"
        or summary.get("producer_id") != manifest.get("producer_id")
    ):
        problems.append("fuzz summary schema/kind/producer identity mismatch")
    by_name = {
        item.get("target"): item
        for item in targets
        if isinstance(item, dict) and isinstance(item.get("target"), str) and item.get("target")
    }
    target_names = [item.get("target") for item in targets if isinstance(item, dict)]
    if (
        len(target_names) != len(targets)
        or any(not isinstance(name, str) or not name for name in target_names)
        or len(target_names) != len(set(target_names))
        or len(targets) != len(required)
    ):
        problems.append("fuzz target rows are duplicate, malformed, or not one-to-one")
    declared = summary.get("required_targets")
    executed = summary.get("executed_targets")
    if not isinstance(declared, list) or not all(isinstance(name, str) for name in declared):
        problems.append("fuzz declared required target set is malformed")
        declared = []
    if not isinstance(executed, list) or not all(isinstance(name, str) for name in executed):
        problems.append("fuzz executed target set is malformed")
        executed = []
    if set(by_name) != set(required):
        problems.append("fuzz summary target set differs from manifest")
    if (
        len(declared) != len(required)
        or len(declared) != len(set(declared))
        or set(declared) != set(required)
    ):
        problems.append("fuzz declared required target set differs from manifest")
    if (
        len(executed) != len(required)
        or len(executed) != len(set(executed))
        or set(executed) != set(required)
    ):
        problems.append("fuzz executed target set is partial")
    for name, checkpoints in required.items():
        item = by_name.get(name, {})
        witnesses = item.get("semantic_checkpoints") if isinstance(item, dict) else None
        if not isinstance(checkpoints, list) or not all(
            isinstance(checkpoint, str) for checkpoint in checkpoints
        ):
            problems.append(f"fuzz manifest checkpoints for {name} are malformed")
            continue
        if (
            item.get("attempted") is not True
            or type(item.get("runs")) is not int
            or item["runs"] <= 0
            or type(item.get("valid_inputs")) is not int
            or item["valid_inputs"] <= 0
        ):
            problems.append(f"fuzz target {name} has zero or missing exploration")
        if (
            not isinstance(witnesses, dict)
            or set(witnesses) != set(checkpoints)
            or any(
                type(witnesses.get(checkpoint)) is not int or witnesses[checkpoint] <= 0
                for checkpoint in checkpoints
            )
        ):
            problems.append(f"fuzz target {name} has zero, partial, or extra semantic witnesses")
    if result.get("selected_count") != len(required) or result.get("executed_count") != len(
        required
    ):
        problems.append("fuzz envelope target count differs from manifest")
    if summary.get("status") != "PASS" or summary.get("problems") != []:
        problems.append("fuzz summary is not PASS")
    if summary.get("source_stable") is not True:
        problems.append("fuzz source changed during execution")
    return problems


def _expected_model_rows(manifest: dict[str, Any]) -> tuple[dict[str, dict[str, Any]], list[str]]:
    """Project only V03 producer-emitted fields from the reviewed manifest."""
    checkers = manifest.get("checkers")
    if not isinstance(checkers, dict) or set(checkers) != {"loom", "shuttle"}:
        return {}, ["model manifest checkers are malformed"]
    loom, shuttle = checkers.get("loom"), checkers.get("shuttle")
    if not isinstance(loom, dict) or not isinstance(shuttle, dict):
        return {}, ["model manifest checkers are malformed"]
    loom_ids, bounds = loom.get("required_models"), loom.get("bounds")
    shuttle_models = shuttle.get("required_models")
    if (
        not isinstance(loom_ids, list)
        or not loom_ids
        or any(not isinstance(name, str) or not name for name in loom_ids)
        or len(loom_ids) != len(set(loom_ids))
        or not isinstance(bounds, dict)
        or set(bounds) != {"max_threads", "max_branches", "max_permutations", "preemption_bound"}
        or not isinstance(shuttle_models, dict)
        or not shuttle_models
        or not isinstance(shuttle.get("scheduler"), str)
        or type(shuttle.get("max_steps")) is not int
        or shuttle["max_steps"] <= 0
        or any(
            not isinstance(name, str)
            or not name
            or not isinstance(spec, dict)
            or set(spec) != {"seed", "requested"}
            or type(spec.get("seed")) is not int
            or type(spec.get("requested")) is not int
            or spec["requested"] <= 0
            for name, spec in shuttle_models.items()
        )
        or set(loom_ids).intersection(shuttle_models)
    ):
        return {}, ["model manifest required models/bounds are malformed"]
    expected = {name: {"checker": "loom", "model_id": name, **bounds} for name in loom_ids}
    expected.update(
        {
            name: {
                "checker": "shuttle",
                "model_id": name,
                "scheduler": shuttle["scheduler"],
                "seed": spec["seed"],
                "requested": spec["requested"],
                "completed": spec["requested"],
                "max_steps": shuttle["max_steps"],
            }
            for name, spec in shuttle_models.items()
        }
    )
    return expected, []


def _model_problems(
    summary: dict[str, Any],
    result: dict[str, Any],
    manifest: dict[str, Any],
    envelope: dict[str, Any],
) -> list[str]:
    problems = []
    models = summary.get("models")
    if not isinstance(models, list):
        return ["model summary models are malformed"]
    if any(
        not isinstance(item, dict) or not isinstance(item.get("model_id"), str) for item in models
    ):
        return ["model summary model identity is malformed"]
    expected_rows, manifest_problems = _expected_model_rows(manifest)
    if manifest_problems:
        return manifest_problems
    if (
        summary.get("schema_version") != 1
        or summary.get("kind") != "taskmesh-modelcheck-evidence"
        or summary.get("producer_id") != manifest.get("producer_id")
    ):
        problems.append("model summary schema/kind/producer identity mismatch")
    by_id = {item["model_id"]: item for item in models}
    required = set(expected_rows)
    model_ids = [item["model_id"] for item in models]
    if len(model_ids) != len(set(model_ids)) or len(models) != len(required):
        problems.append("model rows are duplicate or not one-to-one with manifest")
    if set(by_id) != required:
        problems.append("model summary set differs from manifest")
    for model_id, expected in expected_rows.items():
        item = by_id.get(model_id)
        if item is None:
            continue
        checker = expected["checker"]
        if item.get("checker") != checker:
            problems.append(f"{checker} model {model_id} checker identity differs from manifest")
        if checker == "loom":
            if type(item.get("completed")) is not int or item["completed"] <= 0:
                problems.append(f"loom model {model_id} has zero exploration")
            without_completion = {key: value for key, value in item.items() if key != "completed"}
            if without_completion != expected:
                problems.append(f"loom model {model_id} fields differ from manifest")
        elif item != expected:
            problems.append(f"shuttle model {model_id} fields differ from manifest")
    replay = summary.get("replay")
    expected_replay = manifest.get("replay", {})
    if not isinstance(expected_replay, dict):
        expected_replay = {}
        problems.append("model manifest replay is malformed")
    if (
        not isinstance(replay, dict)
        or set(replay) != {"model_id", "seed", "result", "artifact"}
        or any(
            replay.get(key) != expected_replay.get(key) for key in ("model_id", "seed", "result")
        )
    ):
        problems.append("model replay identity/result differs from manifest")
    replay_identity = replay.get("artifact") if isinstance(replay, dict) else None
    envelope_artifacts = envelope.get("artifacts")
    replay_artifacts = (
        [
            item
            for item in envelope_artifacts
            if isinstance(item, dict) and item.get("role") == "replay"
        ]
        if isinstance(envelope_artifacts, list)
        else []
    )
    if (
        not isinstance(replay_identity, dict)
        or len(replay_artifacts) != 1
        or replay_identity != replay_artifacts[0]
        or not isinstance(expected_replay.get("artifact_glob"), str)
        or not isinstance(replay_identity.get("path"), str)
        or not fnmatchcase(replay_identity["path"], expected_replay["artifact_glob"])
    ):
        problems.append("model replay artifact identity differs from envelope")
    processes = summary.get("processes")
    if not isinstance(processes, dict) or set(processes) != {
        "loom",
        "shuttle",
        "replay_generate",
        "replay_verify",
    }:
        problems.append("model process set is incomplete")
    else:
        for name, process in processes.items():
            if (
                not isinstance(process, dict)
                or process.get("exit_code") != 0
                or process.get("signal") is not None
                or process.get("timed_out") is not False
            ):
                problems.append(f"model process {name} did not complete cleanly")
    if result.get("selected_count") != len(required) or result.get("executed_count") != len(
        required
    ):
        problems.append("model envelope count differs from manifest")
    if summary.get("status") != "PASS" or summary.get("problems") != []:
        problems.append("model summary is not PASS")
    if summary.get("source_stable") is not True:
        problems.append("model source changed during execution")
    return problems


def producer_records_problems(
    records: object,
    specs: list[dict[str, Any]],
    gates: dict[str, dict[str, Any]],
    source: dict[str, Any],
    expected_action: object,
    *,
    root: Path | None = None,
) -> list[str]:
    if not isinstance(records, list):
        return ["producer records are missing or not a list"]
    problems: list[str] = []
    expected = {spec["registration"]: spec for spec in specs}
    actual = {
        item.get("registration"): item
        for item in records
        if isinstance(item, dict) and isinstance(item.get("registration"), str)
    }
    if len(actual) != len(records):
        problems.append("producer records contain malformed or duplicate registrations")
    if set(expected) != REQUIRED_REGISTRATIONS:
        problems.append("shared inventory producer registration set is incomplete")
    if set(actual) != set(expected):
        problems.append("combined receipt producer registration set differs from inventory")
    expected_source = {
        "head": source.get("head"),
        "tree_digest": source.get("paths_digest"),
        "dirty": source.get("dirty"),
    }
    for registration, spec in expected.items():
        record = actual.get(registration)
        if record is None:
            continue
        prefix = f"producer {registration}: "
        if record.get("gate_id") != spec.get("gate_id") or record.get("surface") != spec.get(
            "surface"
        ):
            problems.append(prefix + "gate/surface identity differs from inventory")
        if record.get("error"):
            problems.append(prefix + f"evidence unavailable: {record['error']}")
            continue
        manifest_record = record.get("manifest")
        summary_record = record.get("summary")
        envelope_record = record.get("envelope")
        if not all(
            isinstance(value, dict) for value in (manifest_record, summary_record, envelope_record)
        ):
            problems.append(prefix + "manifest/summary/envelope record is malformed")
            continue
        if not all(
            isinstance(record_part.get("identity"), dict)
            and isinstance(record_part.get("payload_sha256"), str)
            for record_part in (manifest_record, summary_record, envelope_record)
        ):
            problems.append(prefix + "manifest/summary/envelope record identity is malformed")
            continue
        problems.extend(
            prefix + issue
            for issue in _identity_problems(
                manifest_record.get("identity"),
                expected_path=spec["manifest"],
                name="manifest",
                root=root,
            )
        )
        problems.extend(
            prefix + issue
            for issue in _identity_problems(
                summary_record.get("identity"),
                expected_path=spec["summary"],
                name="summary",
                root=root,
            )
        )
        problems.extend(
            prefix + issue
            for issue in _identity_problems(
                envelope_record.get("identity"),
                expected_path=spec["envelope"],
                name="envelope",
                root=root,
            )
        )
        summary = summary_record.get("payload")
        envelope = envelope_record.get("payload")
        manifest_payload = manifest_record.get("payload")
        if not all(isinstance(value, dict) for value in (manifest_payload, summary, envelope)):
            problems.append(prefix + "manifest/summary/envelope payload is malformed")
            continue
        # The envelope schema validator reports detailed errors, but its input
        # is still downloaded JSON. Do not dereference malformed nested values
        # in the registration/semantic layer after reporting those errors.
        nested = {
            "producer": dict,
            "source": dict,
            "command": dict,
            "tools": list,
            "configs": list,
            "action": dict,
            "artifacts": list,
            "result": dict,
        }
        malformed = [
            name for name, kind in nested.items() if not isinstance(envelope.get(name), kind)
        ]
        if malformed:
            problems.append(prefix + f"envelope nested shape is malformed: {malformed}")
            continue
        if any(
            not isinstance(item, dict)
            for item in envelope["tools"] + envelope["configs"] + envelope["artifacts"]
        ):
            problems.append(prefix + "envelope tool/config/artifact entry is malformed")
            continue
        if root is not None:
            for path, embedded, name in (
                (spec["manifest"], manifest_payload, "manifest"),
                (spec["summary"], summary, "summary"),
                (spec["envelope"], envelope, "envelope"),
            ):
                problems.extend(
                    prefix + issue
                    for issue in _embedded_sidecar_problems(
                        root=root, path=path, embedded=embedded, name=name
                    )
                )
        if manifest_record.get("payload_sha256") != canonical_digest(manifest_payload):
            problems.append(prefix + "embedded manifest digest mismatch")
        if summary_record.get("payload_sha256") != canonical_digest(summary):
            problems.append(prefix + "embedded summary digest mismatch")
        if envelope_record.get("payload_sha256") != canonical_digest(envelope):
            problems.append(prefix + "embedded envelope digest mismatch")
        try:
            envelope_issues = envelope_problems(
                envelope, artifact_root=root, expected_source=expected_source
            )
        except (AttributeError, KeyError, TypeError, ValueError) as exc:
            problems.append(prefix + f"envelope nested value is malformed: {type(exc).__name__}")
            continue
        problems.extend(prefix + issue for issue in envelope_issues)
        producer = envelope.get("producer", {})
        if producer.get("id") != spec.get("producer_id") or producer.get("version") != spec.get(
            "version"
        ):
            problems.append(prefix + "producer id/version differs from inventory")
        manifest_identity = manifest_record.get("identity", {})
        configs = envelope.get("configs", [])
        if not any(
            isinstance(config, dict)
            and config.get("path") == spec.get("manifest")
            and config.get("sha256") == manifest_identity.get("sha256")
            for config in configs
        ):
            problems.append(prefix + "envelope is not bound to the registered manifest")
        config_paths = {
            config.get("path")
            for config in configs
            if isinstance(config, dict) and isinstance(config.get("path"), str)
        }
        if config_paths != set(spec.get("required_configs", [])):
            problems.append(prefix + "envelope config set differs from inventory")
        action = envelope.get("action", {})
        if not isinstance(expected_action, dict):
            problems.append(prefix + "collector action identity is missing")
        elif expected_action.get("context") == "github-actions":
            for key in (
                "context",
                "workflow",
                "workflow_ref",
                "workflow_sha256",
                "source_sha",
                "event",
            ):
                if action.get(key) != expected_action.get(key):
                    problems.append(prefix + f"action {key} differs from collector runtime")
            if action.get("job") != spec.get("hosted_job"):
                problems.append(prefix + "action job differs from registered producer job")
        elif (
            action.get("context") != "local"
            or action.get("event") != "local"
            or action.get("source_sha") != expected_source["head"]
        ):
            problems.append(prefix + "local action context/source identity is invalid")
        summary_identity = summary_record.get("identity", {})
        if not any(
            isinstance(artifact, dict)
            and artifact.get("role") == "summary"
            and artifact.get("path") == summary_identity.get("path")
            and artifact.get("sha256") == summary_identity.get("sha256")
            and artifact.get("size") == summary_identity.get("size")
            for artifact in envelope.get("artifacts", [])
        ):
            problems.append(prefix + "summary identity differs from envelope artifact")
        result = envelope.get("result", {})
        if spec.get("surface") in {"curated", "generated"}:
            campaign = summary.get("campaign")
            if not isinstance(campaign, dict) or (
                campaign.get("source_head") != expected_source["head"]
                or campaign.get("snapshot_sha256") != expected_source["tree_digest"]
                or campaign.get("source_dirty") != expected_source["dirty"]
            ):
                problems.append(prefix + "campaign source identity differs from envelope")
        gate = gates.get(spec["gate_id"])
        if gate is None:
            problems.append(prefix + "registered gate result is missing")
        else:
            expected_gate_status = "PASS" if result.get("status") == "PASS" else "FAIL"
            if gate.get("status") != expected_gate_status:
                problems.append(prefix + "gate status differs from producer result")
            if gate.get("exit_code") != result.get("exit_code"):
                problems.append(prefix + "gate exit code differs from producer result")
            if (
                gate.get("evidence_kind") != "producer"
                or gate.get("producer_registration") != registration
                or gate.get("evidence_digest") != envelope_record.get("payload_sha256")
            ):
                problems.append(prefix + "gate evidence linkage differs from envelope")
            if (
                isinstance(expected_action, dict)
                and expected_action.get("context") == "github-actions"
            ):
                if gate.get("imported_producer") is not True:
                    problems.append(
                        prefix + "hosted gate is not marked as imported producer evidence"
                    )
                if gate.get("derived_from") != spec.get("envelope"):
                    problems.append(
                        prefix + "hosted gate derived_from differs from registered envelope"
                    )
                if gate.get("producer_started_at") != result.get("started_at") or gate.get(
                    "producer_finished_at"
                ) != result.get("finished_at"):
                    problems.append(prefix + "hosted gate producer timing differs from envelope")
        surface = spec.get("surface")
        semantic: list[str]
        if surface == "curated":
            inventory_root = root if root is not None else Path(__file__).resolve().parents[2]
            semantic = _curated_problems(
                summary, result, inventory_root / "tools/verification/mutations.json"
            )
        elif surface == "generated":
            semantic = _generated_problems(summary, result, envelope)
            if root is not None:
                semantic.extend(_generated_structured_raw_problems(root, spec, summary, envelope))
        elif surface == "fuzz":
            semantic = _fuzz_problems(summary, result, manifest_payload)
        elif surface == "modelcheck":
            semantic = _model_problems(summary, result, manifest_payload, envelope)
        else:
            semantic = [f"unknown producer surface {surface!r}"]
        problems.extend(prefix + issue for issue in semantic)
    return problems


def enrich_gate_results(
    gate_results: list[dict[str, Any]],
    records: list[dict[str, Any]],
    *,
    derive_missing: bool = False,
) -> None:
    by_gate = {result.get("id"): result for result in gate_results if isinstance(result, dict)}
    for record in records:
        gate = by_gate.get(record.get("gate_id"))
        envelope = record.get("envelope")
        if not isinstance(envelope, dict):
            continue
        payload = envelope.get("payload")
        result = payload.get("result") if isinstance(payload, dict) else None
        if gate is None and derive_missing and isinstance(result, dict):
            gate = {
                "id": record.get("gate_id"),
                "recipe": record.get("gate_id"),
                "status": "PASS" if result.get("status") == "PASS" else "FAIL",
                "exit_code": result.get("exit_code"),
                "imported_producer": True,
            }
            gate_results.append(gate)
            by_gate[gate.get("id")] = gate
        if gate is None:
            continue
        envelope_identity = envelope.get("identity")
        gate.update(
            {
                "evidence_kind": "producer",
                "producer_registration": record.get("registration"),
                "evidence_digest": envelope.get("payload_sha256"),
                "derived_from": (
                    envelope_identity.get("path") if isinstance(envelope_identity, dict) else None
                ),
                "producer_started_at": result.get("started_at")
                if isinstance(result, dict)
                else None,
                "producer_finished_at": result.get("finished_at")
                if isinstance(result, dict)
                else None,
            }
        )

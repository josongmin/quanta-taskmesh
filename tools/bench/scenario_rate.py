"""Deterministically replay a finite arrival schedule at twice its intended rate.

The transformed scenario is a generator-only control. It repeats the original
schedule twice within the original injection window, retaining class/path/body,
topology and producer cap. It is not a measured host workload or an SLO result.
"""

from __future__ import annotations

from copy import deepcopy
from typing import Any


def doubled_generator_scenario(source: dict[str, Any]) -> dict[str, Any]:
    load = source.get("load")
    offers = source.get("offers")
    if not isinstance(load, dict) or not isinstance(offers, list) or not offers:
        raise ValueError("generator rate source lacks load or offers")
    injection_ms = load.get("injection_ms")
    max_records = load.get("max_records")
    if type(injection_ms) is not int or injection_ms <= 0:
        raise ValueError("generator rate source has invalid injection window")
    if type(max_records) is not int or 2 * len(offers) > 1_000_000:
        raise ValueError("doubled generator population exceeds record bound")
    source_id = source.get("id")
    if not isinstance(source_id, str) or len((source_id + "-genx2").encode()) > 128:
        raise ValueError("doubled generator scenario id is invalid")
    window_ns = injection_ms * 1_000_000
    period_ns = window_ns // 2
    doubled = deepcopy(source)
    doubled["id"] = source_id + "-genx2"
    doubled["load"]["max_records"] = max(max_records, 2 * len(offers))
    replay = []
    previous = -1
    for phase in (0, 1):
        for offer in offers:
            at = offer.get("send_time_ns") if isinstance(offer, dict) else None
            if type(at) is not int or at < 0 or at >= window_ns or at % 2:
                raise ValueError("source intended nanoseconds cannot be exactly doubled")
            if at < previous and phase == 0:
                raise ValueError("source intended schedule is unordered")
            previous = at if phase == 0 else previous
            copied = deepcopy(offer)
            copied["send_time_ns"] = at // 2 + phase * period_ns
            replay.append(copied)
    doubled["offers"] = replay
    return doubled

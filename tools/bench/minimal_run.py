#!/usr/bin/env python3
"""Acquire same-binary Taskmesh work with per-request recording minimized."""

from __future__ import annotations

from generator_run import main

if __name__ == "__main__":
    raise SystemExit(
        main(
            example_name="host_load_probe",
            raw_kind="minimal",
            runner_flag="--recorder-minimal",
        )
    )

#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 LibreType contributors
"""Collect only numeric native latency samples during a deliberate typing check."""
import argparse
import json
import math
from pathlib import Path
import time

FIELDS = (
    "event_to_capture_ms", "capture_to_request_ms", "request_to_worker_ms",
    "worker_ms", "worker_to_reply_ms", "reply_to_draw_ms", "event_to_draw_ms",
)


def extract(status, pid):
    if not isinstance(status, dict) or status.get("pid") != pid:
        raise ValueError("LibreType process changed or does not match --pid")
    sample = status.get("current", {}).get("last_typing_latency")
    if sample is None:
        return None
    if not isinstance(sample, dict) or set(sample) != {"sequence", *FIELDS}:
        raise ValueError("Unexpected latency sample schema")
    sequence = sample["sequence"]
    if type(sequence) is not int or sequence < 1:
        raise ValueError("Invalid sample sequence")
    for name in FIELDS:
        value = sample[name]
        if type(value) not in (int, float) or not math.isfinite(value) or value < 0:
            raise ValueError("Invalid latency duration")
    if not math.isclose(sum(sample[name] for name in FIELDS[:-1]), sample[FIELDS[-1]], abs_tol=0.001):
        raise ValueError("Latency phases do not match the total")
    # Whitelist the output; never copy the rest of the native diagnostic file.
    return {name: sample[name] for name in ("sequence", *FIELDS)}


def collect(status_path, pid, duration, count):
    initial = extract(json.loads(status_path.read_text()), pid)
    seen = initial["sequence"] if initial else 0
    samples = []
    deadline = time.monotonic() + duration
    while time.monotonic() < deadline and len(samples) < count:
        time.sleep(0.1)
        # The app atomically replaces the status file about once per second.
        sample = extract(json.loads(status_path.read_text()), pid)
        if sample and sample["sequence"] < seen:
            raise ValueError("Sample sequence moved backwards")
        if sample and sample["sequence"] > seen:
            samples.append(sample)
            seen = sample["sequence"]
            print(f"Captured timing sample {len(samples)}/{count}", flush=True)
    return samples


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", required=True, type=int)
    parser.add_argument("--seconds", type=float, default=120)
    parser.add_argument("--samples", type=int, default=3)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if args.pid < 1 or not 1 <= args.seconds <= 600 or not 1 <= args.samples <= 100:
        parser.error("Use a positive PID, 1–600 seconds, and 1–100 samples")
    status_path = Path.home() / "Library/Caches/LibreTypist/status.json"
    # Exclusive creation prevents replacing another report or following a symlink.
    with args.output.open("x") as output:
        samples = collect(status_path, args.pid, args.seconds, args.samples)
        json.dump({
            "pid": args.pid, "requested_samples": args.samples,
            "complete": len(samples) == args.samples, "samples": samples,
            "limits": [
                "Controlled ordinary-typing trials only; no automatic input is generated.",
                "Includes successful model completions with a matched capture and visible AppKit draw.",
                "Missing captures, canceled requests, manual activation and replacements are not samples.",
                "Starts at event-tap receipt; excludes earlier hardware/OS delivery and final display scanout.",
                "One-second diagnostic refresh can omit rapid samples; this is not a population latency benchmark.",
                "Stores durations and sequence numbers only; no writing, key values or app identities.",
            ],
        }, output, indent=2)
        output.write("\n")
    print(f"Saved {len(samples)} timing samples to {args.output}")


if __name__ == "__main__":
    main()

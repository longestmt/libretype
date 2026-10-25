#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 LibreTypist contributors
"""Measure a fixed synthetic production workload without reading writing or settings.

Build examples/completion_probe in release mode first. The caller selects the
installed app and Ollama server PIDs; only their process metadata is retained.
No model is downloaded, unloaded, selected in the app, or reconfigured.
"""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import statistics
import subprocess
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parent.parent


def cpu_seconds(value):
    total = 0.0
    for part in value.split(":"):
        total = total * 60 + float(part)
    return total


def processes():
    output = subprocess.check_output(
        ["ps", "-axo", "pid=,ppid=,rss=,time=,comm="], text=True
    )
    result = {}
    for line in output.splitlines():
        pid, parent, rss, cpu, command = line.split(None, 4)
        result[int(pid)] = {"parent": int(parent), "rss_kib": int(rss),
                            "cpu_seconds": cpu_seconds(cpu), "command": command}
    return result


def local_models():
    # Numeric loopback only; do not inherit proxy settings.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open("http://127.0.0.1:11434/api/ps", timeout=5) as response:
        models = json.load(response)["models"]
    return [{key: model.get(key) for key in
             ("name", "digest", "size", "size_vram", "context_length")}
            for model in models]


def sample(app_pid, server_pid, probe_pid, identities):
    rows = processes()
    for pid, command in identities.items():
        if pid not in rows or rows[pid]["command"] != command:
            raise RuntimeError("A selected process exited or changed during measurement")
    server_tree = {server_pid}
    while True:
        expanded = server_tree | {pid for pid, row in rows.items()
                                  if row["parent"] in server_tree}
        if expanded == server_tree:
            break
        server_tree = expanded
    selected = []
    for pid, row in rows.items():
        category = ("application" if pid == app_pid else "probe" if pid == probe_pid
                    else "ollama_server" if pid == server_pid
                    else "ollama_child" if pid in server_tree else None)
        if category:
            selected.append({"pid": pid, "category": category,
                             "rss_kib": row["rss_kib"], "cpu_seconds": row["cpu_seconds"]})
    return {"monotonic": time.monotonic(), "processes": selected}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", required=True)
    parser.add_argument("--app-pid", type=int, required=True)
    parser.add_argument("--server-pid", type=int, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("Output already exists; choose a new measurement file")
    probe = ROOT / "target/release/examples/completion_probe"
    if not probe.is_file():
        parser.error("First run cargo build --locked --release --example completion_probe")
    rows = processes()
    identities = {pid: rows[pid]["command"] for pid in (args.app_pid, args.server_pid)}
    if Path(identities[args.app_pid]).name != "libretypist":
        parser.error("App PID is not the LibreType executable")
    if Path(identities[args.server_pid]).name != "ollama":
        parser.error("Server PID is not the Ollama executable")
    cases = [
        {"id": "plain", "prefix": "Thank you for your "},
        {"id": "partial", "prefix": "Thanks for your flexib", "suffix": " with the schedule."},
        {"id": "midline", "prefix": "We should ", "suffix": "tomorrow."},
        {"id": "instructions", "prefix": "The new service is called ",
         "instructions": "Call the service Lumen Gateway. Do not abbreviate its name."},
    ]
    # Fixed rotated order makes repeated-context cache effects visible; it does
    # not turn these four contexts into twelve independent quality examples.
    workload = [{**case, "id": f"round{round_ + 1}_{case['id']}"}
                for round_ in range(3)
                for case in cases[round_:] + cases[:round_]]
    before_models = local_models()
    samples = [sample(args.app_pid, args.server_pid, None, identities)]
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="libretype-resource-") as temporary:
        case_file = Path(temporary) / "cases.json"
        case_file.write_text(json.dumps(workload))
        with (Path(temporary) / "results.jsonl").open("w+") as output, \
                (Path(temporary) / "stderr.txt").open("w+") as errors:
            process = subprocess.Popen([str(probe), args.model, str(case_file)],
                                       stdout=output, stderr=errors, cwd=ROOT)
            try:
                while process.poll() is None:
                    if time.monotonic() - started > 120:
                        raise TimeoutError("Synthetic workload exceeded 120 seconds")
                    samples.append(sample(args.app_pid, args.server_pid, process.pid, identities))
                    time.sleep(0.25)
                if process.returncode:
                    raise RuntimeError(f"Completion probe exited with code {process.returncode}")
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
            output.seek(0)
            results = [json.loads(line) for line in output if line.strip()]
    samples.append(sample(args.app_pid, args.server_pid, None, identities))
    if len(results) != len(workload) or any(row["error"] for row in results):
        raise RuntimeError("Incomplete workload or failed completion request")
    summary = []
    for pid in sorted({row["pid"] for snap in samples for row in snap["processes"]}):
        points = [row for snap in samples for row in snap["processes"] if row["pid"] == pid]
        summary.append({"pid": pid, "category": points[0]["category"],
                        "rss_min_kib": min(row["rss_kib"] for row in points),
                        "rss_max_kib": max(row["rss_kib"] for row in points),
                        "observed_cpu_seconds_delta": points[-1]["cpu_seconds"] - points[0]["cpu_seconds"]})
    report = {"observed_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "model": args.model, "probe_sha256": hashlib.sha256(probe.read_bytes()).hexdigest(),
              "elapsed_seconds": time.monotonic() - started,
              "median_request_ms": statistics.median(row["elapsed_ms"] for row in results),
              "models_before": before_models, "models_after": local_models(),
              "process_summary": summary, "results": results, "samples": samples,
              "limits": [
                  "Four synthetic contexts, three rotated rounds; no quality accuracy score.",
                  "No workload isolation or forced model unload; initial loaded state is recorded.",
                  "Request timing excludes debounce, native capture, display and user acceptance.",
                  "RSS is sampled, not peak physical allocation; shared/unified GPU memory can overlap.",
                  "Per-process CPU deltas omit unsampled startup/exit work and do not measure GPU energy.",
                  "No battery, system-wide footprint or comparative product performance claim.",
              ]}
    with args.output.open("x") as output:
        json.dump(report, output, indent=2)
        output.write("\n")
    print(json.dumps({key: report[key] for key in
                      ("model", "elapsed_seconds", "median_request_ms", "process_summary")}, indent=2))


if __name__ == "__main__":
    main()

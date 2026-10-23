#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 LibreTypist contributors
"""Opt-in live check: downloads ~0.8 GB to temporary storage, then removes it.

Uses the existing Ollama executable and production Rust download worker. Never
changes the app config, default model directory, or existing server process.
"""
import argparse
from contextlib import ExitStack
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.request
from model_fault_proxy import FaultProxy

ROOT = Path(__file__).resolve().parents[1]
RESEARCH = ROOT / "research"
CONFIG = Path.home() / "Library/Application Support/LibreTypist/config.json"
HTTP = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def request(base, path, body=None, timeout=10):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(base + path, data=data,
                                 headers={"Content-Type": "application/json"})
    with HTTP.open(req, timeout=timeout) as response:
        raw = response.read(1024 * 1024 + 1)
        assert len(raw) <= 1024 * 1024
        return json.loads(raw)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def inventory():
    models = request("http://127.0.0.1:11434", "/api/tags")["models"]
    return sorted((m["name"], m["digest"], m["size"]) for m in models)


def main(outage=False, stall=False):
    ollama = shutil.which("ollama")
    assert ollama, "Install Ollama before running this explicit live check"
    original_models = inventory()
    config_hash = digest(CONFIG.read_bytes())
    RESEARCH.mkdir(exist_ok=True)
    evidence = {"started_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
                "model": "gemma3:1b", "installed_app_rebuilt": False,
                "scenario": "network_stall" if stall else "network_outage" if outage else "cancellation"}
    prefix = "model-stall" if stall else "model-outage" if outage else "fresh-model-download"
    with ExitStack() as resources, tempfile.TemporaryDirectory(prefix="libretype-fresh-model-") as scratch:
        store = Path(scratch) / "models"
        store.mkdir(mode=0o700)
        with socket.socket() as reserve:
            reserve.bind(("127.0.0.1", 0))
            port = reserve.getsockname()[1]
        assert port != 11434
        base = f"http://127.0.0.1:{port}"
        env = dict(os.environ, OLLAMA_HOST=f"127.0.0.1:{port}",
                   OLLAMA_MODELS=str(store), OLLAMA_NO_CLOUD="1",
                   OLLAMA_KEEP_ALIVE="0", OLLAMA_NUM_PARALLEL="1")
        relay = resources.enter_context(FaultProxy()) if outage or stall else None
        if relay:
            env.update(HTTPS_PROXY=relay.url, https_proxy=relay.url,
                       NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost")
        with (Path(scratch) / "server.log").open("w") as server_log:
            server = subprocess.Popen([ollama, "serve"], env=env,
                                      stdout=server_log, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + 20
                while True:
                    assert server.poll() is None, "Isolated Ollama exited"
                    try:
                        version = request(base, "/api/version", timeout=1)
                        break
                    except (OSError, ValueError):
                        assert time.monotonic() < deadline, "Isolated server startup timed out"
                        time.sleep(0.1)
                owner = subprocess.check_output([
                    "/usr/sbin/lsof", "-nP", "-a", "-p", str(server.pid),
                    f"-iTCP:{port}", "-sTCP:LISTEN", "-Fn"], text=True)
                assert f"n127.0.0.1:{port}" in owner, "Test process must own the loopback listener"
                assert request(base, "/api/tags")["models"] == []
                assert not list(store.rglob("sha256-*")), "Fresh blob directory required"
                evidence.update(server_version=version, initial_model_count=0,
                                initial_blob_count=0, server_pid=server.pid)
                print(f"Fresh isolated store verified; download test running on loopback port {port}.", flush=True)
                started = time.monotonic()
                log_path = RESEARCH / f"{prefix}.log"
                test = "download_network_outage_and_retry_live" if outage or stall else "fresh_download_cancel_and_retry_live"
                test_env = dict(os.environ, LIBRETYPE_TEST_OLLAMA_BASE=base)
                if relay:
                    test_env.update(LIBRETYPE_TEST_FAULT_CONTROL=relay.url,
                                    LIBRETYPE_TEST_FAULT_TOKEN=relay.token,
                                    LIBRETYPE_TEST_FAULT_MODE="stall" if stall else "outage")
                with log_path.open("w") as log:
                    subprocess.run([
                        "cargo", "test", "--locked", "--lib",
                        f"model_setup::tests::{test}", "--",
                        "--ignored", "--exact", "--nocapture"], cwd=ROOT,
                        env=test_env,
                        stdout=log, stderr=subprocess.STDOUT, check=True, timeout=650)
                events = [json.loads(line) for line in log_path.read_text().splitlines()
                          if line.startswith('{"')]
                required = {"network_cut", "network_failed", "network_restored", "verified_local"} if outage or stall else {
                    "cancel_requested", "cancel_confirmed", "verified_local"}
                assert required <= {event["event"] for event in events}
                evidence.update(download_elapsed_seconds=time.monotonic() - started, events=events)
                if relay:
                    evidence["fault_relay"] = relay.snapshot()
                model = request(base, "/api/show", {"model": "gemma3:1b"})
                assert model["details"]["format"] == "gguf"
                assert not model.get("remote_host") and not model.get("remote_model")
                evidence["downloaded_models"] = [
                    {key: item[key] for key in ("name", "digest", "size")}
                    for item in request(base, "/api/tags")["models"]]
                generated = request(base, "/api/generate", {
                    "model": "gemma3:1b", "prompt": "Complete the phrase: Thank you for your",
                    "stream": False, "keep_alive": 0,
                    "options": {"temperature": 0, "num_ctx": 512, "num_predict": 12, "num_gpu": 0}}, timeout=90)
                assert generated.get("done") and generated.get("response", "").strip()
                evidence.update(local_format="gguf", synthetic_generation=generated["response"],
                                generated_token_count=generated.get("eval_count"),
                                stored_bytes=sum(p.stat().st_size for p in store.rglob('*') if p.is_file()))
            finally:
                if server.poll() is None:
                    server.terminate()
                    try:
                        server.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        server.kill()
                        server.wait(timeout=5)
                evidence["isolated_server_exit_code"] = server.returncode
        evidence["temporary_store_removed"] = True
    assert not Path(scratch).exists()
    assert inventory() == original_models
    assert digest(CONFIG.read_bytes()) == config_hash
    evidence.update(original_model_inventory_unchanged=True, config_sha256=config_hash,
                    config_unchanged=True,
                    original_inventory_sha256=digest(json.dumps(original_models).encode()))
    (RESEARCH / f"{prefix}-check.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps({"result": "pass", "download_elapsed_seconds": evidence["download_elapsed_seconds"],
                      "temporary_store_removed": True, "config_unchanged": True}), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--outage", action="store_true", help="interrupt the isolated server's download connection, wait for failure, then retry")
    parser.add_argument("--stall", action="store_true", help="pause encrypted traffic without closing relay sockets, wait for failure, then retry")
    args = parser.parse_args()
    assert not (args.outage and args.stall), "Choose one fault scenario"
    main(args.outage, args.stall)

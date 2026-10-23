# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 LibreTypist contributors
"""Temporary loopback CONNECT relay for an isolated Ollama outage check.

No TLS interception: encrypted bytes pass unchanged. Only the child Ollama
process receives this proxy setting; control requests require a random token.
"""
from contextlib import AbstractContextManager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import secrets
import select
import socket
import threading
import time


class FaultProxy(AbstractContextManager):
    def __init__(self):
        self.token = secrets.token_urlsafe(24)
        self.lock = threading.Lock()
        self.offline = False
        self.stalled = False
        self.stall_waits = 0
        self.sockets = set()
        self.accepted = 0
        self.blocked = 0
        self.cut = 0
        self.received = 0
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                if self.headers.get("X-LibreType-Test-Token") != owner.token:
                    self.send_error(403)
                    return
                if self.path not in ("/offline", "/online", "/stall"):
                    self.send_error(404)
                    return
                owner.set_offline(self.path == "/offline")
                with owner.lock:
                    owner.stalled = self.path == "/stall"
                body = json.dumps(owner.snapshot()).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_CONNECT(self):
                remote = None
                try:
                    host, port = self.path.rsplit(":", 1)
                    if port != "443" or not host or any(c in host for c in "/@[]"):
                        self.send_error(403)
                        return
                    with owner.lock:
                        if owner.offline:
                            owner.blocked += 1
                            self.send_error(503)
                            return
                    remote = socket.create_connection((host, 443), timeout=5)
                    self.connection.settimeout(5)
                    with owner.lock:
                        if owner.offline:
                            owner.blocked += 1
                            self.send_error(503)
                            return
                        owner.accepted += 1
                        owner.sockets.update((self.connection, remote))
                    self.send_response(200, "Connection Established")
                    self.end_headers()
                    while True:
                        with owner.lock:
                            if owner.offline:
                                return
                            stalled = owner.stalled
                            if stalled:
                                owner.stall_waits += 1
                        if stalled:
                            time.sleep(0.05)
                            continue
                        ready, _, _ = select.select((self.connection, remote), (), (), 1)
                        for source in ready:
                            chunk = source.recv(65536)
                            if not chunk:
                                return
                            destination = remote if source is self.connection else self.connection
                            destination.sendall(chunk)
                            if source is remote:
                                with owner.lock:
                                    owner.received += len(chunk)
                except (OSError, ValueError):
                    pass
                finally:
                    with owner.lock:
                        owner.sockets.discard(self.connection)
                        owner.sockets.discard(remote)
                    if remote is not None:
                        remote.close()
                    self.close_connection = True

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def snapshot(self):
        with self.lock:
            return {"offline": self.offline, "stalled": self.stalled,
                    "stall_waits": self.stall_waits, "accepted_connections": self.accepted,
                    "blocked_connections": self.blocked, "cut_sockets": self.cut,
                    "upstream_encrypted_bytes": self.received}

    def set_offline(self, offline):
        with self.lock:
            self.offline = offline
            sockets = list(self.sockets) if offline else []
            self.cut += len(sockets)
        for stream in sockets:
            try:
                stream.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            stream.close()

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.set_offline(True)
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)

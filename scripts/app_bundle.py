#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 LibreTypist contributors
"""Local LibreType packaging and reversible updates. Python 3.11+, macOS only."""
from __future__ import annotations

import argparse
import contextlib
import ctypes
import fcntl
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import tomllib
import uuid

ROOT = Path(__file__).resolve().parent.parent
APP_ID = "org.libretypist.app"
EXECUTABLE = "libretypist"
APP_NAME = "LibreType.app"
CODESIGN = "/usr/bin/codesign"


class UpdateError(RuntimeError):
    pass


def run(args, *, check=True):
    result = subprocess.run([str(a) for a in args], capture_output=True, text=True)
    if check and result.returncode:
        raise UpdateError(f"{Path(args[0]).name} failed: {(result.stderr or result.stdout).strip()}")
    return result


def real_directory(path: Path):
    if path.is_symlink() or not path.is_dir():
        raise UpdateError(f"Expected a real directory: {path}")


def metadata(app: Path):
    real_directory(app)
    for item in app.rglob("*"):
        if item.is_symlink() or not (item.is_dir() or item.is_file()):
            raise UpdateError(f"Unsupported bundle entry: {item}")
    try:
        info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
    except (OSError, ValueError, plistlib.InvalidFileException) as exc:
        raise UpdateError(f"Invalid app metadata: {app}") from exc
    if not isinstance(info, dict):
        raise UpdateError("App metadata must be a dictionary")
    expected = {"CFBundleIdentifier": APP_ID, "CFBundleExecutable": EXECUTABLE,
                "CFBundleName": "LibreType", "CFBundlePackageType": "APPL"}
    if any(info.get(k) != v for k, v in expected.items()):
        raise UpdateError("The bundle is not a LibreType app with the expected executable and identifier")
    executable = app / "Contents/MacOS" / EXECUTABLE
    if not executable.is_file() or not os.access(executable, os.X_OK):
        raise UpdateError("The app executable is missing or not executable")
    version = info.get("CFBundleShortVersionString", "")
    build = info.get("CFBundleVersion", "")
    if not isinstance(version, str) or not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise UpdateError("Expected a three-component release version")
    if not isinstance(build, str) or not re.fullmatch(r"[1-9]\d{0,3}", build):
        raise UpdateError("Expected a local build number from 1 to 9999")
    return info


def verify(app: Path):
    info = metadata(app)
    run([CODESIGN, "--verify", "--deep", "--strict", app])
    detail = run([CODESIGN, "--display", "--verbose=4", app])
    details = detail.stdout + detail.stderr
    if "Signature=adhoc" in details:
        kind = "ad-hoc"
    elif "Authority=" in details:
        kind = "certificate"
    else:
        raise UpdateError("Cannot identify this app's signing method")
    requirement = run([CODESIGN, "--display", "-r-", app])
    match = re.search(r"^(?:# )?designated => (.+)$", requirement.stdout + requirement.stderr, re.M)
    if not match:
        raise UpdateError("The app has no readable designated requirement")
    return {"version": info["CFBundleShortVersionString"], "build": info["CFBundleVersion"],
            "signing": kind, "requirement": match.group(1)}


def satisfies(app: Path, requirement: str) -> bool:
    return run([CODESIGN, "--verify", "--strict", "-R", "=" + requirement, app], check=False).returncode == 0


def inventory_digest(app: Path) -> str:
    digest = hashlib.sha256()
    for p in sorted(app.rglob("*")):
        relative = p.relative_to(app).as_posix().encode()
        digest.update(len(relative).to_bytes(8, "big") + relative)
        digest.update((p.stat().st_mode & 0o777).to_bytes(4, "big"))
        digest.update(b"d" if p.is_dir() else b"f")
        if p.is_file():
            digest.update(p.stat().st_size.to_bytes(8, "big"))
            with p.open("rb") as source:
                for block in iter(lambda: source.read(1024 * 1024), b""):
                    digest.update(block)
    return digest.hexdigest()


def inspect_update(source: Path, destination: Path):
    new = verify(source)
    old = verify(destination) if destination.exists() or destination.is_symlink() else None
    same_identity = old is not None and satisfies(source, old["requirement"])
    identical = old is not None and inventory_digest(source) == inventory_digest(destination)
    return {"source": str(source), "destination": str(destination),
            "new": {k: v for k, v in new.items() if k != "requirement"},
            "installed": {k: v for k, v in old.items() if k != "requirement"} if old else None,
            "same_signing_identity": same_identity, "identical": identical,
            "accessibility_renewal_may_be_needed": old is not None and not same_identity}


def authorize_update(report, allow_identity_change: bool):
    if report["installed"] and report["installed"]["signing"] == "certificate" and report["new"]["signing"] != "certificate":
        raise UpdateError("Refusing to replace a certificate-signed app with an ad-hoc build")
    if report["installed"] and not report["same_signing_identity"] and not allow_identity_change:
        raise UpdateError("Signing identity changed. Review 'check' output, then explicitly use --allow-identity-change for this migration")


@contextlib.contextmanager
def exclusive_lock(parent: Path, name: str):
    parent.mkdir(parents=True, exist_ok=True)
    real_directory(parent)
    flags = os.O_WRONLY | os.O_CREAT | os.O_NOFOLLOW | os.O_NONBLOCK
    fd = os.open(parent / name, flags, 0o600)
    try:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            raise UpdateError("Another build/update owns this location") from exc
        yield
    finally:
        os.close(fd)


def atomic_swap(first: Path, second: Path):
    libc = ctypes.CDLL("/usr/lib/libSystem.B.dylib", use_errno=True)
    swap = libc.renamex_np
    swap.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint]
    swap.restype = ctypes.c_int
    # SDK sys/stdio.h: RENAME_SWAP. No remove-then-rename fallback.
    if swap(os.fsencode(first), os.fsencode(second), 0x00000002):
        code = ctypes.get_errno()
        raise OSError(code, os.strerror(code), str(second))


def running_pids(destination: Path):
    """Check the kernel's executable path, not process names or command regexes."""
    lib = ctypes.CDLL("/usr/lib/libproc.dylib")
    query = lib.proc_pidpath
    query.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_uint32]
    query.restype = ctypes.c_int
    expected = (destination / "Contents/MacOS" / EXECUTABLE).resolve()
    pids = run(["/bin/ps", "-axo", "pid="]).stdout.split()
    matching = []
    for entry in pids:
        pid = int(entry)
        buffer = ctypes.create_string_buffer(4096)
        if query(pid, buffer, len(buffer)) > 0:
            actual = Path(os.fsdecode(buffer.value))
            try:
                # Resolves /var aliases and filesystem Unicode/case spelling,
                # while also requiring the exact app directory (not a hardlink
                # to the executable from an unrelated app).
                if actual.samefile(expected) and actual.parents[2].samefile(destination):
                    matching.append(pid)
            except (OSError, IndexError):
                pass
    return matching


def stop_app(destination: Path, requested: bool):
    pids = running_pids(destination)
    if pids and not requested:
        raise UpdateError("LibreType is running. Quit it, or use --quit to request graceful termination")
    for pid in pids:
        # Check again immediately before signaling; never kill a reused unrelated PID.
        if pid in running_pids(destination):
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
    deadline = time.monotonic() + 5
    while running_pids(destination):
        if time.monotonic() >= deadline:
            raise UpdateError("LibreType did not exit; update cancelled without replacing it")
        time.sleep(0.1)


def backup_path(parent: Path):
    directory = parent / ".LibreType-backups"
    directory.mkdir(exist_ok=True)
    real_directory(directory)
    slot = directory / (time.strftime("%Y%m%d-%H%M%S") + "-" + uuid.uuid4().hex[:8])
    slot.mkdir(mode=0o700)
    return slot / APP_NAME


def install(source: Path, destination: Path, *, allow_identity_change=False, quit_app=False):
    source = source.absolute()
    destination = destination.absolute()
    if destination.name != APP_NAME:
        raise UpdateError(f"The destination must be named {APP_NAME}")
    if source.resolve() == destination.resolve():
        raise UpdateError("Source and destination must be different paths")
    if source.resolve().is_relative_to(destination.resolve()) or destination.resolve().is_relative_to(source.resolve()):
        raise UpdateError("Source and destination cannot contain one another")
    with exclusive_lock(destination.parent, ".libretype-install.lock"):
        report = inspect_update(source, destination)
        authorize_update(report, allow_identity_change)
        if report["identical"]:
            return {**report, "result": "already installed"}
        # Stage and verify before asking the running app to exit.
        staging = Path(tempfile.mkdtemp(prefix=".libretype-update-", dir=destination.parent))
        candidate = staging / APP_NAME
        keep_staging = False
        try:
            shutil.copytree(source, candidate)
            copied = inspect_update(candidate, destination)
            authorize_update(copied, allow_identity_change)
            if inventory_digest(source) != inventory_digest(candidate):
                raise UpdateError("The source changed while it was being copied")
            stop_app(destination, quit_app)
            # A failed validation must leave the existing installation untouched.
            verify(candidate)
            backup = backup_path(destination.parent) if destination.exists() else None
            if backup:
                atomic_swap(candidate, destination)
                keep_staging = True  # The old app now lives here, including on a crash.
            else:
                candidate.rename(destination)
            try:
                verify(destination)
            except Exception:
                if backup:
                    atomic_swap(candidate, destination)
                    keep_staging = False
                else:
                    destination.rename(candidate)
                raise
            if backup:
                candidate.rename(backup)
                keep_staging = False
            return {**report, "result": "installed", "backup": str(backup) if backup else None}
        finally:
            if not keep_staging:
                shutil.rmtree(staging)
            else:
                print(f"Previous app retained for recovery at {candidate}", file=sys.stderr)


def build(identity: str | None, build_number: str | None):
    if identity is not None:
        if not re.fullmatch(r"[0-9A-Fa-f]{40}", identity):
            raise UpdateError("Signing identity must be the exact 40-character certificate SHA-1 fingerprint")
        valid = run(["/usr/bin/security", "find-identity", "-v", "-p", "codesigning"]).stdout
        if not re.search(r"\b" + re.escape(identity) + r"\b", valid, re.I):
            raise UpdateError("The requested code-signing identity is unavailable; no ad-hoc fallback was used")
    dist = ROOT / "dist"
    with exclusive_lock(dist, ".libretype-build.lock"):
        current = dist / APP_NAME
        if current.exists():
            old = verify(current)
            if old["signing"] == "certificate" and identity is None:
                raise UpdateError("The previous build used a certificate. Supply its signing identity explicitly")
        else:
            old = None
        number = build_number or str(int(old["build"]) + 1 if old else 1)
        if not re.fullmatch(r"[1-9]\d{0,3}", number):
            raise UpdateError("Build number must be 1–9999; supply --build-number when starting a new version series")
        # Cargo is invoked directly; no shell interpolation of identities or paths.
        subprocess.run(["cargo", "build", "--release", "--locked"], cwd=ROOT, check=True)
        staging = Path(tempfile.mkdtemp(prefix=".libretype-build-", dir=dist))
        app = staging / APP_NAME
        keep_staging = False
        try:
            (app / "Contents/MacOS").mkdir(parents=True)
            resources = app / "Contents/Resources"
            resources.mkdir()
            shutil.copy2(ROOT / "target/release" / EXECUTABLE, app / "Contents/MacOS" / EXECUTABLE)
            info = plistlib.loads((ROOT / "packaging/Info.plist").read_bytes())
            cargo = tomllib.loads((ROOT / "Cargo.toml").read_text())
            info["CFBundleShortVersionString"] = cargo["package"]["version"]
            info["CFBundleVersion"] = number
            (app / "Contents/Info.plist").write_bytes(plistlib.dumps(info))
            for dependency in ("gemoji", "curl"):
                for name in ("LICENSE", "SOURCE.md"):
                    shutil.copy2(ROOT / "vendor" / dependency / name,
                                 resources / (dependency + "-" + ("LICENSE.txt" if name == "LICENSE" else name)))
            # Retain application and transitive/runtime notices before signing.
            subprocess.run([sys.executable, str(ROOT / "scripts/license_inventory.py"), "--check"], cwd=ROOT, check=True)
            legal = resources / "Licenses"
            legal.mkdir()
            for name in ("LICENSE", "COPYRIGHT", "THIRD_PARTY_NOTICES.md"):
                shutil.copy2(ROOT / name, legal / name)
            shutil.copytree(ROOT / "third_party", legal / "third_party")
            shutil.copytree(ROOT / "docs", legal / "docs")
            for dependency in ("gemoji", "curl"):
                dest = legal / "vendor" / dependency
                dest.mkdir(parents=True)
                for name in ("LICENSE", "SOURCE.md"):
                    shutil.copy2(ROOT / "vendor" / dependency / name, dest / name)
            # Keep the local binding patch's provenance and both license texts.
            objc_legal = legal / "vendor" / "objc"
            objc_legal.mkdir(parents=True)
            for name in ("LICENSE.txt", "COPYING-GPL-3.0", "SOURCE.md"):
                shutil.copy2(ROOT / "vendor" / "objc" / name, objc_legal / name)
            args = [CODESIGN, "--force", "--sign", identity or "-", "--identifier", APP_ID]
            if identity:
                args += ["--options", "runtime", "--timestamp"]
            run([*args, app])
            new = verify(app)
            if identity and new["signing"] != "certificate":
                raise UpdateError("The produced signature is not certificate-based")
            if old and old["signing"] == "certificate" and not satisfies(app, old["requirement"]):
                raise UpdateError("The selected identity does not satisfy the previous build's signing requirement")
            backup = backup_path(dist) if current.exists() else None
            if backup:
                atomic_swap(app, current)
                keep_staging = True
                app.rename(backup)
                keep_staging = False
            else:
                app.rename(current)
            return {"app": str(current), "version": new["version"], "build": number,
                    "signing": new["signing"], "previous_build": str(backup) if backup else None}
        finally:
            if not keep_staging:
                shutil.rmtree(staging)
            else:
                print(f"Previous build retained at {app}", file=sys.stderr)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    b = sub.add_parser("build", help="Build and sign in staging, then publish the verified app")
    b.add_argument("--identity", default=os.environ.get("LIBRETYPE_SIGNING_IDENTITY"))
    b.add_argument("--build-number")
    for action in ("check", "install"):
        p = sub.add_parser(action)
        p.add_argument("source", type=Path, nargs="?", default=ROOT / "dist" / APP_NAME)
        p.add_argument("--destination", type=Path, default=Path.home() / "Applications" / APP_NAME)
        if action == "install":
            p.add_argument("--allow-identity-change", action="store_true")
            p.add_argument("--quit", action="store_true")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("This app workflow requires macOS")
    try:
        if args.command == "build":
            result = build(args.identity, args.build_number)
        elif args.command == "check":
            result = inspect_update(args.source.absolute(), args.destination.absolute())
        else:
            result = install(args.source, args.destination, allow_identity_change=args.allow_identity_change, quit_app=args.quit)
        print(json.dumps(result, indent=2))
    except (UpdateError, OSError, subprocess.CalledProcessError, ValueError) as exc:
        print(f"LibreType: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

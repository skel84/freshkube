#!/usr/bin/env python3
"""Start a packaged Freshkube with --fixture and wait for its window's first frame.

Checks the archive against its .sha256, extracts it, runs the executable with
FRESHKUBE_FIRST_FRAME=1 and waits for `first frame: window after N ms` on
stderr. Fails if the app exits, panics or stays silent until the timeout. On
Linux, run it under a display (`xvfb-run`).
"""

import argparse
import hashlib
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tempfile
import threading
import zipfile

FIRST_FRAME = re.compile(r"first frame: window after (\d+) ms")
# Rust's panic message, from any thread.
PANIC = re.compile(r"panicked at ")


def verify(archive):
    # Read as `sha256sum -c` does: one LF-ended line, two spaces, the exact name.
    expected, name = Path(f"{archive}.sha256").read_bytes().decode().removesuffix("\n").split("  ", 1)
    if name != archive.name:
        sys.exit(f"{archive.name}.sha256 names {name!r}, not {archive.name!r}")
    actual = hashlib.sha256(archive.read_bytes()).hexdigest()
    if actual != expected:
        sys.exit(f"{archive.name}: sha256 {actual}, expected {expected}")


def extract(archive, into):
    if archive.name.endswith(".tar.gz"):
        with tarfile.open(archive) as tar:
            tar.extractall(into, filter="data")
    else:
        with zipfile.ZipFile(archive) as zip_file:
            zip_file.extractall(into)
    [executable] = [p for p in into.glob("*/freshkube*") if p.name in ("freshkube", "freshkube.exe")]
    if os.name != "nt" and not os.access(executable, os.X_OK):
        sys.exit(f"{executable.name} isn't executable after extraction")
    return executable


def first_frame(executable, home, timeout):
    env = {
        **os.environ,
        "FRESHKUBE_FIRST_FRAME": "1",
        "RUST_BACKTRACE": "1",
        # Preferences and caches stay in the temporary directory on Linux;
        # Windows finds AppData through the shell, on a throwaway runner.
        "HOME": str(home),
        "XDG_CONFIG_HOME": str(home / "config"),
        "XDG_DATA_HOME": str(home / "data"),
        "XDG_CACHE_HOME": str(home / "cache"),
        "XDG_STATE_HOME": str(home / "state"),
    }
    env.pop("WAYLAND_DISPLAY", None)
    process = subprocess.Popen(
        [str(executable), "--fixture"], env=env, cwd=home,
        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
        text=True, errors="replace",
    )
    found = threading.Event()
    result = {}

    def read():
        for line in process.stderr:
            print(f"  | {line}", end="", flush=True)
            if found.is_set():
                continue
            if PANIC.search(line):
                result["panic"] = line.strip()
                found.set()
            elif match := FIRST_FRAME.search(line):
                result["ms"] = int(match.group(1))
                found.set()
        result.setdefault("closed", True)
        found.set()

    reader = threading.Thread(target=read, daemon=True)
    reader.start()
    found.wait(timeout)
    exited = process.poll()
    process.terminate()
    try:
        process.wait(10)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
    if "panic" in result:
        sys.exit(f"Freshkube panicked before its first frame: {result['panic']}")
    if "ms" in result:
        return result["ms"]
    if exited is not None:
        sys.exit(f"Freshkube exited with {exited} before its first frame")
    if "closed" in result:
        sys.exit("Freshkube closed its stderr before its first frame")
    sys.exit(f"No first frame within {timeout} s")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("archive", type=Path, help="Freshkube-<version>-<target>.tar.gz or .zip")
    parser.add_argument("--timeout", type=int, default=120, help="seconds to wait (default 120)")
    args = parser.parse_args()
    verify(args.archive)
    with tempfile.TemporaryDirectory(prefix="freshkube-first-frame-") as temporary:
        temporary = Path(temporary)
        (temporary / "home").mkdir()
        executable = extract(args.archive, temporary / "files")
        ms = first_frame(executable, temporary / "home", args.timeout)
    line = f"{args.archive.name}: first frame after {ms} ms with --fixture"
    print(line)
    if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(summary, "a") as out:
            out.write(f"- {line}\n")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Build and verify the Linux tarball or Windows zip; no signing or publishing."""

import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import re
import shutil
import struct
import subprocess
import tarfile
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
LINUX = "x86_64-unknown-linux-gnu"
WINDOWS = "x86_64-pc-windows-msvc"
TARGETS = {LINUX: "Linux", WINDOWS: "Windows"}
# The oldest glibc the tarball may need: Ubuntu 22.04's, which CI builds on.
GLIBC_FLOOR = (2, 35)
# The Ubuntu/Debian packages the tarball names. Every linked library must
# come from one of them.
RUNTIME_PACKAGES = ROOT / "scripts/linux-runtime-packages.txt"
# DLLs a Windows install has. The C runtime is linked in (crt-static), so a
# Visual C++ redistributable or UCRT import means the build went wrong.
WINDOWS_RUNTIME = re.compile(r"(vcruntime|msvcp|ucrtbase|api-ms-win-crt-).*", re.I)
WINDOWS_GUI = 2  # IMAGE_SUBSYSTEM_WINDOWS_GUI


def run(*args, capture=False, env=None):
    return subprocess.run(
        args, cwd=ROOT, env=env, check=True, text=True,
        stdout=subprocess.PIPE if capture else None,
    ).stdout


def runtime_packages():
    return {line.strip() for line in RUNTIME_PACKAGES.read_text().splitlines()
            if line.strip() and not line.startswith("#")}


def owning_package(library):
    """The installed package that provides a shared library, by dpkg."""
    # Only the x86_64 multiarch directory: a 32-bit copy (lib32gcc-s1's
    # /usr/lib32/libgcc_s.so.1) has the same name.
    found = subprocess.run(["dpkg-query", "-S", f"*/x86_64-linux-gnu/{library}"], text=True,
                           stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    # Lines read "libc6:amd64, libc6:i386: /path"; skip "diversion by" lines.
    owners = {owner.split(":")[0]
              for line in found.stdout.splitlines() if ": /" in line and not line.startswith("diversion")
              for owner in line.split(": /", 1)[0].split(", ")}
    if found.returncode != 0 or len(owners) != 1:
        raise ValueError(f"Can't tell which package provides {library}: {sorted(owners)}")
    return owners.pop()


def linux_details(binary):
    needed = re.findall(r"\(NEEDED\)\s+Shared library: \[([^\]]+)\]",
                        run("readelf", "-d", str(binary), capture=True))
    if not needed:
        raise ValueError("readelf found no linked libraries; its output may have changed")
    if shutil.which("dpkg-query") is None:
        raise ValueError("Checking linked libraries needs dpkg; package on Ubuntu 22.04")
    listed = runtime_packages()
    unlisted = {lib: package for lib in needed
                if (package := owning_package(lib)) not in listed}
    if unlisted:
        raise ValueError(f"Linked libraries from packages {RUNTIME_PACKAGES.name} "
                         f"doesn't list: {unlisted}")
    versions = re.findall(r"GLIBC_(\d+)\.(\d+)", run("readelf", "-V", str(binary), capture=True))
    glibc = max((int(a), int(b)) for a, b in versions)
    if glibc > GLIBC_FLOOR:
        raise ValueError(f"Needs glibc {glibc[0]}.{glibc[1]}, newer than "
                         f"{GLIBC_FLOOR[0]}.{GLIBC_FLOOR[1]}; build on Ubuntu 22.04")
    return {"libraries": needed, "glibc_min": f"{glibc[0]}.{glibc[1]}"}


def pe_imports(data):
    """The subsystem and imported DLL names of a PE32+ executable."""
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("Not a PE executable")
    sections, optional_size = struct.unpack_from("<2xH12xH", data, pe + 4)
    optional = pe + 24
    if struct.unpack_from("<H", data, optional)[0] != 0x20B:
        raise ValueError("Not a 64-bit (PE32+) executable")
    subsystem = struct.unpack_from("<H", data, optional + 68)[0]
    # Each section header: virtual size, virtual address and file offset.
    table = [struct.unpack_from("<8xII4xI", data, optional + optional_size + 40 * i)
             for i in range(sections)]

    def offset(rva):
        for size, address, raw in table:
            if address <= rva < address + size:
                return rva - address + raw
        raise ValueError(f"RVA {rva:#x} is in no section")

    def name(rva):
        start = offset(rva)
        return data[start:data.index(b"\0", start)].decode("ascii")

    names = []
    # Data directories 1 (imports) and 13 (delay-loaded imports).
    for index, entry_size, name_at in ((1, 20, 12), (13, 32, 4)):
        rva = struct.unpack_from("<I", data, optional + 112 + 8 * index)[0]
        if not rva:
            continue
        at = offset(rva)
        while (dll := struct.unpack_from("<I", data, at + name_at)[0]):
            names.append(name(dll))
            at += entry_size
    return subsystem, names


def windows_details(binary):
    subsystem, imports = pe_imports(binary.read_bytes())
    if subsystem != WINDOWS_GUI:
        raise ValueError(f"Subsystem {subsystem}; a release build must be a GUI program")
    runtime = [dll for dll in imports if WINDOWS_RUNTIME.fullmatch(dll)]
    if runtime:
        raise ValueError(f"Imports the C runtime; build with crt-static: {runtime}")
    system = Path(os.environ.get("SystemRoot", r"C:\Windows")) / "System32"
    missing = [dll for dll in imports
               if not dll.lower().startswith("api-ms-win-") and not (system / dll).exists()]
    if missing:
        raise ValueError(f"Imports DLLs Windows doesn't have: {missing}")
    return {"libraries": imports, "subsystem": "gui"}


def stage(directory, binary, target):
    """The files users get: the executable, licences and notices."""
    directory.mkdir()
    executable = directory / binary.name
    shutil.copy2(binary, executable)
    executable.chmod(0o755)
    for name in ("LICENSE", "NOTICE"):
        shutil.copy2(ROOT / name, directory / name)
    shutil.copytree(ROOT / "licenses", directory / "licenses")
    # Keep the licence paths NOTICE names. Fonts, themes and icons are also
    # embedded in the executable.
    shutil.copytree(ROOT / "crates/freshkube-ui/assets", directory / "crates/freshkube-ui/assets")
    if target == LINUX:
        shutil.copy2(ROOT / "scripts/linux-runtime-packages.txt", directory / "RUNTIME-PACKAGES.txt")
        (directory / "freshkube.desktop").write_text(
            "[Desktop Entry]\nType=Application\nName=Freshkube\n"
            "Comment=Talos Linux and Kubernetes clusters\nExec=freshkube\n"
            "Terminal=false\nCategories=Development;\n",
            newline="\n",
        )


def files(directory, target):
    """Each file's hash and, in the tarball, whether it is executable."""
    return {p.relative_to(directory).as_posix(): (hashlib.sha256(p.read_bytes()).hexdigest(),
                                                  target == LINUX and p.stat().st_mode & 0o111 != 0)
            for p in directory.rglob("*") if p.is_file()}


def archive_directory(directory, archive, target):
    if target == LINUX:
        # gzip writes the time into its header; zero it so equal files give equal bytes.
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w") as tar:
            for path in sorted([directory, *directory.rglob("*")]):
                info = tar.gettarinfo(path, path.relative_to(directory.parent).as_posix())
                info.uid = info.gid = 0
                info.uname = info.gname = ""
                info.mtime = 0
                if info.isfile():
                    with open(path, "rb") as source:
                        tar.addfile(info, source)
                else:
                    tar.addfile(info)
        archive.write_bytes(gzip.compress(buffer.getvalue(), mtime=0))
    else:
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zip_file:
            for path in sorted(directory.rglob("*")):
                if path.is_file():
                    zip_file.write(path, path.relative_to(directory.parent).as_posix())


def extract(archive, into, target):
    if target == LINUX:
        with tarfile.open(archive) as tar:
            tar.extractall(into, filter="data")
    else:
        with zipfile.ZipFile(archive) as zip_file:
            zip_file.extractall(into)


def package(args):
    metadata = json.loads(run(
        "cargo", "metadata", "--no-deps", "--locked", "--format-version=1", capture=True,
    ))
    package_info = next(p for p in metadata["packages"] if p["name"] == "freshkube")
    version = package_info["version"]
    executable = "freshkube.exe" if args.target == WINDOWS else "freshkube"
    binary = Path(metadata["target_directory"]) / args.target / "release" / executable
    if not args.no_build:
        env = dict(os.environ)
        if args.target == WINDOWS:
            # No Visual C++ redistributable needed on a clean install.
            env["RUSTFLAGS"] = (env.get("RUSTFLAGS", "") + " -C target-feature=+crt-static").strip()
        run("cargo", "build", "--release", "--locked", "--bin", "freshkube",
            "--target", args.target, env=env)
    details = linux_details(binary) if args.target == LINUX else windows_details(binary)

    output = ROOT / "target/packages" / args.target
    output.mkdir(parents=True, exist_ok=True)
    basename = f"Freshkube-{version}-{args.target}"
    archive = output / (basename + (".tar.gz" if args.target == LINUX else ".zip"))
    with tempfile.TemporaryDirectory(prefix=".package-", dir=output) as staging:
        staging = Path(staging)
        directory = staging / basename
        stage(directory, binary, args.target)
        staged = staging / archive.name
        archive_directory(directory, staged, args.target)
        # Verify the downloadable bytes, executable bit included.
        extracted = staging / "extracted"
        extract(staged, extracted, args.target)
        if files(directory, args.target) != files(extracted / basename, args.target):
            raise ValueError("Archive round trip changed the files")
        staged.replace(archive)

    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    # LF on Windows too: `sha256sum -c` reads a CR as part of the file name.
    Path(f"{archive}.sha256").write_text(f"{checksum}  {archive.name}\n", newline="\n")
    manifest = {
        "version": version, "target": args.target, "signing": "none",
        "source_commit": run("git", "rev-parse", "HEAD", capture=True).strip(),
        "source_dirty": bool(run("git", "status", "--porcelain", capture=True).strip()),
        "repository": package_info["repository"],
        "archive": archive.name, "sha256": checksum, **details,
    }
    (output / f"{basename}.json").write_text(json.dumps(manifest, indent=2) + "\n", newline="\n")
    print(f"Packaged {archive}\nNot signed.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=TARGETS)
    parser.add_argument("--no-build", action="store_true", help="Package an existing release build")
    args = parser.parse_args()
    if platform.system() != TARGETS[args.target]:
        parser.error(f"Package {args.target} on {TARGETS[args.target]}")
    package(args)


if __name__ == "__main__":
    main()

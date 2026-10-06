#!/usr/bin/env python3
"""Build and verify an ad-hoc signed Freshkube.app; no Apple secrets or publishing."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
BUNDLE_ID = "io.github.skel84.freshkube"
MINIMUM_MACOS = "15.0"
# Made from packaging/icon/freshkube.svg by scripts/app-icon.sh.
ICON = ROOT / "packaging/icon/freshkube.icns"
ICON_FILE = "Freshkube.icns"
ARCHITECTURES = {
    "aarch64-apple-darwin": "arm64",
    "x86_64-apple-darwin": "x86_64",
}


def run(*args, capture=False, env=None):
    return subprocess.run(
        args, cwd=ROOT, env=env, check=True, text=True,
        stdout=subprocess.PIPE if capture else None,
    ).stdout


def inspect_binary(binary, target):
    arch = run("lipo", "-archs", str(binary), capture=True).strip()
    if arch != ARCHITECTURES[target]:
        raise ValueError(f"Expected {ARCHITECTURES[target]} executable, found {arch}")
    load_commands = run("otool", "-l", str(binary), capture=True)
    uuid = re.search(r"\buuid ([0-9A-Fa-f-]+)", load_commands)
    if uuid is None:
        raise ValueError("Executable needs LC_UUID for macOS Local Network privacy")
    minimum = re.search(r"cmd LC_BUILD_VERSION\s+cmdsize \d+\s+platform \d+\s+minos (\d+\.\d+(?:\.\d+)?)", load_commands)
    if minimum is None or minimum.group(1) not in (MINIMUM_MACOS, MINIMUM_MACOS + ".0"):
        raise ValueError("Executable must be built with MACOSX_DEPLOYMENT_TARGET=15.0")
    # This bundle has no private dylibs. Fail rather than ship a Homebrew link.
    linkage = run("otool", "-L", str(binary), capture=True)
    libraries = [line.strip().split(" (", 1)[0] for line in linkage.splitlines()[1:]]
    external = [lib for lib in libraries if not lib.startswith(("/usr/lib/", "/System/Library/"))]
    if external:
        raise ValueError(f"Unbundled runtime libraries: {external}")
    return {"architecture": arch, "uuid": uuid.group(1), "libraries": libraries}


def bundle_files(app):
    return {p.relative_to(app): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in app.rglob("*") if p.is_file()}


MACH_O = {
    bytes.fromhex(magic)
    for magic in ("feedface", "feedfacf", "cefaedfe", "cffaedfe", "cafebabe", "bebafeca")
}


def stray_executables(app):
    """Executables in the bundle other than the app's own binary.

    Developer tools such as `freshkube-workbench` must never ship, so any
    other file that is executable or Mach-O fails the bundle."""
    own = app / "Contents/MacOS/freshkube"
    stray = []
    for path in sorted(app.rglob("*")):
        if path == own or path.is_symlink() or not path.is_file():
            continue
        with path.open("rb") as file:
            magic = file.read(4)
        if path.stat().st_mode & 0o111 or magic in MACH_O:
            stray.append(path.relative_to(app))
    return stray


def verify_bundle(app, target, version):
    run("plutil", "-lint", str(app / "Contents/Info.plist"))
    info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
    icon = app / "Contents/Resources" / info.get("CFBundleIconFile", "")
    if not icon.is_file() or icon.read_bytes()[:4] != b"icns":
        raise ValueError(f"CFBundleIconFile does not name an icon in Resources: {icon}")
    run("codesign", "--verify", "--deep", "--strict", "--verbose=2", str(app))
    stray = stray_executables(app)
    if stray:
        raise ValueError(f"Bundle holds executables other than freshkube: {stray}")
    binary = app / "Contents/MacOS/freshkube"
    details = inspect_binary(binary, target)
    # CLI smoke test on native builds only; never opens a cluster connection.
    host = run("rustc", "-vV", capture=True)
    if f"host: {target}\n" in host:
        actual = run(str(binary), "--version", capture=True).strip()
        if actual != f"freshkube {version}":
            raise ValueError(f"Incorrect executable version: {actual}")
    return details


def package(args):
    metadata = json.loads(run(
        "cargo", "metadata", "--no-deps", "--locked", "--format-version=1", capture=True,
    ))
    package_info = next(p for p in metadata["packages"] if p["name"] == "freshkube")
    version = package_info["version"]
    # Apple's marketing version has three integers; preserve SemVer in the manifest.
    short_version = version.split("-", 1)[0].split("+", 1)[0]
    build_number = args.build_number or short_version
    if not re.fullmatch(r"\d+(?:\.\d+){0,2}", build_number):
        raise ValueError("Build number must have one to three integer components")
    binary = Path(metadata["target_directory"]) / args.target / "release/freshkube"
    if not args.no_build:
        env = {**os.environ, "MACOSX_DEPLOYMENT_TARGET": MINIMUM_MACOS}
        run("cargo", "build", "--release", "--locked", "--bin", "freshkube",
            "--target", args.target, env=env)
    inspect_binary(binary, args.target)

    output = ROOT / "target/macos" / args.target
    output.mkdir(parents=True, exist_ok=True)
    basename = f"Freshkube-{version}-{args.target}-adhoc"
    archive = output / f"{basename}.zip"
    with tempfile.TemporaryDirectory(prefix=".package-", dir=output) as staging:
        stage = Path(staging)
        app = stage / "Freshkube.app"
        contents = app / "Contents"
        (contents / "MacOS").mkdir(parents=True)
        resources = contents / "Resources"
        resources.mkdir()
        shutil.copy2(binary, contents / "MacOS/freshkube")
        (contents / "MacOS/freshkube").chmod(0o755)
        info = {
            "CFBundleDevelopmentRegion": "en",
            "CFBundleDisplayName": "Freshkube",
            "CFBundleName": "Freshkube",
            "CFBundleExecutable": "freshkube",
            "CFBundleIconFile": ICON_FILE,
            "CFBundleIdentifier": BUNDLE_ID,
            "CFBundleInfoDictionaryVersion": "6.0",
            "CFBundlePackageType": "APPL",
            "CFBundleShortVersionString": short_version,
            "CFBundleVersion": build_number,
            "LSMinimumSystemVersion": MINIMUM_MACOS,
            "LSApplicationCategoryType": "public.app-category.developer-tools",
            "NSHighResolutionCapable": True,
            "NSSupportsAutomaticGraphicsSwitching": True,
            "NSHumanReadableCopyright": next(
                line for line in (ROOT / "LICENSE").read_text().splitlines()
                if line.startswith("Copyright")
            ),
            "NSLocalNetworkUsageDescription": (
                "Freshkube connects to the Talos Linux and Kubernetes clusters "
                "you choose on your local network to display their state and manage them."
            ),
        }
        (contents / "Info.plist").write_bytes(plistlib.dumps(info))
        for name in ("LICENSE", "NOTICE"):
            shutil.copy2(ROOT / name, resources / name)
        shutil.copy2(ICON, resources / ICON_FILE)
        shutil.copytree(ROOT / "licenses", resources / "licenses")
        # Preserve the licence paths referenced by NOTICE. Fonts, themes, icons
        # and Metal shaders are also embedded in the executable by Rust/GPUI.
        shutil.copytree(ROOT / "crates/freshkube-ui/assets",
                        resources / "crates/freshkube-ui/assets")
        run("codesign", "--force", "--sign", "-", "--identifier", BUNDLE_ID, str(app))
        details = verify_bundle(app, args.target, version)
        staged_zip = stage / archive.name
        run("ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(app), str(staged_zip))
        # Verify the downloadable bytes, including signature and executable mode.
        extracted = stage / "extracted"
        run("ditto", "-x", "-k", str(staged_zip), str(extracted))
        verify_bundle(extracted / "Freshkube.app", args.target, version)
        if bundle_files(app) != bundle_files(extracted / "Freshkube.app"):
            raise ValueError("Archive round trip changed bundle contents")
        destination = output / "Freshkube.app"
        if destination.exists():
            shutil.rmtree(destination)
        shutil.move(str(app), destination)
        staged_zip.replace(archive)

    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix(".zip.sha256").write_text(f"{checksum}  {archive.name}\n")
    manifest = {
        "version": version, "build_number": build_number, "target": args.target,
        "bundle_identifier": BUNDLE_ID, "minimum_macos": MINIMUM_MACOS,
        "signing": "ad-hoc", "notarized": False,
        "source_commit": run("git", "rev-parse", "HEAD", capture=True).strip(),
        "source_dirty": bool(run("git", "status", "--porcelain", capture=True).strip()),
        "repository": package_info["repository"],
        "archive": archive.name, "sha256": checksum, **details,
    }
    archive.with_suffix(".json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"Packaged {archive}\nAd-hoc signed; not notarized.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=ARCHITECTURES)
    parser.add_argument("--build-number", help="Numeric CFBundleVersion (defaults to Cargo version)")
    parser.add_argument("--no-build", action="store_true", help="Package an existing release build")
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("Packaging requires macOS and Xcode command line tools")
    package(args)


if __name__ == "__main__":
    main()

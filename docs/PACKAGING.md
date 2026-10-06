# Linux and Windows packaging

Freshkube builds a Linux tarball and a Windows zip from every push to `main`.
Both are **preview**: CI builds them, checks the archive and opens a window
from it, but nobody has yet run them on a real desktop. They are unsigned. macOS
packaging, signing and releases are in [MACOS_PACKAGING.md](MACOS_PACKAGING.md).

| Target | Archive | Built on |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu` | `Freshkube-<version>-x86_64-unknown-linux-gnu.tar.gz` | Ubuntu 22.04: glibc 2.35 or later |
| `x86_64-pc-windows-msvc` | `Freshkube-<version>-x86_64-pc-windows-msvc.zip` | Windows Server (`windows-latest`): Windows 10 or later |

ARM builds for Linux and Windows are out of scope until signing and
notarization (ROADMAP step 8).

## What an archive holds

One directory, `Freshkube-<version>-<target>/`, with:
- the executable (`freshkube`, mode 0755, or `freshkube.exe`);
- `LICENSE`, `NOTICE` and `licenses/`, plus the font notices under
  `crates/freshkube-ui/assets/` at the paths `NOTICE` names.

Fonts, themes and icons are embedded in the executable. The Linux tarball adds:
- `RUNTIME-PACKAGES.txt`, the shared libraries it needs as Ubuntu/Debian package names;
- a `freshkube.desktop` entry that runs `freshkube` from `PATH`.

Next to each archive are a `.sha256` and a `.json` manifest with:
- the version, target and source commit (clean);
- the archive's checksum;
- the libraries the executable links;
- on Linux, the newest glibc symbol version it uses.

## Build locally

On the platform itself, with Rust, Python 3 and `protoc`, plus GPUI's build
dependencies on Linux (the list in `.github/workflows/platforms.yml`):

```sh
python3 scripts/package-portable.py --target x86_64-unknown-linux-gnu
python scripts/package-portable.py --target x86_64-pc-windows-msvc
```

The script builds `--release --locked`, stages the files, archives them,
extracts the archive again and compares every file. Output goes to
`target/packages/<target>/`. It fails on any of the following:
- **Linux:** no linked libraries found, a linked library whose package (by
  `dpkg-query -S`) isn't in `scripts/linux-runtime-packages.txt`, or a glibc
  symbol newer than 2.35. So it packages only on Debian or Ubuntu.
- **Windows:** an executable that isn't a GUI program, imports the C runtime, or
  imports a DLL that isn't in `System32`.

The tarball is reproducible: gzip and tar times are zeroed, owners are blank,
and files are added in sorted order.

### Windows specifics

- **Static C runtime.** The Windows build links the C runtime statically
  (`-C target-feature=+crt-static`), so a clean install needs no Visual C++
  redistributable.
- **No console window.** A release `freshkube.exe` is a GUI program, so opening
  it from Explorer shows no console.
- **Output in a terminal.** Started from a terminal, it attaches to that
  terminal's console (`AttachConsole`), so `--help` and errors appear there. A
  redirected stderr (a pipe or a file) keeps the redirection. cmd and PowerShell
  don't wait for a GUI program, so that output lands after the next prompt, and
  an error at startup from Explorer isn't shown anywhere.
- **Debug builds** stay console programs.

## CI

`packages.yml` runs:
- on pushes to `main` that change more than `spikes/`, `docs/` or Markdown;
- on pull requests that change the workflow or its scripts;
- by hand (**Run workflow**) on any branch.

- **`package`** builds each archive natively, without a Rust cache. A release
  build's cache would push the macOS check cache, which gates merges, out of the
  repository's 10 GB budget. It uploads `freshkube-<target>` for 90 days, like
  the macOS bundles.
- **`first-frame-linux` and `first-frame-windows`** run on machines that didn't
  build the archive. They download it, check its `.sha256`, extract it and run
  `scripts/first-frame.py`. That script starts `freshkube --fixture` with
  `FRESHKUBE_FIRST_FRAME=1` and waits up to 120 s for
  `first frame: window after N ms` on stderr. An exit, a panic in any thread,
  closed output or silence fails the job, and the time goes in the job summary.
  - **Linux** runs in a bare `ubuntu:22.04` container. It installs
    `linux-runtime-packages.txt` alone and fails if `ldd` finds a linked library
    missing; then it adds Xvfb and Mesa's software Vulkan and OpenGL, which
    bring libraries of their own, and starts the app. A library loaded at run
    time but left out of the list can still be hidden by Mesa's.
  - **Windows** draws through Direct3D 11's software renderer (WARP).

This workflow is apart from `ci.yml`, so a failure here never stops a macOS
release while these platforms are preview.

## Releases

A `v*` tag's [Release](../.github/workflows/release.yml) workflow, after
verifying the macOS bundles ([MACOS_PACKAGING.md](MACOS_PACKAGING.md#releases)):
1. Finds the finished `packages.yml` run on `main` for the tagged commit.
2. For each platform whose package and first-frame jobs both passed, downloads
   its build and checks the archive against its `.sha256` and the manifest
   (`scripts/release-manifest.py`) against the tag.
3. Attaches each verified build to the draft, with a preview install section in
   the notes.

While a platform is preview, a run still going or never started, a failed
job, a failed or expired download, a mismatch or an error from GitHub's API
leaves that platform out. A build is downloaded apart and joins the draft only
once every check has passed. It adds a warning at the top of the
notes and in the run's annotations, and the macOS release goes ahead. A
platform turns supported, and its build required, with its first
[first-run report](FIRST_RUN.md).

## Install a preview build

**Linux:**

```sh
sha256sum -c Freshkube-<version>-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf Freshkube-<version>-x86_64-unknown-linux-gnu.tar.gz
sudo apt-get install $(grep -v '^#' Freshkube-<version>-x86_64-unknown-linux-gnu/RUNTIME-PACKAGES.txt)
./Freshkube-<version>-x86_64-unknown-linux-gnu/freshkube
```

GPUI draws on X11 or Wayland through Vulkan or OpenGL. Remembered keys go to
the Secret Service (GNOME Keyring, KWallet).

**Windows:**
- Check the zip with `Get-FileHash -Algorithm SHA256`, extract it, and run `freshkube.exe`.
- The executable isn't signed, so SmartScreen may stop the first start: choose
  **More info → Run anyway**.
- Remembered keys go to Credential Manager.

## Before a platform is supported

A tester's first-run report on that platform is recorded in
[FIRST_RUN.md](FIRST_RUN.md) before its build is called supported:
- start, `--fixture`, one real context read-only, logs and a shell;
- no names, hosts or cluster details from the tester's environment.

Until then, release notes mark it preview.

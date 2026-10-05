# macOS application packaging

Freshkube packages as two separate `Freshkube.app` bundles for macOS 15 or
later. The pipeline ad-hoc signs them without Apple credentials, so they are
**not notarized distributables**. A version tag drafts a GitHub pre-release from
the bundles `main` already built; a person publishes it
([Releases](#releases)).

## Build locally

Use macOS, Rust/Cargo, Python 3, Xcode command line tools and `protoc`
(`brew install protobuf`). GPUI Kit enables `runtime_shaders`: Metal shader
source is embedded and compiled at runtime using macOS Metal. Fonts, themes and
icons are embedded too. The bundle carries the checked-in assets, `LICENSE`,
`NOTICE`, Apache-2.0 and Lucide/Feather texts, and both font OFL notices under
`Contents/Resources`. There is no external asset directory to install.

Talos protobuf Rust code is generated from the checked-in schemas into Cargo's
`OUT_DIR` for each build, including release and target-specific builds. Set
`PROTOC` to an absolute compiler path if necessary. Generated code and `protoc`
are not runtime bundle resources.

On the machine matching the target architecture:

```sh
# Apple Silicon
python3 scripts/package-macos.py --target aarch64-apple-darwin
# Intel
python3 scripts/package-macos.py --target x86_64-apple-darwin
```

The script runs `cargo build --release --locked --bin freshkube --target <target>`
with `MACOSX_DEPLOYMENT_TARGET=15.0`, reads the version from Cargo metadata and
creates `target/macos/<target>/Freshkube.app`. It also creates
`Freshkube-<version>-<target>-adhoc.zip`, a `.zip.sha256` checksum and a JSON
manifest with source revision, dirty state, signing status, executable UUID and
linked libraries. `--build-number <integer>` sets `CFBundleVersion`; CI uses its
run number. The default is the numeric part of the Cargo version.

To package a build already made with the correct deployment target:

```sh
MACOSX_DEPLOYMENT_TARGET=15.0 cargo build --release --locked --bin freshkube --target x86_64-apple-darwin
python3 scripts/package-macos.py --target x86_64-apple-darwin --no-build
```

The script checks the Mach-O architecture, minimum OS, UUID and dynamic links,
lints `Info.plist`, verifies the bundle signature, checks `--version` on a native
host, then extracts and verifies the actual ZIP and compares every bundled file.
It rejects non-system dynamic libraries rather than quietly depending on a
Homebrew installation. Cross builds need the Rust target installed and a suitable
SDK; their CLI smoke test is skipped. CI uses native hosts for both architectures.
The app currently uses the system's generic application icon.

## CI

[CI](../.github/workflows/ci.yml) runs on pull requests against `main`, on pushes
to `main` and by hand:

| Event | Checks (fmt, Clippy, tests) | Both app bundles |
| --- | --- | --- |
| Pull request | Yes | No |
| Push to `main` (a merge) | Yes | Yes, kept 90 days |
| Run by hand on a branch | Yes | Yes |

The checks are formatting, Clippy with warnings denied and the workspace tests,
including headless UI tests. A change that touches only `spikes/`, `docs/` or
Markdown files skips them; the skipped job counts as passed, so it never blocks
a merge. A draft pull request skips them as well, and they run when it is marked
ready for review, so work in progress doesn't hold the macOS runners. A spike has
its own workspace and tests that CI doesn't run. A new push to a pull request
cancels its previous run. The checks build the workspace crates with line tables
only, rather than full debug info, which keeps backtraces readable and the test
binaries quicker to link.

Changelog fragments have a separate Ubuntu job on every run, including
Markdown-only changes and draft PRs. It runs `scripts/changelog.test.sh` and
`scripts/changelog.sh check` without a Rust build or a macOS runner.

The separate [Platform checks workflow](../.github/workflows/platforms.yml)
runs on native `ubuntu-latest` and `windows-latest` runners. It uses the same
application/docs filter and ready-PR, `main` push and manual triggers, plus a
weekly main run on Monday at 04:23 UTC. Scheduled and manual runs always check,
even without an application change. Each job runs
`cargo check --workspace --all-targets --locked`, strict workspace Clippy,
and `cargo test -p freshkube-terminal --lib keyboard --locked`. The keyboard
tests drive the real terminal view: Ctrl-C and Ctrl-V reach shell output, while
Ctrl-Shift-C/V copy and paste and Ctrl-Shift-Q returns focus on Linux and
Windows. The macOS workspace tests exercise the unchanged Command shortcuts.

These checks are **advisory**, separate from the macOS workflow used to promote
release bundles. A red native run blocks the lead's approval, but it is not a
required merge-button check; branch protection remains unchanged. Both install
protoc; Linux also installs the pinned GPUI Kit version's X11, Wayland, font,
WebKit and Vulkan prerequisites. A green run establishes
compilation, linting and the focused headless keyboard tests only: the full
Linux/Windows test suites, running-app checks, credential stores and packaging
remain separate work. The first uncached main run was green (Linux 16m23s,
Windows 32m42s); Linux becoming required will be reconsidered if its warm
runtime approaches macOS's.

Linux uses `Swatinem/rust-cache` with the `platform-check-Linux` key. Windows
remains uncached until an audit shows that adding it would preserve the macOS
entries. Only successful `main` push, scheduled or manual runs save; PRs and
dispatches on other branches restore without saving. The
action caches dependencies, excluding workspace crates and installed Cargo
tools, so each PR does not add its own copy. Check the Linux entry size and
the current macOS check, bundle and capture entries in **Actions → Caches**
after seeding or changing dependencies: all workflows share the 10 GB budget.
Let GitHub evict older unused entries; do not widen the cache policy if the
active set would evict the macOS caches. To measure a warm run after main has
saved the Linux entry, dispatch `gh workflow run platforms.yml --ref main` and
record restore, check, Clippy and keyboard test times.

Both the checks and bundle jobs install protobuf before building. They generate
the Talos client in their own Cargo output directory; no checked-in generated
Rust or regeneration maintenance command is required. Schema changes trigger
generation through the build script's recursive `proto/` dependency.
Both jobs also check that building leaves the tracked and untracked source tree
clean (Cargo outputs are ignored).

The bundles come from the shared
[macOS packaging workflow](../.github/workflows/macos-app.yml), on these standard
native runners:

| Rust target | GitHub runner | Executable architecture |
| --- | --- | --- |
| `aarch64-apple-darwin` | `macos-15` | `arm64` |
| `x86_64-apple-darwin` | `macos-15-intel` | `x86_64` |

These labels are listed for both public and private repositories in
[GitHub's runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
(checked 2 October 2026). Each job checks Rust's host triple to catch a changed
runner architecture.

Each architecture uploads a `freshkube-<target>` artifact containing the ZIP,
checksum and manifest, kept for 90 days, the longest GitHub allows, because a
release promotes these files. The `.app` is archived with `ditto` before upload,
so the GitHub artifact wrapper cannot strip its executable permissions. To get
bundles of a branch before merging it, run CI by hand: in
[Actions](https://github.com/skel84/freshkube/actions/workflows/ci.yml), **Run
workflow** on the branch, or
`gh workflow run ci.yml --repo skel84/freshkube --ref <branch>`. Artifacts from a
run that failed anywhere are diagnostic builds only.

CI has only `contents: read` permission and needs no Apple secrets. The old
cargo-dist publishing workflow, shell installer, updater configuration and dist
profile have been removed; do not regenerate them with `cargo dist init`.

For local captures on an Intel Mac, the separate
[Capture workflow](../.github/workflows/capture.yml) builds a **debug** executable
on `macos-15-intel`. Add the `capture` label to a pull request; each new commit
then replaces its pending capture build. Adding other labels does not start a
build, and this workflow never cancels the required CI check. To request a build
on a branch by hand, run
`gh workflow run capture.yml --ref <branch> -f debug_binary=true`.
Manual builds on the same branch wait for a running capture instead of cancelling
it; only newer PR heads cancel an in-progress capture.

The job uses stable Rust, protobuf and `Swatinem/rust-cache`, with a separate
`capture-debug-x86_64` key for debug builds. Only runs on `main` save the cache;
PRs and manual runs on other branches restore it without saving. Seed it once
on `main` with `gh workflow run capture.yml --ref main -f debug_binary=true`.
Keep an eye on the repository's shared cache usage under **Actions → Caches**
(10 GB budget).

The job runs
`cargo build --locked --bin freshkube --features capture-embed` and uploads `target/debug/freshkube` as
`freshkube-debug-x86_64-apple-darwin-<short-sha>`, kept for **3 days**. The SHA is
the checked-out PR head (or the selected revision for a manual run). Release
bundles ignore the debug page, kind, theme, window-size and text-size overrides;
use this debug artifact when a capture needs them.

`capture-embed` embeds GPUI Kit's icons in the debug executable. Without it,
`rust-embed` reads them from the build machine's Cargo registry at runtime, so
a downloaded binary draws no Kit icons on another Mac. Normal local debug
builds keep their existing asset loading; this feature is enabled only for
portable capture binaries. Its optional `rust-embed` dependency is pinned to
the version resolved for `gpui-kit-assets`, so their features unify; keep the
pin aligned when updating that dependency.

After the Capture run succeeds, download the artifact for the revision you want.
GitHub's artifact archive strips executable permissions, so restore them before
using the smoke helper. Replace the example run ID and short SHA below:

```sh
gh run list --workflow capture.yml
CAPTURE_RUN=123456789
CAPTURE_SHA=abc1234
CAPTURE_DIR="$PWD/target/ci-capture/$CAPTURE_SHA"
gh run download "$CAPTURE_RUN" --name "freshkube-debug-x86_64-apple-darwin-$CAPTURE_SHA" --dir "$CAPTURE_DIR"
chmod +x "$CAPTURE_DIR/freshkube"
FRESHKUBE_SMOKE_BINARY="$CAPTURE_DIR/freshkube" scripts/smoke.sh start --page monitoring
scripts/smoke.sh shot monitoring
scripts/smoke.sh stop
```

## Releases

A release ships the bundles `main` built for the release commit; it never
compiles again. [Release](../.github/workflows/release.yml) runs when a tag
`v*` is pushed:

1. It checks that the tag is `v` plus the `workspace.package` version in
   `Cargo.toml`, and that the tagged commit is on `main`. It also runs
   `scripts/changelog.sh check --empty`: any pending fragment stops the release
   with an instruction to collect the notes before tagging.
2. It finds the successful CI run of that commit on `main` and downloads both
   bundles. Without one (still running, failed, skipped or expired) it stops and
   says so; it never builds a replacement.
3. It verifies each ZIP against its checksum, and each manifest's version,
   target, source commit and clean tree against the tag.
4. It takes the release notes from the `## <version>` section of
   `CHANGELOG.md`, adds install instructions, and creates a **draft
   pre-release** with the ZIPs, checksums and manifests.

To release:

1. Open a pull request that runs `scripts/changelog.sh collect` **before**
   renaming the changelog's Unreleased section to `## <version> (<date>)`, and
   sets the version in `Cargo.toml` (and `Cargo.lock`). Commit the collected
   changelog and deleted fragments together; leave a new `## Unreleased`
   heading for the next release. Feature PRs add [fragments](../changelog.d/README.md)
   instead of editing Unreleased directly.
2. Merge it, and wait for CI on `main` to finish both bundles.
3. Tag the merge commit and push the tag:
   `git tag -a v<version> -m "Freshkube <version>" <commit>` then
   `git push origin v<version>`.
4. Review the draft on the
   [Releases page](https://github.com/skel84/freshkube/releases), and publish it.
5. The tap's [Bump casks](https://github.com/skel84/homebrew-tap/actions/workflows/bump.yml)
   workflow finds the published release within three hours and commits its
   version and checksums to the cask. Run it by hand to update the cask at once.
   It skips drafts and takes pre-releases.

To retry after a failure, fix the cause, delete the tag
(`git push origin :refs/tags/v<version>`, and any draft it left), and push it
again. Release needs `contents: write` to create the draft and `actions: read` to
download CI's artifacts, and nothing else.

Releases stay pre-releases while they are ad-hoc signed. Developer ID signing
and notarization would be added to the Release workflow, after the download and
before the draft ([Trusted distribution prerequisites](#trusted-distribution-prerequisites)).

The destination is the public [skel84/freshkube repository](https://github.com/skel84/freshkube).
Cargo metadata and artifact manifests refer to that destination.

## Install a release

With [Homebrew](https://brew.sh):

```sh
brew install --cask skel84/tap/freshkube
```

The cask lives in [skel84/homebrew-tap](https://github.com/skel84/homebrew-tap).
It installs `Freshkube.app` in `/Applications`, links the `freshkube` command
and verifies the download against the release's checksum. `brew upgrade --cask
freshkube` updates it, and `brew uninstall --zap --cask freshkube` also removes
preferences, the remembered selection and operation audit files. Homebrew
quarantines the app like any download, so the first open asks for **Open
Anyway** as described below.

To install by hand, download the ZIP and its `.sha256` for your Mac from the
[Releases page](https://github.com/skel84/freshkube/releases): `aarch64` for
Apple silicon, `x86_64` for Intel. For a build from a CI run instead, download
the `freshkube-<target>` artifact from the run's page and extract GitHub's outer
artifact ZIP. In the directory with the files:

```sh
shasum -a 256 -c Freshkube-*-adhoc.zip.sha256
ditto -x -k Freshkube-0.2.0-x86_64-apple-darwin-adhoc.zip .
codesign --verify --deep --strict --verbose=2 Freshkube.app
```

Substitute the version and target from your download. Move `Freshkube.app` to
`/Applications` or `~/Applications`, then open it. An ad-hoc signature validates
bundle integrity but does not establish a trusted publisher. macOS may block a
quarantined download; for a development build you trust, try opening it and then
use **System Settings → Privacy & Security → Open Anyway**, following
[Apple's instructions](https://support.apple.com/en-us/102445). Do not disable
Gatekeeper globally. Managed Macs may prohibit this override.

For a cluster-free launch check through LaunchServices:

```sh
open -n /Applications/Freshkube.app --args --fixture
```

For real use, click `Freshkube.app`, open **Settings → Talosconfig → Browse…**,
choose your talosconfig, then select its context in the sidebar. If the default
config cannot connect, **Choose talosconfig…** on the failure screen opens the
same native file picker. The selected file and context are remembered for the
next launch; you do not need a terminal command. The app saves only the absolute
file path and context name in
`~/Library/Application Support/Freshkube/connection.json`, without copying
certificates, private keys or file contents. A file that cannot be read or parsed
does not replace the last valid selection. A missing remembered file stays selected and offers
Browse for recovery.

You can also pass explicit paths and a named context with
`open -n ... --args --config <path> --context <name>`
or `--kubernetes-only --kubeconfig <path> --kube-context <name>`. No credentials
are included in artifacts. LaunchServices does not inherit a terminal's usual
environment or working directory. Use an absolute config path when passing it
to a bundle launch:

```sh
open -n /Applications/Freshkube.app --args --config /absolute/path/to/talosconfig --context '<context>'
```

Explicit `--config` and `TALOSCONFIG` take precedence over the remembered file;
`--context` takes precedence over the remembered context. With an explicit
config but no context, the file's current context is used. Without a saved
selection or explicit startup options, the app reads `~/.talos/config`.
Kubernetes-only, fixture and maintenance launches do not restore a Talos
selection over their requested mode. Preferences for text size remain in the
separate `preferences.json` file.

A Coroot API key or session value is saved only when **Remember key in
Keychain** is checked, as a login-keychain item named `Freshkube` whose
account is `Coroot <server URL>`; `coroot.json` holds the rest of the
connection and no key. The keychain trusts the app by its signature, and an
ad-hoc signature changes with every build, so after an update macOS may ask
whether Freshkube may use the item; **Always Allow** answers for that build.
Disconnect deletes the item; `brew uninstall --zap` does not, so remove it in
Keychain Access if you uninstall without disconnecting.

Most browsing uses the built-in Talos and Kubernetes clients;
`talosctl` is used by particular COSI queries and maintenance features, and a
kubeconfig may require its own exec authentication tool. Those tools are not
bundled, and are not prerequisites for a fixture launch or every browsing path.
Install/configure them only for the features and credentials you use. Finding
user-installed auth tools from a Finder launch remains separate work.

## Local Network privacy

The roadmap records a bundle launch failing to reach a LAN cluster while the
terminal launch succeeded. [Apple's TN3179](https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy)
explains that macOS 15 applies Local Network privacy to outgoing LAN connections,
while command-line tools launched from Terminal or SSH are automatically allowed.
This is consistent with the observation; the cause has **not** been confirmed on
a live cluster by this packaging change.

The bundle declares `NSLocalNetworkUsageDescription`, has a stable
`io.github.skel84.freshkube` identifier and a Mach-O UUID. Allow Freshkube's local
network request, or enable it in **System Settings → Privacy & Security → Local
Network**, then retry the connection. Apple notes that the first attempt may
fail before the user answers. The description explains the request; it does not
grant permission. Apple recommends an Apple-issued signing identity for reliable
privacy identity tracking across builds; ad-hoc artifacts can still show identity
or permission inconsistencies. Avoid keeping multiple copies installed during
this check. Retest macOS 15.0 issues on 15.1 or later.

Freshkube connects to configured endpoints; it does not browse/register Bonjour
services. `NSBonjourServices` is therefore not declared. The iOS multicast
entitlement is not required on macOS. This bundle is not App Sandboxed, so App
Sandbox network entitlements are not added. No App Transport Security bypass is
needed for its Rust/Talos/Kubernetes networking.

The remaining live check must launch the installed bundle through Finder or
`open`, grant Local Network access and list/read a cluster on a context the user
chooses. Compare with the same context in a terminal launch; record allow, deny
and retry behavior without exposing credentials or Secret values. A Terminal
execution of `Contents/MacOS/freshkube` cannot prove the bundle's privacy path.
Local and hosted packaging validation did not use a live cluster. The user's
subsequent ARM terminal-launch result is recorded below; the corresponding
LaunchServices connection check remains open.

## Investigate a TLS connection failure

`invalid peer certificate: BadSignature` is a TLS peer-verification error from
Rustls, separate from the `.app` code signature. It can arise while checking a
certificate or a TLS handshake signature. A peer responded far enough to start
TLS; this error alone does not establish a Local Network permission problem or
an architecture-specific bug. See Rustls's [certificate errors](https://docs.rs/rustls/0.23.36/rustls/enum.CertificateError.html).

Compare on the affected Mac with the **same explicit file and context**. A
talosconfig can contain multiple contexts, and a Finder launch does not inherit
`TALOSCONFIG` from a terminal. Substitute the actual path, selected Talos context
and installed app path below; `version` is a read-only request:

```sh
talosctl --talosconfig /path/to/talosconfig --context <context> version
/path/to/Freshkube.app/Contents/MacOS/freshkube --config /path/to/talosconfig --context <context>
```

Quit the app between launch comparisons. If both clients fail, investigate the
selected context's CA, endpoint and network route. If `talosctl` succeeds but
both app launch methods fail, investigate Rust TLS verification and the peer's
certificate/signature algorithm. If only the bundle launch fails, investigate
launch environment and Local Network permissions. These comparisons narrow the
cause; none alone proves it. Share the context name and outcomes, without
pasting talosconfig contents, private keys or credentials.

## Trusted distribution prerequisites

Before claiming a release is ready for ordinary download:

1. Choose the release policy and confirm a permanent bundle ID.
2. Obtain Apple Developer Program membership and a **Developer ID Application**
   certificate with its private key. Import it into a temporary CI keychain using
   restricted secrets. Sign the final bundle with hardened runtime and a secure
   timestamp; validate any necessary entitlements against actual app behavior.
3. Provide notarization credentials (App Store Connect API key, or the supported
   Apple ID/team/app-specific password arrangement). Submit the signed archive
   with `xcrun notarytool submit ... --wait`, inspect the result and log, staple
   the accepted ticket to the app with `xcrun stapler staple`, validate it, and
   recreate the ZIP and checksum from the stapled bundle.
4. Verify `codesign`, `stapler validate` and `spctl --assess --type execute`, then
   test a quarantined download on clean Macs for both architectures, fixture
   rendering, credential/tool discovery, LAN privacy and read-only cluster access.
5. Review transitive dependency licence/attribution requirements in addition to
   the checked-in notices, then explicitly approve release publication.

See Apple's [notarization requirements](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).
None of Developer ID signing, notarization, stapling, Gatekeeper acceptance or
live LAN access has been verified by the ad-hoc pipeline. An ad-hoc certificate
cannot substitute for Developer ID notarization.

## Validation

On 2 October 2026, an Intel Mac running macOS 15.2 built the release binary and
packaged the real app. Architecture, deployment target, UUID, system-only dynamic
links, plist metadata, bundled notices, ad-hoc signature, checksum and ZIP round
trip passed. A modified resource was rejected by signature verification. An
`open --args --fixture` launch stayed running without startup errors and created
native windows; the test instance was then stopped. Packaging rejection checks
also covered a wrong architecture, wrong minimum OS and a private dylib.

Formatting, Clippy with warnings denied, the workspace tests (including headless
UI tests and doctests), and workflow validation with actionlint 1.7.12 passed. The release build emitted an existing unused `navigation::known`
warning, and Cargo reported future incompatibility in `block` 0.1.6.

The [hosted candidate run](https://github.com/skel84/freshkube/actions/runs/37063705491)
at `77dcb1e` passed formatting, Clippy and the workspace tests, and successfully
packaged both architectures with Rust 1.99.0. Each native runner verified CLI
execution and the archived bundle. Both downloaded
artifacts passed checksum, revision, architecture, metadata, licence/resource,
executable mode and signature checks on this Intel Mac; its downloaded Intel
executable also passed the CLI smoke test. Trusted distribution and live LAN
access remain unverified.

The user subsequently reported the downloaded ARM bundle rendering fixture data
on their Mac.
A real Talos connection on that Mac initially reported `invalid peer
certificate: BadSignature`. The user then reported successful connectivity by
running `/Applications/Freshkube.app/Contents/MacOS/freshkube` from the terminal
with an explicit config path and the `kubernetes` context. The same explicit
selection through LaunchServices remains to be checked. The terminal result
does not prove a bundle privacy fix or the cause of the original TLS error.

The subsequent Finder-startup fix passed local formatting, Clippy with warnings
denied, 683 workspace tests and 11 doctests. Its UI regression failed before the
fix and passed afterward: choosing a file and context, then constructing a new
launch without arguments, restores both. Additional coverage checks the picker
on a connection failure, cancellation, recovery from an unreadable/malformed
file, startup overrides and keeping credentials out of saved preferences.

The [updated hosted candidate](https://github.com/skel84/freshkube/actions/runs/37069573594)
at `a8e8a5c` also passed formatting, Clippy, all 683 tests and 11 doctests,
and native packaging for ARM and Intel. Both downloaded artifacts passed the
same checksum, revision, architecture, metadata, notice, executable mode and
signature checks; the Intel executable ran locally, and each runner checked
its native executable. Clicking the updated ARM app and connecting with the
chosen file on the user's Mac remains the live check. The pipeline still signs
ad-hoc and does not notarize or publish releases.

Both hosted runs used manual dispatch. GitHub never started CI for a push or
pull request while `ci.yml` also declared `workflow_call`. Once that trigger was
removed, in favour of `workflow_dispatch`, pull request #12 and its merge to
`main` (`0861fb4`) ran CI by themselves. Removing it is the only trigger change
that coincides with the fix; the cause wasn't confirmed further.

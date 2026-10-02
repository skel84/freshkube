# macOS application packaging

Freshkube packages as two separate `Freshkube.app` bundles for macOS 15 or
later. The current pipeline ad-hoc signs them without Apple credentials. These
are development artifacts, **not notarized distributables**. No workflow creates
repositories, tags or GitHub Releases.

## Build locally

Use macOS, Rust/Cargo, Python 3, Xcode command line tools and `protoc`
(`brew install protobuf`). GPUI Kit enables `runtime_shaders`: Metal shader
source is embedded and compiled at runtime using macOS Metal. Fonts, themes and
icons are embedded too. The bundle carries the checked-in assets, `LICENSE`,
`NOTICE`, Apache-2.0 and Lucide/Feather texts, and both font OFL notices under
`Contents/Resources`. There is no external asset directory to install.

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

## CI and release candidates

[CI](../.github/workflows/ci.yml) runs on pushes to `main` and pull requests
against `main`. It runs formatting, Clippy and the workspace tests (including
headless UI tests), and calls the shared
[macOS packaging workflow](../.github/workflows/macos-app.yml). Packaging uses
these standard native runners:

| Rust target | GitHub runner | Executable architecture |
| --- | --- | --- |
| `aarch64-apple-darwin` | `macos-15` | `arm64` |
| `x86_64-apple-darwin` | `macos-15-intel` | `x86_64` |

These labels are listed for both public and private repositories in
[GitHub's runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
(checked 2 October 2026). Each job checks Rust's host triple to catch a changed
runner architecture.

[Package release candidate](../.github/workflows/release.yml) runs **only** via
`workflow_dispatch` and invokes the same checks and packaging at the selected
branch or existing tag. In Actions, choose that workflow, **Run workflow**, and
the branch to review. An existing tag can be selected
with `gh workflow run release.yml --repo skel84/freshkube --ref <tag>`. A candidate is acceptable only when the whole run is green;
artifacts from an otherwise failed CI run are diagnostic builds. Version tags
alone trigger nothing. The old cargo-dist publishing workflow, shell installer,
updater configuration and dist profile have been removed; do not regenerate the
legacy workflow with `cargo dist init`.

The destination is the public [skel84/freshkube repository](https://github.com/skel84/freshkube).
Cargo metadata and artifact manifests refer to that destination. The manual
workflow is available from its default branch in [Actions](https://github.com/skel84/freshkube/actions).

Both workflows have only `contents: read` permission and need no Apple secrets.
Each architecture uploads a `freshkube-<target>` artifact for 14 days, containing
the ZIP, checksum and manifest. The `.app` is archived with `ditto` before upload,
so the GitHub artifact wrapper cannot strip its executable permissions.

A future release flow should explicitly approve a versioned commit, run this
candidate workflow, validate both downloaded architectures and LAN access,
complete signing and notarization, and only then create and publish a release in
the chosen repository. No publication step is enabled here.

## Install a workflow artifact

Download the artifact for your Mac from the successful run's Actions page and
extract GitHub's outer artifact ZIP. In the extracted directory:

```sh
shasum -a 256 -c Freshkube-*-adhoc.zip.sha256
ditto -x -k Freshkube-0.1.11-x86_64-apple-darwin-adhoc.zip .
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

For real use, select your credentials and context in the app, or pass explicit
paths and a named context with `open -n ... --args --config <path> --context <name>`
or `--kubernetes-only --kubeconfig <path> --kube-context <name>`. No credentials
are included in artifacts. LaunchServices does not inherit a terminal's usual
environment. Most browsing uses the built-in Talos and Kubernetes clients;
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
No cluster context was supplied for this task, so no live connection was made.

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
executable also passed the CLI smoke test. Graphical launch on Apple Silicon,
trusted distribution and live LAN access remain unverified.

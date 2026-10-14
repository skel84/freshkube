# Smoke tests

How to run a smoke test with `scripts/smoke.sh` and `scripts/browser.sh`, moved from AGENTS.md's [Smoke tests](../AGENTS.md#smoke-tests) unchanged apart from its link paths (#587). AGENTS.md keeps the rule and the short form of each item below; everything here still applies.

## The helper

```sh
scripts/smoke.sh start --page observability-traces          # builds, launches with --fixture, waits for the window
scripts/smoke.sh shot traces                                # → target/smoke/<worktree>/traces.png; open it and look
scripts/smoke.sh key 'keystroke "k" using command down'     # any System Events key clause
scripts/smoke.sh click 640 220                              # points from the window's top-left
scripts/smoke.sh right-click 640 220                        # the same with the right button: a context menu
scripts/smoke.sh hover 640 220                              # the pointer there, no click or wheel: for tooltips
scripts/smoke.sh scroll 900 500 600                         # wheel at a point, 600 points down
scripts/smoke.sh full traces                                # traces-0.png, traces-1.png, … the whole scrolling page
scripts/smoke.sh stop
scripts/smoke.sh pages                                      # every page, one capture each
scripts/smoke.sh start --page overview -- --config <talosconfig> --context <name> --kubeconfig <file>
```

## Captures on an Intel Mac

For captures on an Intel Mac, add the `capture` label to your PR to get a debug
binary from the separate [Capture workflow](MACOS_PACKAGING.md#ci). New
commits refresh its artifact; it expires after three days. After the run succeeds,
download the artifact for the PR head you want (replace the example run ID and
short SHA), restore its executable bit, and pass it to the helper:

```sh
CAPTURE_RUN=123456789
CAPTURE_SHA=abc1234
CAPTURE_DIR="$PWD/target/ci-capture/$CAPTURE_SHA"
gh run download "$CAPTURE_RUN" --name "freshkube-debug-x86_64-apple-darwin-$CAPTURE_SHA" --dir "$CAPTURE_DIR"
chmod +x "$CAPTURE_DIR/freshkube"
FRESHKUBE_SMOKE_BINARY="$CAPTURE_DIR/freshkube" scripts/smoke.sh start --page monitoring
```

Use a debug binary for page, kind, theme, window-size and
text-size overrides; release builds and CI app bundles ignore them. Reuse a
recent debug capture set of `main` for a before comparison when one is available.

## The rules in full

- Smoke the pages and flows the change touches, in fixture mode and, when the change reads a cluster or Coroot, against the context the user chose. Run `scripts/smoke.sh pages` when a change reaches the frame, the theme or shared components.
- A page moved onto the shared components is compared with Pods side by side: capture both at the same size, theme and text size (`FRESHKUBE_KIND=pods scripts/smoke.sh start --page resources`, then the page) and check that title, padding, header, rows, group rows, glyphs and states line up.
- Look at every capture yourself, then report what you checked and what you saw: the captures that show the change, anything wrong, and anything you could not check.
- Judge a page from all of it, not its first screen: `full` captures each screenful down to the bottom, and `scroll` reaches a part further down to click there.
- `start` takes `--page`, `--theme`, `--size` and `--release`; the slugs are those of `FRESHKUBE_PAGE` (see [Build, run and test](../AGENTS.md#build-run-and-test)), plus `observability-<destination>`. Open pages with `--page` rather than navigating to them, and click only to exercise the change.
- `FRESHKUBE_SMOKE_BINARY=/absolute/path/to/saved/freshkube` makes `start` use that executable without building; `FRESHKUBE_STRESS_BINARY` selects a saved stress executable the same way.
- Live checks only look and navigate. Never press Operations or maintenance actions, and keep credentials out of captures you share.
- One worktree uses the screen at a time. `start`, `browser.sh open` and `stress.sh` wait for a lock (`scripts/smoke/lock.sh`, in `~/.cache/freshkube/screen.lock`) that `stop` and `close` release; a dead owner's lock, or a smoke test idle for ten minutes, is taken over. Build before you start, keep the session short, and always `stop`. "screen: waiting for …" means another worktree is checking; let it finish. A locked screen spoils captures and keys too, so every command waits for it to be unlocked ("screen: locked; waiting …"); keep the Mac awake for long unattended runs (`caffeinate -d -i`).
- A live `start` after a new build plays a sound: the app may ask Keychain for the remembered Coroot key, and only the user answers it.

## Comparing with Coroot or Grafana

To compare a page with the tool it reads from (Coroot, Grafana), `scripts/browser.sh` drives a Chrome window with the same commands: `open URL`, `go URL`, `shot`, `full`, `scroll`, `click`, `key`, `url` and `close`, with captures in `target/smoke/browser/`. Sign-ins are the user's: when a page asks for one, stop and ask them to sign in in that window.

## Permissions and the screen

The terminal needs Screen Recording and Accessibility in System Settings → Privacy & Security, and the screen must be unlocked. GPUI stops drawing a covered window, so a capture of a covered window or a locked screen shows only the first frames, before any read finishes. macOS drops keys sent to a window that isn't frontmost, so every helper command brings the app forward first. Keep interaction logic in UI tests too; a smoke test adds to them, it doesn't replace them.

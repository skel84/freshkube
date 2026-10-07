# Pod exec

A shell tab in the dock runs an interactive shell in one container, the way `kubectl exec -it` does. This file records the feasibility check, the decisions taken with the user, and the design that the build follows. The [roadmap](ROADMAP.md) tracks its progress.

## Feasibility

Checked on 2 October 2026 with a throwaway spike outside the workspace (`target/stress/term`, not committed). It fed synthetic byte streams into `alacritty_terminal` 0.26.0 and drew the grid in a GPUI Kit 0.7.0 window on a release build.

- **Emulator.** [`alacritty_terminal`](https://crates.io/crates/alacritty_terminal) is the terminal state machine behind Alacritty and Zed: VT parsing (`vte`), the grid, scrollback, the alternate screen, modes, selection and the escape-sequence replies. It is Apache-2.0, which an MIT app can ship with its licence and notice. Its rust-version is 1.85.
- **Dependencies it adds:** `alacritty_terminal`, `vte`, `cursor-icon`, `rustix-openpty` and `signal-hook`; the rest are already in the tree. The last two serve its local pty, which Freshkube doesn't use.
- **Parsing** runs at 65–90 MB/s, far beyond what an exec stream carries.
- **Drawing.** No published GPUI terminal widget builds against GPUI Kit 0.7.0: `gpui-terminal` targets gpui 0.2, `bezel-terminal` its own GPUI fork, and `gpui_xterm` (MIT) GPUI Kit 0.6. Freshkube draws its own grid. The spike's view copies the visible rows into style runs when bytes arrive, never in `render`, then paints backgrounds as quads and shapes each run with `shape_line` at its column × the cell width:

  | Synthetic stream | Frames/s | Paint per frame | Copy into runs | CPU |
  | --- | --- | --- | --- | --- |
  | `top`-like full redraw on the alternate screen, every row coloured | 47 (the source's rate) | 4.0 ms median, 6.3 ms max | 0.45 ms | 33% |
  | Coloured flood, 10,000 lines/s (1 MB/s) | 60 | 2.1 ms median, 3.5 ms max | 0.46 ms | 34% |
  | Coloured flood, 50,000 lines/s (5 MB/s) | 60 | 1.7 ms median, 2.8 ms max | 0.46 ms | 36% |

  CJK, emoji, combining marks, box drawing and wrapping lines keep to the grid. Box-drawing lines come out thin in Menlo, which is cosmetic.
- **Exec transport.** kube 0.98 runs exec over a websocket behind its `ws` feature, which adds `tokio-tungstenite`. `AttachedProcess` gives stdin, stdout, a terminal-size channel and the final `Status`.

Verdict: it fits. The work is ordinary: keyboard encoding, selection and scrollback in our own view, and the exec session's lifetime.

## Decisions

Taken with the user on 2 October 2026.

- **A narrow exception to read-only.** Exec needs `create` on `pods/exec`, and it is the only write the browsing pages make. A session starts only from an explicit Start on a chosen pod and container, never by opening the tab. Sessions are not recorded in the audit log.
- **A Shell tab in the pod's pane,** beside Logs, with the same container picker.
- **Ask before dropping a session.** Opening another object, closing the pane, switching context and quitting the app first ask "End the shell in ⟨pod⟩?". The session lives on while another page shows, so the hidden Resources page holds one connection, as an exception to "hidden screens never contact the cluster".
- **Moved to the dock** ([#284](https://github.com/skel84/freshkube/issues/284), 7 October 2026), superseding the two decisions above. The pane's header has a Shell menu, "Start shell in ⟨container⟩" for each container, enabled for running ones; the pick is the explicit Start, and opening the menu runs nothing. Each container's shell has a dock tab, "Shell ⟨pod⟩", which lives whatever page or object shows, so navigating never asks. Closing a running shell's tab asks "End the shell in ⟨pod⟩?", or "End 2 shells?" for several; so do another context, kubeconfig or talosconfig, closing the window and quitting. Shell tabs are saved by name with their container and come back idle: nothing starts until Start. At most eight shell tabs, counted apart from log tabs: a ninth asks "Close the oldest shell tab (⟨title⟩)?", and agreeing ends that shell. Control-. and Control-, go to a focused terminal like every other Control key.
- **A shortcut leaves the terminal.** Escape, Ctrl-C and Ctrl-V reach the shell. On macOS, Command-Escape hands the keyboard back to the list, and Command-C/V copy and paste; on Linux and Windows, those shortcuts use Ctrl-Shift-Q and Ctrl-Shift-C/V.
- **Option types characters,** as Terminal.app does by default, so international layouts keep working. Meta is Escape followed by the key. A setting can come later.
- **The pod may read and write the clipboard** through OSC 52, which tmux and vim use to yank. A program in the pod can then read whatever was last copied on the Mac, a password included, while a session is open. The user chose this; it is the place to revisit if the risk outweighs the convenience.

## Design

### Core: `freshkube-core::resources::exec`

- `start_exec(client, pod identity, container, size)` runs on Tokio and returns once the API server has accepted the exec, never before (Kubeli reports Connected before it knows). It gets the pod first: a pod with another UID, or a container that isn't running, fails without exec.
- The command tries the usual shells in turn: `sh -c 'command -v bash >/dev/null && exec bash; command -v ash >/dev/null && exec ash; exec sh'`. `exec` makes the shell's exit end the session.
- `AttachParams` turn on stdin and a TTY; with a TTY, stderr arrives on stdout.
- The first terminal size goes out before any input, so the remote side starts at the view's size, not 80 × 24. Kubeli never sends it until the window is resized.
- The session hands back two bounded channels. **Input** carries bytes and resizes in order, a resize coalesced with any newer one queued behind it. **Output** carries byte batches, up to 64 KiB or 8 ms each, and then one end event. A UTF-8 character split across reads needs no care: the emulator takes bytes.
- The end event says why it ended: the process exited (with its code, from the exec `Status`), the connection dropped, the pod went away, or the user ended it. Failures keep their category (forbidden, not found, container not running, no shell: exit 126 or 127 from the fallback chain, connection), rather than a single string.
- No automatic reconnect. A shell's state doesn't survive one, so the user starts a new session.
- Talos mode uses the Talos-derived Kubernetes client. It is built the same way as a kubeconfig context's, `Client::try_from(Config)`, so the upgrade takes the same path; step 4 checks it live.
- **Landed in step 2** (`crates/freshkube-core/src/resources/exec/`): `start_exec(client, ExecRequest)` returns an `ExecSession` with an `input` sender (`ExecInput::Bytes` or `Resize`), an `output` receiver (`ExecOutput::Bytes` batches, then one `End`) and a `guard` that stops the session when dropped; closing `input` ends it as `ExecEnd::Closed`. `ExecEnd` is `Exited(code)`, `PodGone(reason)`, `Disconnected(failure)`, `Closed` or `Failed(ExecFailure)`, and a failure's kind is a read's `FailureKind`, `NotRunning` or `NoShell`. Neighbouring keystrokes are joined into one frame and neighbouring resizes keep only the last. kube drops a refused upgrade's body, so a refusal is classified by its status code alone. Exit 126 or 127 means no shell only when the shell wrote nothing first: a shell that ran and whose last command wasn't found exits 127 too. When the exec closes without a status, the pod is read again to tell a deleted, replaced, stopped or restarted container from a dropped connection. The pod check, the request's parameters, classification, coalescing and batching are unit-tested against a fake API server; the websocket itself waits for step 4.

### Desktop: a terminal view and a shell source

- **`freshkube-terminal`** (desktop reaches it as `crate::terminal`) holds a reusable `TerminalView`. It knows nothing about Kubernetes.
  - **State.** It owns `Term` and the `vte` processor, with 10,000 lines of scrollback.
  - **Snapshot.** When bytes arrive, it feeds them and rebuilds the visible rows' runs, so `render` only paints. It redraws at most once a frame.
  - **Size.** The grid follows the element's bounds: rows and columns from the cell size, the cell from the theme's monospace font, so the text size scales the terminal too. A size change resizes `Term` and emits the new size, at most every 100 ms during a drag.
  - **Keys.** The view encodes keys from `TermMode`: application cursor keys and bracketed paste. A keystroke interceptor hands a focused terminal shell keys ahead of the app's bindings, so Escape, Tab, Control-Tab and plain Ctrl-C/V reach the program. Command shortcuts on macOS, and every Ctrl-Shift chord on Linux/Windows, pass through to the app keymap. Off macOS, none of those Ctrl-Shift chords reaches the shell, including Ctrl-Shift-arrows that an editor might otherwise handle. Typed text, Option characters, dead keys and input methods come through the platform input handler. GPUI reports keypad keys as their main-keyboard twins, so application keypad mode can't be honoured: the keypad types digits and Enter, as most terminals do by default.
  - **Mouse and scrollback.** It handles selection by dragging, double-click for a word and triple-click for a line. The wheel scrolls the scrollback; on the alternate screen it sends arrow keys, as Alacritty's alternate-scroll mode does. Mouse reporting to the program waits for a later step.
  - **Events.** The emulator's events go through an `EventListener`:
    - `PtyWrite` replies go back to stdin;
    - `ClipboardStore` and `ClipboardLoad` use the Mac clipboard;
    - `ColorRequest` answers with the theme's palette;
    - `Title` sets the tab's tooltip;
    - `Bell` is ignored.
  - **Theme.** The 16 ANSI colours come from the theme, in light and dark.
  - **Landed in step 1** (`crates/freshkube-desktop/src/terminal/`): `mod.rs` holds the view, its size and its events (`Output`, `Resize`, `Title`, `Leave`); `snapshot.rs` the style runs; `paint.rs` the canvas; `input.rs` keys, paste and the input handler; `mouse.rs` selection and the wheel; `listener.rs` the emulator's requests; `streams.rs` the synthetic streams for tests and `stress terminal`. The colours are a tuned pair in `palette.rs`. Small choices made with the user: a block cursor, hollow without focus, following the program's shape and never blinking; output keeps a scrolled-back view in place and a key or paste returns to the bottom; three lines a wheel notch; Command-C without a selection does nothing and the selection stays after copying; bold is SemiBold, never bright; no ligatures, which also halves the cost of shaping; a thin thumb at the right edge only while scrolled back.
- **`resources/shell/`** is the shell tab (formerly the pane's Shell tab, `resources/pane/shell/`): the container picker, Start, End, the session's state, and the confirmations. Its states are Idle, Connecting, Running, Ended (with the exit code or reason), and Failed (with the failure's category and Retry).
  - A tab shows one container's shell; the pane's Shell menu picks the container (until #284, the tab had its own picker and sat beside Logs).
  - An ended session's screen stays visible and selectable until a new one starts, which begins with a fresh terminal.
- **Example mode** (`--fixture`) runs a small local script instead of exec. It shows a prompt, echoes input and answers a few commands, including a full-screen redraw and a colour test. UI tests and screenshots drive it.
- **Keys.** The view has its own key context inside `KubeDetail`, so the pane's tab keys and Command shortcuts keep working. Command-F searching the scrollback waits for a later step.
- **Landed in step 3** (`crates/freshkube-desktop/src/resources/pane/shell/`): `mod.rs` holds the session and its states, `view.rs` the controls, status, failure banner and the empty state over the terminal, `example.rs` the example shell. One terminal serves the pane; it lays out under the empty state, so the first exec starts at its real size, and a new session resets it. Keys go to the exec through one ordered queue; End ends the shell politely (see Lifetime), and the screen stays until a new session. Choices made with the user: while a shell runs, the pane stays pinned to its pod and the list's selection moves freely, and only Enter or a click on another row (or Escape on the list) asks; the Shell tab has its own container picker, defaulting to the pod's default container and offering only running ones; a dot on the Shell tab marks a running session, with the shell's title as its tooltip. Choices made without asking, to revisit: another kind or namespace keeps the pinned pane rather than asking; a session still connecting ends at once on End, without asking; Cancel on another kubeconfig in Settings leaves the draft showing the new choice. Command-Shift-] and [ still switch tabs from the terminal, and the Shell tab hands the keyboard to the terminal once it shows a session. UI tests drive the example shell through the tab, the screen and the window.
- **Checked live in step 4**, with the user's go-ahead, on argo-system/argo-cd-argocd-repo-server, container repo-server (Kubernetes 1.32.3). The user ran the app in Talos mode: echo, `stty size` before and after resizing the pane, and `exit 3` ending as "The shell exited with code 3.". A throwaway program on core's `start_exec`, connecting from the kubeconfig as Kubernetes-only mode does, checked a stale UID refused before any exec, echo, the starting size (24 80), a resize (30 100), `exit 7`, and End and a dropped session while `sleep 120` ran. Its first runs showed that closing the connection left both the command and the shell running in the container, which led to the Control-C and Control-D ending under Lifetime; after it, both were gone within about 200 ms. Not covered: `vi` (the image has none), a forbidden or missing shell (none at hand), and the 1 s wait on quitting, which only the UI tests drive.

### Lifetime

| Event | What happens |
| --- | --- |
| Closing a running shell's tab, switching context, kubeconfig or talosconfig | Ask "End the shell in ⟨pod⟩?" ("End 2 shells?" for several); Cancel keeps everything as it was |
| Opening another object, closing the pane, another kind or namespace | The tab keeps its session; nothing asks |
| Another page shows | The session keeps running |
| The shell exits | Ended, with the exit code |
| The pod is deleted or restarts, or the connection drops | Ended, with the reason |
| End, or agreeing to any of the above | Control-C, then Control-D 200 ms later; the connection closes once the shell exits, or after 2 s |
| Quitting the app or closing the window | Ask, then end every session, waiting up to 1 s for them |

Ending a session sends keys because closing the connection alone stops nothing: the live check found that the container runtime keeps the terminal open, so the shell and whatever ran in it stayed in the container. Control-C stops the foreground command and Control-D ends the shell at its prompt. A program that ignores them, such as an open editor, keeps running, and the prompt says so. The user chose this over killing the shell's process group with a second exec, which would need `kill` in the image.

## Build order

Each step lands with its tests and moves to Done in the roadmap.

1. **Terminal view.** Promote the spike into `terminal/`, fed by synthetic streams. Add UI tests for keys, selection, copy, paste, scrollback, resize and text size. Add a terminal flood workload to the stress harness, with its numbers in [PERFORMANCE.md](PERFORMANCE.md).
2. **Exec in core.** Turn on kube's `ws` feature and add `start_exec` with input, resize, output, status and failure categories. Unit-test the parts that don't need a server: classification and resize coalescing.
3. **Shell tab.** Add the picker, Start and End, the states, confirmations, keys, example mode and UI tests.
4. **Live check.** Exec writes to the cluster, so it runs only with the user's go-ahead, on a pod and container they name, using harmless commands (`echo`, `stty size`, `ls`, opening and quitting `vi` without saving). The check covers both modes, a resize, an exit code, a forbidden or missing shell if one is at hand, and ending a session from every row of the lifetime table.

## References

- [Kubeli's shell commands](../../Kubeli/src-tauri/src/commands/shell.rs) for the shell fallback chain and batching (8 ms or 64 KiB). Avoid its gaps: it reports Connected before exec succeeds, never sends the first size and ignores the exit status.
- [`gpui_xterm`](https://crates.io/crates/gpui_xterm) (MIT) for drawing an `alacritty_terminal` grid in GPUI Kit, read for approach, not copied.

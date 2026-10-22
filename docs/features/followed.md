# Followed items, toasts and the bell

What the user turns on to be told about (issue #846), slice 1: the app-wide owner, the follows, toasts and the bell, and Settings › Notifications. macOS banners and their permission, firing alerts as a source and "Tell me when" come later and plug into the same owner.

Code:

- `crates/freshkube-followed/src/`: `model.rs` (a follow, its subject, the caps), `rules/` (what a read saw into updates), `notice/` (grouping, quiet hours, the words, the bell's lists), `saved.rs` (`follows.json`), `owner/` (`Followed` and its feeds), `live.rs` (the reads), `example.rs` (`FRESHKUBE_FOLLOWS` and the example script), `menu.rs` ("Notify me"), `bell.rs` (the status bar's entry).
- `crates/freshkube-desktop/src/desktop/followed.rs` (the shell offers connections and pushes toasts), `desktop/settings/notifications.rs`, `desktop/mod.rs` (`quit_app`).
- Entry points: a pod's menu in `crates/freshkube-resources/src/screen/pods.rs` and `menu.rs`; the change page's header in `crates/freshkube-applications/src/change/`.

Tests that prove it: inline tests in `freshkube-followed` (`rules/tests.rs`, `notice/tests.rs`, `owner/tests.rs`, `saved.rs`, `live.rs`, `model.rs`) and the desktop tests in `desktop/followed_tests.rs`.

## What it is

- **Off until picked.** Nothing notifies by default. A pod's menu offers "Notify me if it keeps crash-looping". A change page offers three, each saying what it keeps reading: the Freight reaches a Stage (the selected one, else the last), its verification fails there, an incident opens (in the application the page came from).
- **One owner.** `Followed` is made at startup and kept as a global, like `ForwardList`. A follow reads from the moment it is turned on until Stop, on any page and with the window closed; quitting calls `stop_all`. The lifetime is in [STATE_LIFETIMES.md](../STATE_LIFETIMES.md#followed-items).
- **Bounds.** At most 20 follows. One watch per pod, one per Kargo project however many follows share it, and Coroot's incidents once a minute. All reads use the user's own access and only read.
- **Saved in preferences.** `follows.json` beside `preferences.json` holds the follows, the next id and the notice settings. A follow comes back at the next launch reading nothing, and starts when the shell offers a connection for its context.
- **Not watching.** A follow that can't be read (no connection offered, the pod or Stage gone, a refused or unreachable read) shows "Not watching" and why. A read that fails says it once, as an update that is not a failure, and again only after it read again.

## What it tells

- **The same state never repeats.** A crash loop fires once, and again only after the pod has been Ready for 5 minutes. A landing and a failed verification end their follow. An incident is told once by its key; those open when the follow starts are not news.
- **Grouping.** The first update shows at once; updates within 2 minutes after it join one notice, shown when the window ends. A failure never waits: it shows at once, with whatever waited.
- **Quiet hours** (off by default) hold every update but failures. A held notice is in the bell as Held and shows no toast. Crash loops, failed verifications and incidents are failures; a landing and "Not watching" are not.
- **Hide details.** With it on a notice says only "Freshkube · 1 update" (or "n updates"). Default is on for protected and read-only clusters; the bell inside the app lists the details.

## The shell

The shell offers the follows the connection of the cluster the window is on, with its protection level, and Coroot's provider once Observability has a project. The owner emits an event for each notice that shows; the shell pushes it as Kit's in-app notification. The bell sits in the status bar after the port forwards: "Notifying · n", with its popover of New, Earlier and Following (with Stop).

## Known limits in this slice

- A change page's "reaches a Stage" and "verification fails" follows are about the cluster Kargo runs in (`Change.kargo_context`), not the one the window is on, and are saved with that context: the Kubernetes context of the workspace entry Kargo was read from (`At.context`), never the entry's id or the page's label, which differ from it. When that cluster isn't open they show "Not watching" and read once the shell offers its connection. The incident follow is about the window's cluster, whose Coroot project the shell offers. A confirmation toast about a follow in a cluster that isn't open hides what it is about, as an unknown level does: Hide details defaults to On until the shell offers that cluster.
- A saved follow of a cluster that is not open stays "Not watching" until it is.
- The rules' state (what was last told, a crash loop's re-arm, the incidents seen) is not saved. After a launch a follow can tell what it told before once more; that is accepted for now.
- Clicking a toast does not open the item yet; that comes with banners.

## Connections and levels

The shell offers the owner a connection (`Connection`: its access id, context, Coroot project and whether details hide by default) together with the level read in the same call (`Pilot::offer_followed`, `desktop/followed.rs`), so a level never reaches another cluster's follows. A new access id, or another Coroot project, replaces the running reads of that context; the same ones are left alone.

## Example data

`FRESHKUBE_FOLLOWS` (with `--fixture`) seeds the bell and each state: `bell`, `states`, `quiet`, `hidden` (the window opens on the protected `prod-ams`), `full`, `unwatched` and `grouped` (three landings told in a row, so the 2-minute grouping shows). Without it a follow made in the example cluster plays a short story: a pod becomes Ready and crash-loops, a Freight lands and its verification fails, an incident opens.

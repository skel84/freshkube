# First-run reports

A platform's build is **preview** until a tester has run it on a real desktop and
recorded the result here. CI proves that the archive opens a window
([PACKAGING.md](PACKAGING.md#ci)). A report proves that a person can use it.
The first release after a report with no blocking problem calls that platform
supported in its notes. From then on, its build is required for a release, like
macOS's.

## What a report records

Public content: no names, hostnames, cluster or context names, namespaces,
registries or anything else from the tester's environment. Describe what you
saw in general terms, and redact screenshots.

- **Build:** the version and archive file name, and the `.sha256` you checked it with.
- **Machine:**
  - Linux: OS and version, X11 or Wayland, desktop environment.
  - Windows: Windows version and display scaling.
  - Both: GPU and driver, or "software" if none.
- **Tester:** a role, such as "maintainer" or "contributor"; not a name.

## Checklist

Mark each item **works**, **problem** (with an issue link) or **not checked**.

1. **Start.** Extract the archive and start Freshkube.
   - Linux: after installing `RUNTIME-PACKAGES.txt`, a missing-library message is a problem.
   - Windows: note what SmartScreen asked.
   - The window opens within a few seconds, at a sensible size and scale.
2. **Example data.** Start with `--fixture`.
   - Overview, Nodes and Pods show their example rows.
   - A pod's log tab in the dock follows new lines, and a wheel up pauses it.
   - A pod's Shell menu starts the local example shell in a dock tab, and typing reaches it.
   - Settings: light and dark theme, and one larger text size.
3. **One real context, read-only.** A kubeconfig or talosconfig context the
   tester chooses (`--kubeconfig <file> --kube-context <name>`, or
   `--config <file> --context <name>`):
   - Overview loads, and a pod list shows.
   - A pod's logs stream.
   - No Operations or maintenance action is pressed.
4. **A shell.** Start a shell on a pod and container the tester names, and run a
   harmless command such as `ls`.
   - Copy and paste with Ctrl-Shift-C and Ctrl-Shift-V.
   - Ctrl-Shift-Q hands the keyboard back to the list.
   - `exit` ends the session.
5. **A remembered key.** Remember a key once (Coroot, or a dashboard source), then
   quit and start again.
   - It is read back from the Secret Service (Linux) or Credential Manager
     (Windows). A failure is shown, never a crash.
6. **Scaling,** if the machine has it: Windows at 150 %, or fractional scaling
   on Wayland. Text and icons stay sharp and aligned.
7. **Terminal output.** `freshkube --help` from a terminal prints the options.
   - Windows: the output appears after the prompt returns.

## Record

Newest first. A row links the issue or pull request that holds the full report.

| Date | Platform | Version | Machine | Result | Report |
| --- | --- | --- | --- | --- | --- |
| | Linux | | | not yet run | |
| | Windows | | | not yet run | |

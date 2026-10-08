- **Request snapshots in core.** `Snapshot` and `Request`, which keep a read's
  last answer for one identity and drop superseded completions, now live in
  `freshkube-core`. Desktop keeps them as `state::Snapshot`, still defaulting to
  the applied configuration. This is the second step toward moving
  Observability into its own crate.

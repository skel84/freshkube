- **No crate can be published by mistake:** `freshkube-core`, `talos-rs`
  and the `freshkube` binary now set `publish = false`, as the other
  workspace crates do, since Freshkube ships as an app.

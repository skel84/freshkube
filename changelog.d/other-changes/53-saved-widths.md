- **Inspector widths through a shared store.** Observability's Incidents,
  Traces and service map read and save their inspector widths through
  `freshkube-ui`'s store of widths, which the shell backs with
  `navigation.json` as before. The saved widths and their keys don't change.
  This is the third step toward moving Observability into its own crate.

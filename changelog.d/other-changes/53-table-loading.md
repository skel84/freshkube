- **Shared table loading rows.** `TableLoading`, which holds a table's loading
  rows and their motion until the first answer, now lives in `freshkube-ui`'s
  table module. The screens reach it by its old path. This is the first step
  toward moving Observability into its own crate.

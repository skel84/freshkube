- **Resources:** the list's watch now runs on the shared `Watch` from
  `freshkube-reads`, which batches what it sends at most ten times a second
  and drops a replaced read's late batches, as before; nothing on screen
  changes.

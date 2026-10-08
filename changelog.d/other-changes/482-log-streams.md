- **Shared log streams.** A workload's log tab reads its containers through a
  shared stream set (`logs/streams.rs`), which a pod's tab will use to show
  all its containers at once; nothing changes on screen.

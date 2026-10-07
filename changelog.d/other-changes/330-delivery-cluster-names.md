- **Delivery join destinations:** an Argo CD Application sent to `in-cluster`
  or `https://kubernetes.default.svc` now reaches the Rollout and pods when Argo
  CD runs in the environment cluster. Any other cluster name maps with
  `--known-as-name alias=name`, and when a name isn't mapped the join says how
  to map it. A Rollout's pods come from its ReplicaSet at the promoted digest,
  never from an older one beside it. The output now shows whether a Promotion
  was automatic or by hand, the commit it pushed, why a Stage is unhealthy, the
  Freight image's OCI revision and the run's result names, and it ends with a
  summary line ([#330](https://github.com/skel84/freshkube/issues/330)).

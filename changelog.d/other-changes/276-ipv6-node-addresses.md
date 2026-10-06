- **IPv6 nodes, continued:** Storage, the etcd status of each member, the
  control-plane lookup and the overview's version fallback read an IPv6 node
  address whole instead of cutting it at its first colon. Overview and
  Lifecycle discovery and the KubeSpan check handle IPv6 endpoints too.

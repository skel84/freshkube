- **Host names out of Status messages:** a Kubernetes or object status message
  now also loses the host names Go's errors write: after `lookup` (`lookup
  git.example.com on …: no such host`) and `address`, and in an x509 "valid
  for" list, its `not …` and its `none matched …`. A webhook's quoted name
  goes too (`failed calling webhook "…"`, `admission webhook "…"`). API group
  and resource names, such as `pipelineruns.tekton.dev` or `(get
  widgets.example.com)`, stay
  ([#314](https://github.com/skel84/freshkube/issues/314)).

- **Less hand-built Kubernetes plumbing:** single-object metadata reads use
  kube's `get_metadata`, a Service's label selector uses kube's `Selector`,
  and the GET reads, pod lookups and base-URL checks share one helper each.
  Requests are unchanged apart from an extra Content-Type header on
  single-object metadata reads.

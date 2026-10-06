- A Kubernetes-only UI test waits for the kubeconfig to be read before it
  checks the files Settings lists, so it no longer fails on a busy machine.

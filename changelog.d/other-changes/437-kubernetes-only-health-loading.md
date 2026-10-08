- **Health waits for the cluster in Kubernetes-only mode:** it shows its
  loading state until the Kubernetes summary first answers, instead of
  briefly saying no node is selected. When the kubeconfig or its context
  can't be read, it says why, with a Retry that reads the kubeconfig and
  connects again; with no context chosen, it asks for one.

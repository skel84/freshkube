- **Internal:** `freshkube-core` reads no environment variable. The
  kubeconfig variables, home directory, proxy settings and user name are read
  once at launch and handed to core as plain values, with the same precedence
  as before.

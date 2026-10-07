- **Proxy and local clusters:** a kubeconfig server on loopback (`127.0.0.1`,
  `::1`, `localhost`) is reached directly when `HTTPS_PROXY` is set, as kubectl
  does, instead of failing to make a client; a kubeconfig's own `proxy-url` is
  still used.

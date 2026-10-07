- **Kubernetes only from inside the app:** the "can't read your talosconfig"
  screen has a **Use Kubernetes only** button, and the context switcher in
  Talos mode has a **Use Kubernetes only…** entry. Both open the kubeconfig
  chooser; nothing connects until you pick a file and then a context. The
  switch asks first when a shell or port forward is open. The choice is
  remembered, so the next launch opens on that kubeconfig and context;
  choosing a talosconfig again returns to Talos and is remembered. A
  terminal's options still win.

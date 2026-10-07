- **Kubernetes only from inside the app:** the "can't read your talosconfig"
  screen has a **Use Kubernetes only** button, and the context switcher in
  Talos mode has a **Use Kubernetes only…** entry. Both open the kubeconfig
  chooser; nothing connects until you pick a file and then a context, and
  choosing another kubeconfig file in Settings works the same way. The
  switch asks first when a shell is open, as another connection does; port
  forwards keep running on the connection they started with. The choice is
  remembered, so the next launch opens on that kubeconfig and context;
  choosing a talosconfig again returns to Talos and is remembered. A
  terminal's options and its `KUBECONFIG` or `TALOSCONFIG` still win. A
  kubeconfig that can't be read now gets its own message in Kubernetes-only
  mode instead of the talosconfig one.

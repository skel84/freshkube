- **Log messages no longer lose their start:** a line such as
  `cart-db: connection refused` shows whole in pod, node and Coroot logs. Only
  a Talos service's own prefix (`main.go:94: `, `udevd[812]: `) and a leading
  level word the row already shows are left out.

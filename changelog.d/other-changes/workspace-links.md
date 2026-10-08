- **Links into another cluster:** an object link made in a cluster of
  `workspace.json` you have since left now opens that cluster first (asking
  before it ends a running shell) and then the object. A link from a session
  that has since reconnected says so and opens nothing. The header names the
  cluster you picked, with its failed state, when its kubeconfig can't be read,
  instead of "No context".

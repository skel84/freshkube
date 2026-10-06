- **Delivery chain spike:** core gains a read-only join from one commit or pull
  request to the pods running its image, through Tekton builds, Kargo Freight,
  Promotions and Stages, Argo CD Applications and Argo Rollouts, joined only on
  commit SHA and image digest, with each link marked confirmed, claimed or
  unknown. A `delivery_spike` example runs it against named contexts; setups
  that differ are described by settings with no defaults.

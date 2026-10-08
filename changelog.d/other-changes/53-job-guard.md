- **Request guard in core:** the guard that cancels a page's background
  request when the page moves on now lives in `freshkube-core`, with tests
  of its deadline and cancellation, so Monitoring can become a crate of its
  own. Nothing on screen changes.

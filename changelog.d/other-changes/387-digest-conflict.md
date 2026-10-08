- **Delivery trails note a Freight whose digest no build reports:** a
  Freight joined to a commit by its SHA, whose read builds of that commit
  report another digest for its image and none its own, is now claimed
  rather than confirmed. Its reason names the Freight's digest and each
  build's, with when it finished. It is not counted as a failure, since
  rebuilds of one commit rarely give the same digest.

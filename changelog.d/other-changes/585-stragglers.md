- **Internal:** the etcd and Applications example shapes, the fixture holds,
  the app menu button and the starting text size are parsed once into
  `DebugOptions` and handed to the pages, which no longer read the
  environment. Every value opens what it opened before; release builds still
  ignore them.

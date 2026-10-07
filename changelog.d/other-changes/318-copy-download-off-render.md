- **Log Copy and Download:** Copy no longer builds its text on every frame to
  decide whether it's enabled; any selection enables it, and a selection it can't
  copy says why. Download's temporary file is named `.<name>.freshkube-<pid>.part`
  and is removed on every failed save.

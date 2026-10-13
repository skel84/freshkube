- **Applications:** the list, the Changes list and the change page now read
  through the shared `Loader` from `freshkube-reads`. Each still reads only
  while it shows, and reads again on showing once its answer is 30 s old.
  A refresh cut short by leaving the page now reads again when you come
  back.

- **Internal:** delivery's connection, pool and change reads fail with a
  classified `Failure` instead of a string, and the words for a part that
  was refused, isn't served or can't be reached come from the failure's kind
  in one place. Nothing changes on screen.

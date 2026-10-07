- **Delivery links say where their evidence came from:** each link of a
  delivery trail now lists the observations behind it, with the cluster,
  object, UID, resource version, field and whether it was reported or
  declared, and a trail's text prints one `from …` line for each. What the
  join itself compared is marked as derived and named by its rule. A link
  from a source that couldn't be read lists none. A few confirmed links
  rest on a declared field alone on one side (a revision parameter, Chains'
  annotation, a Rollout's spec pin); their evidence says so, and #387 will
  decide their confidence, which this change leaves as it was. The read time
  comes from the caller.

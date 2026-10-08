- **Operations reads only while shown:** once left, Operations no longer
  checks the nodes or reads its audit log when another node is selected or
  the overview refreshes. It reads them for the current target when it is
  shown again; a submitted run still runs to its end.

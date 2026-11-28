# Invariants

| claim | node | anchor |
| --- | --- | --- |
| every ledger, log and embedding-cache append is one O_APPEND write under a shared flock, opened again when forget replaced the file while it waited — no line lands in an unlinked file | / | ※ locked-append |
| the machine id is minted by a hard link from a temp file, so racing first runs agree on one id | / | ※ machine-id |

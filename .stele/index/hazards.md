# Hazards

| claim | node | anchor |
| --- | --- | --- |
| a `SessionEnd` carrying reason `resume` is a beginning, not an ending — Claude Code fires it on the session it is adopting, so `take --hook` declines it. Forcing there moves the transcript out from under the turn about to append to it; Claude Code recreates the file, the next ending takes that live fragment too, and `.recent/<sid>.json` is overwritten to name the stub while the whole conversation sits orphaned in the archive | src | lm:resume-is-not-an-ending |

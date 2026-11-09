pi 1.5.0, recorded 2026-11-09 with scripts/record-pi.sh pi.
One headless turn (-p --mode json) with a capture extension given by -e, sessions under a throwaway --session-dir.
Each NN-<event>.json is one extension event ({event, payload, session, env}); strings are cut at 200 characters.
session.jsonl is the session file the agent wrote, trimmed the same way, provider payloads left out.
Paths and ids are redacted; the session env shows the inherited YARDSORT_* variables.
Recorded through scripts/record-pr-creation.sh pi: Run this exact shell command: gh pr create --title Bump --body Bump. Reply with the link it printed, nothing else.

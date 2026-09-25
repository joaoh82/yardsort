pi 0.87.1, recorded 2026-09-25 with scripts/record-pi.sh pi.
One headless turn (-p --mode json) with a capture extension given by -e, sessions under a throwaway --session-dir.
Each NN-<event>.json is one extension event ({event, payload, session, env}); strings are cut at 200 characters.
session.jsonl is the session file the agent wrote, trimmed the same way, provider payloads left out.
Paths and ids are redacted; the session env shows the inherited YARDSORT_* variables.

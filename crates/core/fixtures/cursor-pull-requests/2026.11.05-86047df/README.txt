cursor-agent 2026.11.05-86047df, recorded 2026-11-09 with scripts/record-cursor.sh.
One headless turn (-p --output-format json) with a capture plugin given by --plugin-dir.
Each NN-<event>[-<tool>].json is one hook call's stdin; strings are cut at 200 characters.
Paths and ids are redacted. The environment the hooks saw is noted above the fixtures in the script's output.
Recorded through scripts/record-pr-creation.sh cursor: Run this exact shell command: /tmp/yardsort-stand-in/bin/gh pr create --title Bump --body Bump. Then call the create_pull_request tool of the github MCP server with owner example, repo lib, title Bump, head bump and base main. Reply with the two links you got, nothing else.

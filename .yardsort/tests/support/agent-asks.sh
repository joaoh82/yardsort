#!/bin/sh
# What Claude Code's hooks report when it asks for a permission, and when the tool it asked about
# then ran, for remote-push.yaml. Typed into a shell Yardsort started, so the run's id is in its
# environment, as it is in an agent's:
#
#   agent-asks.sh <ys> asks | ran
#
# It goes through the hook command an agent runs, into the profile's inbox, and nothing else.
set -eu
case "$2" in
  asks) event=PermissionRequest ;;
  ran) event=PostToolUse ;;
  *)
    echo "Nothing called $2." >&2
    exit 2
    ;;
esac
printf '{"hook_event_name":"%s","session_id":"smoke-agent","tool_name":"Bash"}' "$event" |
  "$1" --yardsort-hook claude "$YARDSORT_DATA_DIR/activity/inbox"

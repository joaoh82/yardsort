#!/bin/sh
# A stand-in agent for host-restart.yaml. It keeps its conversation on disk, beside the workspace,
# as an agent does, so a resume after a restart finds it again and a fork copies it. Yardsort gives
# it the id, as it does Claude Code:
#
#   standin-agent.sh start <id> <message> | resume <id> | fork <id> <new id>
#
# After that it reads lines, adds each to the conversation, and says it heard it.
#
# A harness that passes a model or an effort puts them first, as `model=<name>` and
# `effort=<level>` (run-on-full.yaml): it says them back with the first message.
set -u
mkdir -p .standin
said=""
while :; do
  case "${1:-}" in
    model=* | effort=*)
      said="$said $1"
      shift
      ;;
    *) break ;;
  esac
done
case "$1" in
  start)
    id=$2
    shift 2
    echo "$*" > ".standin/$id"
    echo "conversation $id begins:$said $*"
    ;;
  resume)
    id=$2
    echo "conversation $id again, remembering: $(head -n 1 ".standin/$id")"
    ;;
  fork)
    id=$3
    cp ".standin/$2" ".standin/$id"
    echo "conversation $id forked from $2, remembering: $(head -n 1 ".standin/$id")"
    ;;
  *)
    echo "Nothing called $1." >&2
    exit 2
    ;;
esac
while IFS= read -r line; do
  echo "$line" >> ".standin/$id"
  echo "heard: $line"
done

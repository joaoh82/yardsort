#!/bin/sh
# A stand-in for gh on a host, for publish-there.yaml: it notes what it is asked in
# $STANDIN_GH_DIR/gh.log, opens pull request #7 for whatever branch it is run on, lists it from
# then on with the commit that branch is at, and says yes to a merge.
set -u
dir="${STANDIN_GH_DIR:?}"
echo "$*" >> "$dir/gh.log"
case "${1:-} ${2:-}" in
  "pr create")
    git branch --show-current > "$dir/gh.branch"
    title=""
    while [ $# -gt 0 ]; do
      [ "$1" = "--title" ] && title="$2"
      shift
    done
    printf '%s' "$title" > "$dir/gh.title"
    echo "https://git.example.test/acme/repo/pull/7"
    ;;
  "pr list")
    if [ -f "$dir/gh.branch" ]; then
      printf '[{"number":7,"url":"https://git.example.test/acme/repo/pull/7","title":"%s","headRefName":"%s","state":"OPEN","isDraft":false,"headRefOid":"%s","mergeable":"MERGEABLE"}]\n' \
        "$(cat "$dir/gh.title")" "$(cat "$dir/gh.branch")" "$(git rev-parse HEAD)"
    else
      echo "[]"
    fi
    ;;
  "pr merge") exit 0 ;;
  *)
    echo "gh: this is the smoke test's stand-in; '$*' is not answered" >&2
    exit 1
    ;;
esac

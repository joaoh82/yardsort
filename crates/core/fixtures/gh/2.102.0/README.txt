What gh 2.102.0 printed on 2026-10-02, against a public repository with real reviews and CI.

open-page-1.json, open-page-2.json
    Two pages of forge::OPEN_QUERY through `gh api graphql`, the second asked for with the
    first's cursor. Cut down to five and two pull requests out of 50 and 23, and `totalCount`
    set to 7 to match. A `requestedReviewer` of null is real: it is a team the asking account
    may not see.

pr-view.json
    `gh pr view <n> --json <forge::PULL_REQUEST_FIELDS>,body,comments,reviews,changedFiles`:
    one pull request with a comment and two reviews, cut down to its first three checks.

pr-list.json
    `gh pr list --state all --json <forge::PULL_REQUEST_FIELDS>`: one open, one merged, one
    closed, each cut down to its first three checks.

In all of them the shape, the states, the counts and the times are as recorded. Logins, names,
titles, descriptions, comments, branch names, commit ids and URLs were replaced: a fixture is
no place for the people whose pull requests these were.

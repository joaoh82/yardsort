What gh 2.102.0 printed on 2026-10-02, against a public repository with real reviews and CI.

open-page-1.json, open-page-2.json
    Two pages of forge::OPEN_QUERY through `gh api graphql`, the second asked for with the
    first's cursor. Cut down to five and two pull requests out of 50 and 23, and `totalCount`
    set to 7 to match. A `requestedReviewer` of null is real: it is a team the asking account
    may not see.

pr-view.json
    `gh pr view <n> --json <forge::PULL_REQUEST_FIELDS>,body,comments,reviews,changedFiles`:
    one pull request with a comment and two reviews, cut down to its first three checks.
    `baseRefOid`, which the question gained later, was added by hand with a made-up id.

pr-line-comments.json
    `gh api --paginate --slurp repos/{owner}/{repo}/pulls/<n>/comments`: one page, cut down to
    three comments on lines — one on a single line, one on a range, and a reply to the first.
    A reply carries `in_reply_to_id`; a first comment has no such key at all.

pr-list.json
    `gh pr list --state all --json <forge::PULL_REQUEST_FIELDS>`: one open, one merged, one
    closed, each cut down to its first three checks.

In all of them the shape, the states, the counts and the times are as recorded. Logins, names,
titles, descriptions, comments, branch names, commit ids and URLs were replaced: a fixture is
no place for the people whose pull requests these were.

The issues were recorded on 2026-10-03, against the same kind of repository: public, with
triage bots, outside reporters and maintainers.

issues-page-1.json, issues-page-2.json
    Two pages of the open issues as tasks::github asks for them through `gh api graphql`, the
    second asked for with the first's cursor. Cut down to seven and two issues out of 50 each,
    `totalCount` set to 9 to match, the cursors shortened, and `hasNextPage` on the second set
    to false. An `author` of null is real: an account that was deleted. So is a bot with an
    association of CONTRIBUTOR, and an open issue whose `stateReason` is REOPENED.

issues-closed.json
    The same query for closed issues, cut down to three: one closed as not planned, one as
    completed, one as a duplicate.

issues-disabled.json
    The same query against a repository with issues switched off, whole.

issue-view.json
    One open issue in full, as tasks::github asks for it: five comments, the first a bot's.

issue-view-hidden.json
    One closed issue with 62 comments, cut down to six in a row out of the 62 — one of them
    hidden on the forge as spam. `totalCount` is left at 62.

Replaced in all of them, as above: logins, titles, descriptions, comments and URLs. Label names
and colours are as recorded.

The repositories were recorded on 2026-10-08, against the recording account itself.

repos-page-1.json, repos-page-2.json
    Two pages of repositories::REPOSITORIES_QUERY through `gh api graphql`, the second asked
    for with the first's cursor. Cut down to four and two repositories out of 100 and 100,
    `totalCount` set to 6 to match, and `hasNextPage` on the second set to false. Kept, one of
    each: a repository with no description and no language, a fork with its parent, and an
    archived one — the last made by hand, since the account has none that is public.

repos-search.json
    `gh api search/repositories?q=<text>+in:name&per_page=5`, cut down to the eleven fields
    repositories::search_results reads out of the 83 each item carries; one of the two items
    made private, with issues switched off, by hand.

Replaced in all of them, as above: owners, names, descriptions and URLs. Only public
repositories were kept.

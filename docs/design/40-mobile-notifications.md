# 40 · Mobile notifications

Built for YARD-4. One shared way for a computer to tell the phone that something happened.
Workflows and desktop updates keep doing their own work. This delivers a notice and opens the item.

A workflow question or approval is answered on the computer (YARD-8 owns doing that on the phone).
A desktop update is installed on the computer (YARD-2 owns installing it from the phone). The
notice says the thing is waiting. The phone opens it. That is the whole action.

## Event model

A computer submits three fields: `eventKey`, `category`, and `kind`. The server chooses the title
and the body. The computer does not send words, a project name, a prompt, or a one-time code.

| Category        | Kind        | What is hashed                          | What the phone reads               |
| --------------- | ----------- | --------------------------------------- | ---------------------------------- |
| `task`          | `progress`  | `{owner.name}:{issue}:progress:{stamp}` | A task moved forward               |
| `task`          | `complete`  | `{owner.name}:{issue}:complete`         | A task finished                    |
| `review`        | `attention` | `{owner.name}:{number}:{reason}`        | A review needs attention           |
| `failure`       | `failed`    | `workflow:{run}`                        | Something failed                   |
| `desktopUpdate` | `available` | `{version}`                             | A desktop update is available      |
| `workflow`      | `question`  | `question:{run}:{step}:{opened}`        | A workflow has a question          |
| `workflow`      | `approval`  | `approval:{run}:{step}:{opened}`        | A workflow is waiting for approval |

The event key the computer submits, and the server stores, is the category prefix plus the
SHA-256 of that rest, written in hex. Two computers hash the same rest, so they still dedupe.
The stored key does not contain the repository, the issue or pull request number, or the run id.

`owner.name` is the GitHub repository, lower case, with a dot between owner and name. `reason` is
`checks`, `conflicts`, `changes`, or `unreviewed`. A task counts as complete only when it was
closed as completed. A failure is a workflow run whose status is `failed`. The error text stays on
the computer.

Task and review notices are emitted only when this computer fetches those lists. They are not on
a timer of their own. An item the computer has not seen before is recorded and sends nothing,
including a completed task on a later page and a pull request that was not in the last fetch. A
later change of stamp on an item it has seen, a completion of one it has seen, or a review reason
that appears on a pull request it has seen, is one notice. The same version of a desktop update
is one notice on every computer of the account.

An event key is at most 200 characters, starts with the category's prefix, and uses only lower
case letters, digits, and `:._-`. Upper case is refused, so a one-time code cannot be hidden in
the key. A field the server does not know, including `code`, is refused.

The body is a fixed sentence. It says to open the item on the phone, and that the work, the
answer, the approval, or the install stays on the computer.

## Delivery

The computer writes a row in its own outbox (`mobile_notice_outbox`, migration `0036`). While
Remote is on, the host posts that row to `POST /v1/notices` with its access token, about every 20
seconds. Delivery is HTTP. The relay carries terminal traffic and cannot see these events.

Only a device signed in as a computer (`host`) may submit. The account is the one in the access
token.

The server stores the notice (`mobile_notices`, migration `0006`) and opens one delivery for each
live phone of that account. If that phone has a push token and the provider accepts the send, it
goes out then. Otherwise the delivery stays queued.

The phone registers its token with `POST /v1/notifications/token`. The body is `platform` (`apns`
or `fcm`) and `token`. There is no device id in the body: the token belongs to the phone that is
signed in. Only a phone may register. `POST /v1/notifications/reachable` asks the server to send
what is still queued. The phone does that when the account screen opens, without asking for
permission. `GET /v1/notifications/{id}` is the item the tap opens. Those three posts, and
`POST /v1/notices`, are limited per device. A request with no usable access token is limited by
address instead.

The computer's post returns as soon as the notice is stored. Sending to APNs or FCM happens
after that, so the computer is not held open for the provider.

The deep link is `yardsort://notice/{category}/{id}`. `id` is the server's notice id. The category
in the path is `task`, `review`, `failure`, `desktop-update`, or `workflow`. The phone's screen
shows the title and the body. It asks every account the phone is signed in to, the one in use
first, so a notice for another signed-in account still opens. Answering, approving, and
installing stay on the computer. A link with a query string is left alone, so a code cannot ride
on it.

Permission is asked once, from Settings → Notifications, not when the app launches. iOS tokens
are registered as APNs and Android tokens as FCM. A token the system rotates is registered
again. A tap, including one that launched the app, opens the notice from `deepLink`. The app
config carries the `aps-environment` entitlement and a placeholder `google-services.json`. A
real Firebase file replaces the placeholder through `GOOGLE_SERVICES_JSON`. Neither file is a
credential that can send.

Push is behind a `PushProvider`. Tests record what would have been sent. A server with no APNs or
FCM credentials still starts. Those notices stay queued until a provider is configured, or until
they expire. A key that cannot sign is treated as unset and logged at startup. A failed FCM
token exchange logs its status and leaves the notice queued.

## Preferences

`GET` and `POST /v1/notifications/preferences` are the account's settings: `muted`, and one flag
each for `task`, `review`, `failure`, `desktopUpdate`, and `workflow`. Any device signed in to the
account may read or replace them. The default is all on, and not muted.

The phone's Settings → Notifications is that screen. The mute and the categories are for the
account in use, and the section is titled for that account. Mute covers every category. A
category that is off, or a mute, stores the notice so the same event is not sent later when the
setting is turned back on. A new event after that is sent.

## Deduplication

The server keeps one notice per account and event key. A retry of the same post, and the same
event from a second computer, are the same row and are sent once.

The push carries a collapse id, the SHA-256 of the account id and the event key. It is 64 hex
characters and does not contain the key. A second banner for the same event replaces the first on
the device.

The computer's outbox uses the event key as its primary key, so a retry does not queue a second
copy there either.

## Offline

Two queues, both with a one-day expiry.

- The computer cannot reach the server. The outbox keeps the notice. The next successful post
  delivers it. After a day it is expired. At most eight queued notices are posted. The rest are
  dropped, so a reconnect does not empty a long backlog onto the server.
- The server cannot reach the phone. The delivery stays `queued` when there is no token, or when
  the provider says it is unreachable. Registering a token, or `reachable`, sends it. A day later
  it is expired and a later flush does not send it. Each phone is handed at most eight queued
  notices. The rest are dropped, and the drop is logged with the account and the count. A second
  flush sends nothing that was already sent. A flush claims each delivery for about two minutes.
  Another flush takes it only after that lease, and only the flush that still holds the claim
  may finish it.

An account may have 100 notices still waiting. Notices already sent do not count. Past 100, a
new one is stored so it is not sent later, and the drop is logged with the account and the count.
Sent, dropped and expired rows in the computer's outbox, and items it recorded as seen, are
deleted after a week.

A token the provider says is gone is forgotten. For FCM that is HTTP 404 with
`error.details[].errorCode` of `UNREGISTERED`. For APNs that is HTTP 410 with a JSON reason of
`BadDeviceToken` or `Unregistered`. A `NOT_FOUND` that is not `UNREGISTERED` leaves the token in
place, so a wrong project id does not wipe every phone. An APNs 403 of `ExpiredProviderToken` or
`InvalidProviderToken` drops the cached provider token and tries again next time. The notice is
not sent to a token that is gone. Revoking a phone, or the account, deletes its token, and a
flush does not send to a revoked phone.

## Security

The access token is the only authority. A phone cannot submit a notice. A computer cannot register
a push token. One account's notice is a 404 for another account, and it is not handed to the other
account's token.

The alert is the title, the body, the deep link, and the category. The event key, the push token,
and any one-time approval or answer code are not in it. The token is stored so it can be sent to
APNs or FCM. It is not returned by an API and not written to a log. The request address, which
contains the token, is not logged either.

The desktop's own system notification is unchanged. When Settings → General includes the code,
that notification on the computer may still show it. The phone path never receives the code.

## What to configure

Push is optional. With none of this set, the server runs and notices wait.

Apple, all of these:

- `YARDSORT_APNS_KEY_ID`
- `YARDSORT_APNS_TEAM_ID`
- `YARDSORT_APNS_BUNDLE_ID`: `dev.yardsort.mobile` for a release build,
  `dev.yardsort.mobile.development` for a development build
- `YARDSORT_APNS_KEY` or `YARDSORT_APNS_KEY_FILE`: the `.p8` PEM. Set one of them.

`YARDSORT_APNS_SANDBOX=1` (also `true` or `yes`) sends to Apple's sandbox. A blank value is unset.

Android:

- `YARDSORT_FCM_CREDENTIALS` or `YARDSORT_FCM_CREDENTIALS_FILE`: the service account JSON. Set one.
- `YARDSORT_FCM_PROJECT_ID` when it should override `project_id` in that file.

A partial set, a key that cannot be read, or a key that cannot sign, is logged as a warning and
that provider stays off.
The server still starts. The self-host compose file passes these through and leaves them empty
until they are filled in.

# Deleting your account

Yardsort works without an account. You only have one if you turned on [Remote](remote.md) and
signed in to a Yardsort server. Deleting that account removes everything the server keeps of it.
Nothing on your computers or phones is deleted.

## What goes, and what stays

**Deleted from the server, for every device:**

- the account itself: the identity provider you sign in with, and your id there;
- every phone and computer signed in to it, each of them signed out;
- the devices each computer lets in, and pairing codes still open;
- which phones get notifications, their push tokens, your notification settings, and notices on
  their way to a phone;
- every synced project, note and workspace name, with the history sync keeps of them;
- how much the account used each day.

**Kept, on your own devices:** projects, workspaces, branches, agents and their terminals, notes and
everything else on your computers. Yardsort keeps working there without an account. Remote turns
off on the computer or phone you delete from. Your other devices find out the next time they reach
the server, and say the account was deleted.

Deleting cannot be undone. To keep a copy of what the server holds first,
[export your account](remote.md#exporting-your-account). Signing in to the same server again later
starts a new, empty account.

What the server keeps afterwards: the hashes of the deleted devices' credentials, linked to no
account or device, so a device that asks with one is told why it is signed out. Each goes when that
credential would have lapsed, within 90 days. The account's own sign-in at the identity provider is
the provider's. Deleting your Yardsort account does not delete it there.

## How

You confirm in two ways: you type **delete**, then you sign in again at your server's identity
provider. Signing in again has to happen there and then, even if the browser is still signed in, so
a phone or computer someone else picks up cannot delete your account. Nothing is deleted until that
sign-in comes back.

- **On a computer:** **Settings → Remote → Your account → Delete my account…**. It says what goes and
  what stays. Type **delete** and press **Sign in again and delete**, then sign in in the browser.
  **Cancel** stops waiting for the browser, and nothing is deleted.
- **On the phone:** **Settings → Your account → Delete my account**, then the same.
- **On the web, with no device to hand:** open `/account/delete` at your server's address, for
  example `https://remote.example.com/account/delete`. Press **Sign in again to delete**, sign in,
  type **delete** and press **Delete my account**. The account you signed in as is deleted. The
  page only works in the browser that started it.
- **On a computer with no screen:** `yardsort-host account delete`, at a terminal of your own on it,
  then sign in again in a browser on any computer and paste back the address it lands on. See
  [Deleting your account](headless-host.md#deleting-your-account) in its guide.

Deleted from somewhere else, a computer with no screen says so in
[`yardsort-host status`](headless-host.md#when-the-account-is-deleted-elsewhere).

From a computer, the phone or a computer with no screen, if you signed in again as someone else,
the server says so and deletes nothing. A server too old to delete accounts says so too; its
operator updates it.

-- Remote: the email this machine signed in as, as the identity provider gave it at that sign-in,
-- shown in Settings → Remote so its person can see which account each sign-in is in. The server
-- keeps no email; this copy is the only one. Null for a sign-in from before this, or a provider
-- that would not say. See docs/design/50-which-account.md.
ALTER TABLE remote_host ADD COLUMN email TEXT;
ALTER TABLE remote_computer ADD COLUMN email TEXT;

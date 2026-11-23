-- Who signed in, as the identity provider said, carried from the provider's answer to the device
-- that redeems the handoff, and gone with the handoff (docs/design/50-which-account.md). Nothing
-- else keeps it: an account is still its issuer and subject.
ALTER TABLE handoffs ADD COLUMN email TEXT;

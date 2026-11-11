-- The protocol minor a host last stated, when it made a pairing offer or opened its
-- relay link. Null when it has not stated one. A host below minor 7 is not shown
-- workflow scopes, and they are not stored as granted for it.
ALTER TABLE hosts ADD COLUMN protocol_minor INTEGER;

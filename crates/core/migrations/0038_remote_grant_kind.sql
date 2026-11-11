-- The kind of device this host's person approved, as the server reported it then.
-- A later channel whose kind does not match cannot decide a workflow. Null is a
-- pairing from before this column: the relay's kind is used, and a missing kind
-- still cannot decide. See docs/design/41-mobile-workflow-controls.md.
ALTER TABLE remote_grants ADD COLUMN device_kind TEXT;

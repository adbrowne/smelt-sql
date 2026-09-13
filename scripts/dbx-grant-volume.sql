-- One-off grant needed to unblock outcome 20260912-databricks-dogfood-spine, phase 11c.
-- Run as the identity that owns/administers `workspace.smelt_dogfood` (schema owner or
-- workspace admin) — the scoped oauth-m2m service principal cannot grant this to itself.
--
-- <client-id>: the oauth-m2m service principal's application ID, i.e. the same value
-- recorded as SMELT_DBX_CLIENT_ID in ~/.config/databricks-smelt-dogfood/config.env.
--
-- Idempotent: safe to re-run. This is the same statement scripts/dbx-provision.sh already
-- issues (it was amended in phase 11c to include CREATE VOLUME) — running that script again
-- as the owning identity has the same effect as running this file directly.

GRANT CREATE VOLUME ON SCHEMA workspace.smelt_dogfood TO `<client-id>`;

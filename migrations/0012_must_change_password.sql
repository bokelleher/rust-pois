-- migrations/0012_must_change_password.sql
-- Forced password change on first login.
--
-- `must_change_password = 1` blocks all API access (except the change-password
-- endpoint) until the user sets their own password. Accounts provisioned via the
-- API get a temp password and this flag set (AuthService::create_user); an admin
-- password reset re-arms it; a self-service change clears it.
--
-- No backfill: existing accounts (the seeded admin and any current accounts) keep
-- their access. Only accounts provisioned from here on are forced to change.

ALTER TABLE users ADD COLUMN must_change_password INTEGER NOT NULL DEFAULT 0;

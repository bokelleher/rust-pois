-- migrations/0006_sesame_channel_policy.sql
-- SESAME (SCTE 130-9) per-channel security policy.
--
-- Adds the minimum required SESAME tier per channel (§9.3 "per-channel security
-- policy that specifies the minimum required tier"). 0 = unauthenticated allowed
-- (backward compatible, the default for existing channels); 1 = require Tier 1
-- (HMAC auth); 2 = require Tier 2 (channel-scoped authz); 3 = require Tier 3
-- (payload encryption).
--
-- Key material itself is NOT stored here: per §8.2.5 key distribution is out of
-- band (env vars / secrets managers / config files), supplied via POIS_SESAME_KEYS.

ALTER TABLE channels ADD COLUMN sesame_min_tier INTEGER NOT NULL DEFAULT 0;

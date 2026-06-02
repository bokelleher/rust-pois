-- migration-demo.sql
-- =============================================================================
-- POIS demo seed: channels + rules that exercise most POIS capabilities.
--
-- This is a MANUAL demo seed, NOT an automatic sqlx migration (it has no numeric
-- version prefix, so the server will never run it on startup). Apply it by hand:
--
--     sqlite3 /opt/pois/pois.db < migration-demo.sql
--
-- It is idempotent: it re-creates the demo channels (INSERT OR IGNORE on the
-- unique name) and replaces any prior demo rules (all demo rules are name-tagged
-- 'DEMO:'). Your own channels/rules and the seeded 'default' channel are left
-- untouched.
--
-- Capabilities demonstrated:
--   * SESAME tiers per channel          (sesame_min_tier 0/1/2)
--   * Match by SCTE-35 command          (scte35.command)
--   * Match by segmentation type id     (scte35.segmentation_type_id)
--   * Match by segmentation UPID glob   (scte35.segmentation_upid)
--   * Match by acquisitionSignalID glob (acquisitionSignalID)
--   * Match by UTC time window          (utcBetween)
--   * anyOf / allOf / catch-all         (boolean composition)
--   * Actions: replace (built SCTE-35), replace (literal), delete (blackout), noop
--   * Priority ordering (lowest number wins; first match returns)
-- =============================================================================

BEGIN TRANSACTION;

-- ---- Demo channels (idempotent; name is UNIQUE) ----------------------------
INSERT OR IGNORE INTO channels (name, enabled) VALUES ('SportsFeed-East',   1);
INSERT OR IGNORE INTO channels (name, enabled) VALUES ('PremiumFeed',       1);
INSERT OR IGNORE INTO channels (name, enabled) VALUES ('NewsChannel',       1);
INSERT OR IGNORE INTO channels (name, enabled) VALUES ('RegionalAffiliate', 1);

-- Per-channel SESAME minimum tier (0 = regular ESAM allowed; 1/2 = SESAME required)
UPDATE channels SET sesame_min_tier = 1 WHERE name = 'SportsFeed-East';
UPDATE channels SET sesame_min_tier = 2 WHERE name = 'PremiumFeed';
UPDATE channels SET sesame_min_tier = 0 WHERE name = 'NewsChannel';
UPDATE channels SET sesame_min_tier = 0 WHERE name = 'RegionalAffiliate';

-- ---- Clean prior demo rules so the seed is re-runnable ----------------------
DELETE FROM rules WHERE name LIKE 'DEMO:%';

-- ===========================================================================
-- SportsFeed-East  (Tier 1 required) — live-sports ad avails
-- ===========================================================================
INSERT INTO rules (channel_id, name, priority, match_json, action, params_json) VALUES
((SELECT id FROM channels WHERE name='SportsFeed-East'),
 'DEMO: Ad-out splice_insert -> replace with built 30s avail', 10,
 '{"allOf":[{"scte35.command":"splice_insert"}]}',
 'replace',
 '{"build":{"command":"splice_insert_out","duration_s":30}}'),

((SELECT id FROM channels WHERE name='SportsFeed-East'),
 'DEMO: Program boundary (seg 0x10/0x11) -> pass-through', 20,
 '{"anyOf":[{"scte35.segmentation_type_id":"0x10"},{"scte35.segmentation_type_id":"0x11"}]}',
 'noop',
 '{}'),

((SELECT id FROM channels WHERE name='SportsFeed-East'),
 'DEMO: Catch-all pass-through', 100,
 '{}',
 'noop',
 '{}');

-- ===========================================================================
-- PremiumFeed  (Tier 2 required) — placement opportunities, time signals
-- ===========================================================================
INSERT INTO rules (channel_id, name, priority, match_json, action, params_json) VALUES
((SELECT id FROM channels WHERE name='PremiumFeed'),
 'DEMO: Placement Opportunity Start (seg 0x34) -> replace 60s avail', 10,
 '{"allOf":[{"scte35.segmentation_type_id":"0x34"}]}',
 'replace',
 '{"build":{"command":"splice_insert_out","duration_s":60}}'),

((SELECT id FROM channels WHERE name='PremiumFeed'),
 'DEMO: time_signal -> replace with built time_signal_immediate', 20,
 '{"allOf":[{"scte35.command":"time_signal"}]}',
 'replace',
 '{"build":{"command":"time_signal_immediate"}}'),

((SELECT id FROM channels WHERE name='PremiumFeed'),
 'DEMO: Catch-all pass-through', 100,
 '{}',
 'noop',
 '{}');

-- ===========================================================================
-- NewsChannel  (Tier 0, regular ESAM) — blackout / signal filtering
-- ===========================================================================
INSERT INTO rules (channel_id, name, priority, match_json, action, params_json) VALUES
((SELECT id FROM channels WHERE name='NewsChannel'),
 'DEMO: Blackout by signal id glob (blk-*) -> delete', 10,
 '{"anyOf":[{"acquisitionSignalID":"blk-*"}]}',
 'delete',
 '{}'),

((SELECT id FROM channels WHERE name='NewsChannel'),
 'DEMO: splice_insert -> replace with literal SCTE-35 slate', 20,
 '{"allOf":[{"scte35.command":"splice_insert"}]}',
 'replace',
 '{"scte35_b64":"/DAhAAAAAAAAAP/wEAUAAAAAf+//AAAAAH4AAAAAAAA1FwHt"}'),

((SELECT id FROM channels WHERE name='NewsChannel'),
 'DEMO: Catch-all pass-through', 100,
 '{}',
 'noop',
 '{}');

-- ===========================================================================
-- RegionalAffiliate  (Tier 0, regular ESAM) — UPID targeting + time windows
-- ===========================================================================
INSERT INTO rules (channel_id, name, priority, match_json, action, params_json) VALUES
((SELECT id FROM channels WHERE name='RegionalAffiliate'),
 'DEMO: Region-tagged UPID (*AFE1*) -> replace 15s', 10,
 '{"allOf":[{"scte35.segmentation_upid":"*AFE1*"}]}',
 'replace',
 '{"build":{"command":"splice_insert_out","duration_s":15}}'),

((SELECT id FROM channels WHERE name='RegionalAffiliate'),
 'DEMO: Prime-time window (utcBetween) -> replace 45s', 20,
 '{"allOf":[{"utcBetween":{"start":"2026-06-02T18:00:00Z","end":"2026-06-02T23:00:00Z"}}]}',
 'replace',
 '{"build":{"command":"splice_insert_out","duration_s":45}}'),

((SELECT id FROM channels WHERE name='RegionalAffiliate'),
 'DEMO: Catch-all pass-through', 100,
 '{}',
 'noop',
 '{}');

COMMIT;

-- Summary of what was seeded:
--   SELECT c.name, c.sesame_min_tier, r.priority, r.name, r.action
--   FROM rules r JOIN channels c ON r.channel_id=c.id
--   WHERE r.name LIKE 'DEMO:%' ORDER BY c.name, r.priority;

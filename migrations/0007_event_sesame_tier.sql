-- migrations/0007_event_sesame_tier.sql
-- Record the SESAME (SCTE 130-9) security tier achieved on each ESAM request in
-- the event log, surfaced in the Event Monitor.
--   NULL = SESAME inactive / Tier 0 (unauthenticated passthrough)
--   1    = Tier 1 (HMAC-SHA256 authentication + integrity)
--   2    = Tier 2 (+ channel-scoped authorization)
--   3    = Tier 3 (+ AES-256-GCM payload encryption)

ALTER TABLE esam_events ADD COLUMN sesame_tier INTEGER;

-- Rebuild the view to expose the new column.
DROP VIEW IF EXISTS esam_events_view;
CREATE VIEW esam_events_view AS
SELECT
  e.id,
  e.timestamp,
  e.channel_name,
  e.acquisition_signal_id,
  e.utc_point,
  e.source_ip,
  e.scte35_command,
  e.scte35_type_id,
  e.scte35_upid,
  e.matched_rule_id,
  e.matched_rule_name,
  e.action,
  e.processing_time_ms,
  e.response_status,
  e.error_message,
  e.sesame_tier,
  c.timezone as channel_timezone,
  r.priority as rule_priority
FROM esam_events e
LEFT JOIN channels c ON e.channel_name = c.name
LEFT JOIN rules r ON e.matched_rule_id = r.id
ORDER BY e.timestamp DESC;

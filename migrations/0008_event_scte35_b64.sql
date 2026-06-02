-- migrations/0008_event_scte35_b64.sql
-- Store the original SCTE-35 base64 payload on each event so the Event Monitor
-- can always show it (the collapsible "SCTE-35 BASE64" panel in event details),
-- independently of POIS_STORE_RAW_PAYLOADS (which gates the full request/response
-- XML). This is just the SCTE-35 BinaryData, not the whole payload.

ALTER TABLE esam_events ADD COLUMN scte35_b64 TEXT;

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
  e.scte35_b64,
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

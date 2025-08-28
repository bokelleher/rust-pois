-- migrations/0002_event_logging.sql
CREATE TABLE IF NOT EXISTS esam_events (
  id                    INTEGER PRIMARY KEY AUTOINCREMENT,
  timestamp            TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  channel_name         TEXT NOT NULL,
  acquisition_signal_id TEXT NOT NULL,
  utc_point            TEXT NOT NULL,
  source_ip            TEXT,
  user_agent           TEXT,
  
  -- SCTE-35 details
  scte35_command       TEXT,
  scte35_type_id       TEXT,
  scte35_upid          TEXT,
  
  -- Rule matching
  matched_rule_id      INTEGER REFERENCES rules(id),
  matched_rule_name    TEXT,
  action               TEXT NOT NULL DEFAULT 'noop',
  
  -- Request/Response details
  request_size         INTEGER,
  processing_time_ms   INTEGER,
  response_status      INTEGER NOT NULL DEFAULT 200,
  error_message        TEXT,
  
  -- Raw payloads (optional, for debugging)
  raw_esam_request     TEXT,
  raw_esam_response    TEXT
);

-- Indexes for common queries
CREATE INDEX IF NOT EXISTS idx_esam_events_timestamp ON esam_events(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_esam_events_channel ON esam_events(channel_name, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_esam_events_action ON esam_events(action, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_esam_events_rule ON esam_events(matched_rule_id, timestamp DESC);

-- View for dashboard queries (pre-joined data)
CREATE VIEW IF NOT EXISTS esam_events_view AS
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
  c.timezone as channel_timezone,
  r.priority as rule_priority
FROM esam_events e
LEFT JOIN channels c ON e.channel_name = c.name
LEFT JOIN rules r ON e.matched_rule_id = r.id
ORDER BY e.timestamp DESC;
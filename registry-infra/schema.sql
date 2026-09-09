-- D1 schema: scrubbed failure reports only. No user data ever.
CREATE TABLE IF NOT EXISTS reports (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  record_id TEXT NOT NULL,        -- canonical registry id
  method_id TEXT NOT NULL,
  registry_version INTEGER NOT NULL,
  error_class TEXT NOT NULL,      -- http_404 | hash_mismatch | tls | ...
  http_status INTEGER,
  url_hash TEXT NOT NULL,         -- BLAKE3 of the URL — never the URL
  ts_bucket TEXT NOT NULL,        -- hour precision
  received_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_reports_record ON reports(record_id, error_class);
CREATE TABLE IF NOT EXISTS unresolved (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  query_hash TEXT NOT NULL,       -- hashed; raw queries are never stored
  ts_bucket TEXT NOT NULL,
  received_at TEXT NOT NULL
);

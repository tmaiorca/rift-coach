-- Phase 2: derived metrics, AI reports, persisted activity log.

-- One row per (match, participant, metric), for all ten players in every
-- stored match. The other nine players in your games form the comparison
-- population ("better than X% of mid laners in your games"). Rebuildable
-- from match_raw at any time.
CREATE TABLE participant_metrics (
    match_id       TEXT    NOT NULL REFERENCES matches (match_id) ON DELETE CASCADE,
    participant_id INTEGER NOT NULL,
    metric         TEXT    NOT NULL,
    value          REAL    NOT NULL,
    PRIMARY KEY (match_id, participant_id, metric)
) WITHOUT ROWID;
CREATE INDEX participant_metrics_metric ON participant_metrics (metric, match_id, participant_id);

-- Which metrics version produced a match's rows; bumping the version in
-- code recomputes stale matches.
CREATE TABLE match_metrics_state (
    match_id    TEXT    PRIMARY KEY REFERENCES matches (match_id) ON DELETE CASCADE,
    version     INTEGER NOT NULL,
    computed_at INTEGER NOT NULL
);

-- Every AI report ever generated (kept as history). Cached by
-- (kind, account, match, prompt version, provider, model).
CREATE TABLE ai_reports (
    id             INTEGER PRIMARY KEY,
    kind           TEXT    NOT NULL,          -- post_game | ...
    account_id     INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    match_id       TEXT    REFERENCES matches (match_id) ON DELETE CASCADE,
    provider       TEXT    NOT NULL,
    model          TEXT    NOT NULL,
    prompt_version TEXT    NOT NULL,
    input_json     TEXT    NOT NULL,          -- the facts sent (no keys, no raw dumps)
    output_json    TEXT    NOT NULL,          -- validated report
    validation     TEXT    NOT NULL,          -- ok | unverified
    notes          TEXT,
    input_tokens   INTEGER,
    output_tokens  INTEGER,
    created_at     INTEGER NOT NULL
);
CREATE INDEX ai_reports_match ON ai_reports (match_id, kind, created_at DESC);
CREATE INDEX ai_reports_created ON ai_reports (created_at DESC);

-- Activity shown in the UI, kept across restarts (pruned to recent rows).
CREATE TABLE activity_log (
    id      INTEGER PRIMARY KEY,
    at      INTEGER NOT NULL,
    level   TEXT    NOT NULL CHECK (level IN ('info', 'warn', 'error')),
    message TEXT    NOT NULL
);

-- Phase 3: death events for pattern detection, focus goals, session reports.

-- Every champion death in stored matches (all ten players), with the
-- context the pattern detectors need. Rebuilt with the metrics from
-- match_raw, so it follows METRICS_VERSION.
CREATE TABLE death_events (
    match_id         TEXT    NOT NULL REFERENCES matches (match_id) ON DELETE CASCADE,
    participant_id   INTEGER NOT NULL,
    seq              INTEGER NOT NULL,       -- 1-based, in game order
    t_ms             INTEGER NOT NULL,
    x                INTEGER NOT NULL,
    y                INTEGER NOT NULL,
    zone             TEXT    NOT NULL,       -- rc_metrics::zones::Zone, relative to the victim's team
    enemies          INTEGER NOT NULL,       -- enemy champions credited (killer + assists)
    solo             INTEGER NOT NULL,
    before_14        INTEGER NOT NULL,
    enemy_side       INTEGER NOT NULL,
    gold_vs_opponent INTEGER,                -- at the minute snapshot before the death
    unspent_gold     INTEGER,
    shutdown_bounty  INTEGER NOT NULL,
    enemy_objective  TEXT,                   -- epic monster the enemy took within 60 s
    killer_champion  TEXT,
    PRIMARY KEY (match_id, participant_id, seq)
) WITHOUT ROWID;

-- Focus goals ("average at most 1 death before 14:00 over my next 10 games").
CREATE TABLE goals (
    id            INTEGER PRIMARY KEY,
    account_id    INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    metric        TEXT    NOT NULL,
    comparator    TEXT    NOT NULL CHECK (comparator IN ('at_most', 'at_least')),
    target        REAL    NOT NULL,
    games         INTEGER NOT NULL,          -- how many games the goal runs for
    role          TEXT,                      -- only count games in this role (NULL = any)
    champion_name TEXT,                      -- only count games on this champion (NULL = any)
    baseline      REAL,                      -- your average before the goal started
    source        TEXT    NOT NULL DEFAULT 'user',   -- user | ai | pattern
    note          TEXT,
    status        TEXT    NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'completed', 'abandoned')),
    achieved      INTEGER,                   -- set when completed
    created_at    INTEGER NOT NULL,
    ended_at      INTEGER
);
CREATE INDEX goals_account ON goals (account_id, status, created_at DESC);

-- Reports not tied to one match (sessions, later weekly) are keyed by scope,
-- e.g. "session:NA1_123:4" (first match id, game count).
ALTER TABLE ai_reports ADD COLUMN scope TEXT;
CREATE INDEX ai_reports_scope ON ai_reports (scope, kind, created_at DESC);

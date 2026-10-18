-- Phase 4: games seen live (own numbers only), their snapshots, the app's
-- footprint during them, and planned builds.

-- One row per game the live poller saw. Linked to the Match-V5 match once
-- Riot publishes it.
CREATE TABLE live_games (
    id                 INTEGER PRIMARY KEY,
    account_id         INTEGER REFERENCES accounts (id) ON DELETE SET NULL,  -- NULL: not a tracked Riot ID
    champion_name      TEXT    NOT NULL,     -- as the client shows it
    champion_ddragon   TEXT,                 -- Data Dragon id, when reported
    position           TEXT,
    game_mode          TEXT    NOT NULL,
    started_at         INTEGER NOT NULL,     -- wall clock at game time 0 (estimated)
    ended_at           INTEGER,
    end_reason         TEXT,                 -- game_end | disappeared
    result             TEXT,                 -- Win | Lose (from the GameEnd event)
    duration_s         REAL,
    final_json         TEXT,                 -- last own snapshot
    match_id           TEXT,                 -- set once found in Match-V5
    analyzed_at        INTEGER,              -- metrics computed with the timeline
    report_id          INTEGER,              -- automatic AI review, if any
    footprint_json     TEXT,                 -- rc_live::footprint::Footprint
    snapshots_pruned   INTEGER NOT NULL DEFAULT 0,
    created_at         INTEGER NOT NULL
);
CREATE INDEX live_games_started ON live_games (started_at DESC);
CREATE INDEX live_games_match ON live_games (match_id);

-- Own numbers every ~15 s of game time. Pruned after the game is analyzed
-- (Settings → Live → keep snapshots).
CREATE TABLE live_snapshots (
    live_game_id INTEGER NOT NULL REFERENCES live_games (id) ON DELETE CASCADE,
    t_ms         INTEGER NOT NULL,           -- game time
    data         TEXT    NOT NULL,           -- rc_live::OwnSnapshot as JSON
    PRIMARY KEY (live_game_id, t_ms)
) WITHOUT ROWID;

-- A job asked for again while it was running runs once more when it
-- finishes (e.g. metrics started before the timeline arrived).
ALTER TABLE jobs ADD COLUMN rerun INTEGER NOT NULL DEFAULT 0;

-- Planned build per champion (Data Dragon id), shown in the live view.
CREATE TABLE build_plans (
    champion   TEXT    PRIMARY KEY,
    items      TEXT    NOT NULL,             -- JSON array of item ids, in order
    updated_at INTEGER NOT NULL
);

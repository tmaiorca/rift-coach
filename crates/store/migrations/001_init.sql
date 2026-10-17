-- Rift Coach schema v1.
-- Identity rule: PUUIDs are encrypted per API key, so they are never primary
-- keys. Matches are keyed on match_id; accounts on an internal id + Riot ID;
-- every stored PUUID is tagged with the key generation that produced it.
-- All timestamps are unix milliseconds.

CREATE TABLE accounts (
    id          INTEGER PRIMARY KEY,
    game_name   TEXT    NOT NULL,
    tag_line    TEXT    NOT NULL,
    platform    TEXT    NOT NULL,   -- e.g. na1
    region      TEXT    NOT NULL,   -- e.g. americas (Match-V5 routing)
    is_primary  INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE UNIQUE INDEX accounts_riot_id
    ON accounts (game_name COLLATE NOCASE, tag_line COLLATE NOCASE, platform);

-- A new generation starts whenever the API key changes, detected when
-- Account-V1 returns a different PUUID for an already-resolved account.
-- Nothing derived from the key itself is stored.
CREATE TABLE key_generations (
    id            INTEGER PRIMARY KEY,
    first_seen_at INTEGER NOT NULL,
    note          TEXT
);

CREATE TABLE account_puuids (
    account_id  INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    key_gen_id  INTEGER NOT NULL REFERENCES key_generations (id),
    puuid       TEXT    NOT NULL,
    resolved_at INTEGER NOT NULL,
    PRIMARY KEY (account_id, key_gen_id)
);
CREATE INDEX account_puuids_puuid ON account_puuids (puuid);

CREATE TABLE matches (
    match_id      TEXT    PRIMARY KEY,   -- e.g. NA1_5123456789
    platform      TEXT    NOT NULL,
    queue_id      INTEGER NOT NULL,
    map_id        INTEGER NOT NULL,
    game_mode     TEXT    NOT NULL,
    game_type     TEXT,
    game_version  TEXT    NOT NULL,      -- full, e.g. 16.20.712.3456
    patch         TEXT    NOT NULL,      -- major.minor, e.g. 16.20
    game_start_ms INTEGER NOT NULL,
    duration_s    INTEGER NOT NULL,
    is_remake     INTEGER NOT NULL DEFAULT 0,
    has_timeline  INTEGER NOT NULL DEFAULT 0,
    ingested_at   INTEGER NOT NULL
);
CREATE INDEX matches_start ON matches (game_start_ms DESC);
CREATE INDEX matches_patch ON matches (patch, queue_id);

-- Verbatim API responses, zstd-compressed, so metrics can be recomputed
-- without refetching.
CREATE TABLE match_raw (
    match_id   TEXT    NOT NULL REFERENCES matches (match_id) ON DELETE CASCADE,
    kind       TEXT    NOT NULL CHECK (kind IN ('match', 'timeline')),
    codec      TEXT    NOT NULL DEFAULT 'zstd',
    body       BLOB    NOT NULL,
    raw_len    INTEGER NOT NULL,
    fetched_at INTEGER NOT NULL,
    PRIMARY KEY (match_id, kind)
);

CREATE TABLE match_participants (
    match_id            TEXT    NOT NULL REFERENCES matches (match_id) ON DELETE CASCADE,
    participant_id      INTEGER NOT NULL,   -- 1..10
    puuid               TEXT    NOT NULL,
    key_gen_id          INTEGER NOT NULL REFERENCES key_generations (id),
    riot_id_name        TEXT,
    riot_id_tag         TEXT,
    team_id             INTEGER NOT NULL,
    champion_id         INTEGER NOT NULL,
    champion_name       TEXT    NOT NULL,
    team_position       TEXT,
    individual_position TEXT,
    win                 INTEGER NOT NULL,
    kills               INTEGER NOT NULL,
    deaths              INTEGER NOT NULL,
    assists             INTEGER NOT NULL,
    champ_level         INTEGER NOT NULL,
    gold_earned         INTEGER NOT NULL,
    cs                  INTEGER NOT NULL,   -- lane minions + neutral monsters
    vision_score        INTEGER NOT NULL,
    PRIMARY KEY (match_id, participant_id)
);
CREATE INDEX match_participants_riot_id
    ON match_participants (riot_id_name COLLATE NOCASE, riot_id_tag COLLATE NOCASE);
CREATE INDEX match_participants_puuid ON match_participants (puuid);

-- Which tracked account played which participant slot.
CREATE TABLE account_matches (
    account_id     INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    match_id       TEXT    NOT NULL REFERENCES matches (match_id) ON DELETE CASCADE,
    participant_id INTEGER NOT NULL,
    PRIMARY KEY (account_id, match_id)
);
CREATE INDEX account_matches_match ON account_matches (match_id);

-- Persistent job queue. A dedupe_key that's queued/running is never
-- duplicated; one that's done/failed is re-queued on enqueue.
CREATE TABLE jobs (
    id           INTEGER PRIMARY KEY,
    kind         TEXT    NOT NULL,
    payload      TEXT    NOT NULL,          -- JSON
    status       TEXT    NOT NULL DEFAULT 'queued'
                 CHECK (status IN ('queued', 'running', 'done', 'failed')),
    priority     INTEGER NOT NULL DEFAULT 0, -- higher runs first
    attempts     INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 5,
    next_run_at  INTEGER NOT NULL,
    last_error   TEXT,
    dedupe_key   TEXT    NOT NULL UNIQUE,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
CREATE INDEX jobs_ready ON jobs (status, priority DESC, next_run_at);

CREATE TABLE ddragon_versions (
    version    TEXT    PRIMARY KEY,   -- e.g. 16.20.1
    fetched_at INTEGER NOT NULL,
    complete   INTEGER NOT NULL DEFAULT 0
);

-- Non-secret settings only. Secrets live in Windows Credential Manager.
CREATE TABLE settings (
    key        TEXT    PRIMARY KEY,
    value      TEXT    NOT NULL,      -- JSON
    updated_at INTEGER NOT NULL
);

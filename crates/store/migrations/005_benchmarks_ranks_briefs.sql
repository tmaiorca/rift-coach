-- Phase 5: higher-rank benchmark collection, ranks, pre-game briefs.

-- Matches collected as benchmarks carry the tier they were sampled for
-- (e.g. DIAMOND). NULL = a game of a tracked or looked-up player ("your lobbies").
ALTER TABLE matches ADD COLUMN benchmark_tier TEXT;
CREATE INDEX matches_benchmark ON matches (benchmark_tier, patch);

-- Players sampled from the ladder. PUUIDs are encrypted per API key, so
-- they're only valid for the key generation that fetched them.
CREATE TABLE bench_players (
    key_gen_id  INTEGER NOT NULL REFERENCES key_generations (id) ON DELETE CASCADE,
    puuid       TEXT    NOT NULL,
    platform    TEXT    NOT NULL,
    tier        TEXT    NOT NULL,
    focus_score INTEGER,                      -- focus champions in their top masteries (NULL = not checked)
    scanned_at  INTEGER,                      -- match list fetched
    added_at    INTEGER NOT NULL,
    PRIMARY KEY (key_gen_id, puuid)
);
CREATE INDEX bench_players_next ON bench_players (key_gen_id, tier, scanned_at, focus_score);

-- Benchmark match ids waiting to be fetched (resumable across restarts).
CREATE TABLE bench_queue (
    match_id  TEXT    PRIMARY KEY,
    tier      TEXT    NOT NULL,
    priority  INTEGER NOT NULL DEFAULT 0,     -- higher first (focus champion players)
    status    TEXT    NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'done', 'skipped')),
    added_at  INTEGER NOT NULL
);
CREATE INDEX bench_queue_next ON bench_queue (status, priority DESC, added_at);

-- Seeding runs per key generation and tier.
CREATE TABLE bench_seeds (
    key_gen_id INTEGER NOT NULL REFERENCES key_generations (id) ON DELETE CASCADE,
    tier       TEXT    NOT NULL,
    seeded_at  INTEGER NOT NULL,
    players    INTEGER NOT NULL,
    PRIMARY KEY (key_gen_id, tier)
);

-- Ranked standing of tracked accounts (League-V4).
CREATE TABLE ranks (
    account_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    queue_type TEXT    NOT NULL,              -- RANKED_SOLO_5x5 | RANKED_FLEX_SR
    tier       TEXT,
    division   TEXT,
    lp         INTEGER NOT NULL,
    wins       INTEGER NOT NULL,
    losses     INTEGER NOT NULL,
    fetched_at INTEGER NOT NULL,
    PRIMARY KEY (account_id, queue_type)
);

-- The pre-game brief built at the loading screen (champions only).
ALTER TABLE live_games ADD COLUMN brief_json TEXT;

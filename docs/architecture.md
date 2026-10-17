# Architecture

## Process model

One Windows process (`rift-coach.exe`, Tauri 2). The UI runs in the system
WebView2. Rust owns everything else: Riot ingestion, the job queue, storage,
and later analytics, AI calls, the live poller and the MCP server. The UI calls
Rust through Tauri commands (`src-tauri/src/commands.rs`) and receives progress
as `engine-event` events.

```
UI (React)  --invoke-->  commands.rs  -->  Engine (rc-jobs)
                                             |-- RiotClient --> RateLimiter --> Riot Web API
                                             |-- DdragonCache --> Data Dragon CDN (+ disk cache)
                                             '-- Db (rc-store, SQLite WAL)
```

`rc-cli` builds the identical engine (`rc_jobs::build_engine`) for headless use.

## Rate limiting

`RateLimiter` keeps one bucket per routing value (`na1`, `americas`, ...) for
the application and one per (routing, method). Each bucket holds sliding-log
windows learned from `X-App-Rate-Limit` / `X-Method-Rate-Limit` headers,
starting from the Personal-key defaults (20/1s, 100/120s). A sliding log never
exceeds `limit` in any `period`-long interval, so it stays inside Riot's fixed
windows wherever their boundaries fall. A 100 ms pad and a 0.95 safety factor
absorb clock skew. The `-Count` headers pull in calls made by other processes
using the same key. A 429 blocks the indicated scope for `Retry-After`.

Measured on 2026-10-17: 100 matches + 100 timelines (~201 calls) in 245 s,
with zero 429s.

## Job queue

The `jobs` table is the queue. Rows are claimed atomically with
`UPDATE ... RETURNING` (highest priority, then due time). Outcomes:

- success → `done`
- transient error → back to `queued` with exponential backoff (15 s doubling, capped at 30 min)
- permanent error, or attempts exhausted → `failed`
- key missing or rejected → requeued *without* using an attempt, and all
  workers pause until the key changes

On startup, `running` rows (interrupted by a crash or close) return to
`queued`. A `dedupe_key` that is already queued or running can't be added
twice; re-enqueuing a finished one re-queues it. Because handlers are
idempotent, re-running ingest only fetches what's missing.

Ingestion: `ResolveAccount` → `SyncMatchList` → `FetchMatch` (×N) →
`FetchTimeline` (×N) + `DdragonSync` (per patch). Matches outrank timelines,
so the list fills first.

## Identity and key changes

PUUIDs are encrypted per API key. The schema never keys on them:

- `matches.match_id` is the match identity.
- `accounts` has an internal id plus Riot ID (case-insensitive unique).
- `account_puuids(account_id, key_gen_id)` and `match_participants.key_gen_id`
  record which key produced each PUUID.
- `key_generations` gets a new row when Account-V1 returns a different PUUID
  for an account already resolved under the current generation. Every
  tracked account is then re-resolved. Nothing derived from the key is
  stored.
- `account_matches` links accounts to matches by PUUID within a generation,
  or by Riot ID across generations.

## Storage

SQLite in WAL mode at `%LOCALAPPDATA%\RiftCoach\rift-coach.db`. Raw match and
timeline JSON is kept zstd-compressed (about 60 KB per match with timeline),
so later phases can recompute metrics without refetching. Logs are daily-rotated
JSON files in `logs\` (14 kept). Data Dragon files live in
`cache\ddragon\<version>\`.

## Secrets

`Secret` cannot be formatted or serialized. The client sends the key as an
`X-Riot-Token` header marked sensitive. Key lookup order: process environment,
then the user environment in the registry (`HKCU\Environment`). The first-run
wizard (Phase 6) will add Windows Credential Manager storage.

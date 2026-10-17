# Architecture

## Process model

One Windows process (`rift-coach.exe`, Tauri 2). The UI runs in the system
WebView2. Rust owns everything else: Riot ingestion, the job queue, storage,
and later analytics, AI calls, the live poller and the MCP server. The UI calls
Rust through Tauri commands (`src-tauri/src/commands.rs`) and receives progress
as `engine-event` events.

```
UI (React)  --invoke-->  commands.rs  -->  Engine (rc-jobs)
                             |               |-- RiotClient --> RateLimiter --> Riot Web API
                             |               |-- DdragonCache --> Data Dragon CDN (+ disk cache)
                             |               |-- rc-metrics (pure) --> participant_metrics
                             |               '-- Db (rc-store, SQLite WAL)
                             '--> rc-coach --> rc-ai --> Anthropic / OpenAI / Gemini / OpenAI-compatible
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
so the list fills first. Each stored match and timeline queues
`ComputeMetrics`; with auto review on, a fresh game (ended < 6 h ago) of the
active account then queues `AiReview`.

## Metrics and comparisons

`rc-metrics` turns raw match + timeline JSON into metrics for all ten
participants (registry in `defs.rs`), stored in `participant_metrics` as
(match, participant, metric, value) rows. `match_metrics_state.version`
records `METRICS_VERSION`; on start the engine recomputes stale matches. If
Data Dragon item data is unavailable, metrics are stored as version 0 (no item
timings) and completed on a later start.

Comparisons (`rc-coach::analysis`) use two populations:
- *role*: other players in the same role across the account's stored Rift
  games (mostly lane opponents, roughly the player's bracket), excluding the
  account's own rows;
- *personal*: the account's previous games in that role, limited to the
  look-back window.

`zero_sum` metrics (lane diffs) skip the role population, since opponents'
values mirror the player's. Higher-rank benchmarks are Phase 5.

## AI reviews

`rc-coach::review` builds a facts object (`facts.rs`), sends it with the
versioned prompt (`prompts/post_game_review.v1.md`) via `rc-ai`, and validates
the reply (`validate.rs`): structure, evidence ids, and every number and
mm:ss time in the prose must trace to the facts (the next-game goal may set a
new target). A structural failure retries once with feedback; untraceable
numbers after the retry store the report as `unverified`. Reports live in
`ai_reports` (history kept); the newest one for the same prompt version,
provider and model is reused unless the user regenerates.

Providers: Anthropic Messages API, OpenAI Chat Completions (JSON mode),
Gemini `generateContent` (JSON MIME type), and OpenAI-compatible servers
(custom base URL, key optional). Keys come from Credential Manager per
provider (`RiftCoach:ai_<provider>`).

## UI state and history

Everything that should survive a restart is in the database: settings
(`settings.app`), navigation (`settings.ui_state`), the activity feed
(`activity_log`, last 500 lines, written by the engine from its events), and
reports. The window plugin remembers size and position. The UI is a single
page with a sidebar; the ambient background (`AmbientBackground.tsx`) uses one
canvas at ~30 fps and stops when hidden or when reduced motion is requested.

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
`X-Riot-Token` header marked sensitive. Key lookup order: Windows Credential
Manager (`RiftCoach:riot_api_key`, set from Settings), the process
environment, then the user environment in the registry (`HKCU\Environment`).
AI keys live only in Credential Manager. No command returns a key.

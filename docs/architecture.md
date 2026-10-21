# Architecture

## Process model

One Windows process (`rift-coach.exe`, Tauri 2). The UI runs in the system
WebView2. Rust owns everything else: Riot ingestion, the job queue, storage,
analytics, AI calls, the live poller, and later the MCP server. The UI calls
Rust through Tauri commands (`src-tauri/src/commands.rs`) and receives progress
as `engine-event` events and live view updates as `live-update` events. A
second window (label `live`) loads the same page and renders only the live
board.

```
UI (React)  --invoke-->  commands.rs  -->  Engine (rc-jobs)
                             |               |-- RiotClient --> RateLimiter --> Riot Web API
                             |               |-- DdragonCache --> Data Dragon CDN (+ disk cache)
                             |               |-- rc-metrics (pure) --> participant_metrics, death_events
                             |               '-- Db (rc-store, SQLite WAL)
                             '--> rc-coach --+--> rc-signals (pure): patterns, win links, sessions, goal progress
                                             '--> rc-ai --> Anthropic / OpenAI / Gemini / OpenAI-compatible

live poller (rc-jobs::live) --> LiveClient (rc-live) --> https://127.0.0.1:2999 (game client)
      |-- Tracker (pure): idle / loading / in game / ended
      |-- Engine::set_in_game  (holds the queue while playing)
      |-- rc-live::view (pure): pacing, build progress, goal values  --> watch channel --> UI
      |-- footprint Sampler (sysinfo): this process + WebView2 children
      '-- on end: live_games row, Job::PostGame
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
`queued`. A `dedupe_key` that is already queued can't be added twice;
re-enqueuing a running one sets `rerun`, so it runs once more when it
finishes (its inputs may have changed: metrics that started before the
timeline arrived). Re-enqueuing a finished one re-queues it. Because handlers
are idempotent, re-running ingest only fetches what's missing.

Ingestion: `ResolveAccount` → `SyncMatchList` → `FetchMatch` (×N) →
`FetchTimeline` (×N) + `DdragonSync` (per patch). Matches outrank timelines,
so the list fills first. Each stored match and timeline queues
`ComputeMetrics`; with auto review on, a fresh game (ended < 6 h ago) of the
active account then queues `AiReview`.

## Benchmark collection

`rc-jobs::bench` runs a single background task, not queue jobs, so its pace is
easy to bound. Each step makes at most two Riot calls and is followed by
`benchmarks.step_seconds` (default 5 s), keeping it under half of a Personal
key's 100 calls per 2 minutes. A step only runs when no game is active, no
user job is queued or running, and the key is usable. All state is in SQLite
(`bench_players`, `bench_queue`, `bench_seeds`), so it resumes after a
restart. Steps, in order of need:

1. **Seed** (weekly, per key generation and tier): League-V4 entries for the
   tier (Diamond I and II pages, or the Master/GM/Challenger lists) into
   `bench_players`. PUUIDs there are only valid for their key generation.
2. **Fetch** the highest-priority queued match: skip it if stored, older than
   the current patch, or a remake. Otherwise remove other players' names from
   the raw JSON, store it with `matches.benchmark_tier`, fetch the timeline,
   and queue `ComputeMetrics` (metrics for all ten players). If a tracked
   account is in the game, it stays an ordinary game.
3. **Scan** a player known to main a focus champion.
4. **Check** a player's top 10 masteries (`focus_score`), alternating with
   step 5 while role samples are also short.
5. **Scan** any player: their solo queue match ids from the last 14 days into
   `bench_queue` (priority = focus score).

Targets per current patch (`rc_coach::benchmarks`): 300 players in each main
role and 50 per focus champion. Every 50 fetches, games outside the newest two
patches or beyond `max_games` are pruned. "Your lobbies" populations
(`role_population`, death zones) exclude benchmark games. Benchmark
populations (`rc_store::bench::population`) are per tier, patch, role and
optional champion. `pick_patch` uses the game's own patch when it has 30 role
samples, else the newest patch that does.

## Weekly reports and the pre-game brief

`rc_coach::weekly` groups games into local Monday–Sunday weeks (chrono
`Local`). It summarizes a week against the four before, using the same
`summarize` as sessions, plus a per-week trend and benchmark gaps for the
week's main role. The report (`weekly_report.v1`, scope `week:<monday>`) uses
the review format and validator. The 10-minute checker queues `WeeklyReport`
for the last complete week when `ai.auto_weekly` is on. It also refreshes
ranks older than 6 hours (`FetchRank`, also queued after every match-list
sync).

The pre-game brief is built by the live poller. On the loading screen (or at
game start, if the app opened mid-game) it spawns `build_brief`: Spectator-V5
for each tracked account (active first), keeping only champion ids and teams.
`rc_coach::pregame::brief` then adds:

- your record on the champion;
- a likely lane opponent: the enemy most often in your role across stored
  games;
- your record against it in lane and against each enemy;
- top weakness patterns, goals, and benchmark targets.

It's published in `LiveState.brief` and stored on the `live_games` row. The
optional AI plan (`pregame_brief.v1`, its own format checked by
`validate_plan`) is only started on the loading screen: automatically with
`ai.auto_pregame`, or by `LiveCommand::GeneratePlan` from the UI. Both run
off the poll loop and report back over a channel.

## Live games and game-aware scheduling

`rc-live` reads `/liveclientdata/allgamedata` and keeps only the active
player's own entry (`AllGameData::own`); other players are dropped while
parsing. The game serves it with a certificate from Riot's own root CA that
doesn't name `127.0.0.1`, so `LiveClient` skips certificate checks, and to
keep that safe it refuses any non-loopback URL and never sends credentials.
`RIFT_COACH_LIVE_URL` points it elsewhere (the `rc-cli mock-live` server).

The pure `Tracker` turns polls into transitions:

- no listener → `Absent`; a response without player data → `NotReady`
  (loading screen, spectating); data → in game;
- a `GameEnd` event ends the game; a game whose endpoint vanishes for 2
  minutes without one ends as `disappeared` (shorter gaps are reconnects);
- game time going backwards starts a new game.

Polls run every 2 s in a game and back off from 2 s to 15 s when idle. While
loading or in game, `Engine::set_in_game(true)` holds the queue (a running
job finishes; nothing new starts) and the session-review checker skips; the
UI freezes the ambient background.

On `Started`, the poller records a `live_games` row (account matched by Riot
ID) and builds the static `LiveContext` once (`rc_coach::live::context`): the
CS target (setting → CS/min goal → own role average), a 70th-percentile
benchmark from the role population, the planned or usual build, and the
active goals. Item data comes from the cached Data Dragon files of the newest
stored patch. Each poll then computes pacing, build progress (components owned
count toward the next item via the recipe tree) and goal values, and
publishes a `LiveState` on a `watch` channel. Own snapshots go to
`live_snapshots` every 15 s of game time. The footprint sampler reads CPU and
memory for this process and its descendants every 5 s.

On `Ended`, the row gets the result and footprint, and (for tracked accounts,
except practice tool games) `Job::PostGame` is queued a minute later. It lists
the account's newest match IDs, fetches any not stored, and links the match
that started within 5 minutes of the live game. Until Riot publishes it, it
retries with backoff (10 attempts, about 1.5 hours). The usual
timeline → metrics → auto review chain follows. When metrics with the
timeline are computed for a linked game, the engine emits `PostGameReady`
(once, via `analyzed_at`) and prunes snapshots past the retention setting.
The app shows a Windows notification for that, or for `ReviewReady` of a
linked game.

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

Alongside the metrics, every champion death of every player is stored in
`death_events` (time, position, map zone relative to the victim's team,
enemies involved, solo, laning phase, enemy side, gold vs lane opponent,
unspent gold, shutdown, epic monster the enemy took within 60 s, killer
champion). Zones (`rc_metrics::zones`) are straight-line bands over the map.

## Signals (patterns), win links, goals, sessions

`rc-signals` is pure and unit-tested; `rc-coach::patterns` loads its input
(the account's games with metrics and deaths, plus per-role populations of
metrics and death zones from the other players in the account's games).

- *Rule detectors* (`rules.rs`): a per-game test plus thresholds (min share of
  considered games, ≥ 5 considered, ≥ 3 affected). The same tests produce
  per-game flags (match screen, session games, AI facts).
- *Cross-game detectors* (`cross.rs`): median per-game percentile vs the role
  population (≤ 30 weakness, ≥ 70 strength, ≥ 8 games, population ≥ 20;
  metrics a rule already covers are skipped); death zones vs the role's zone
  distribution weighted per death; session effects (after a loss, 4th+ game).
- Every signal carries counts, win rate affected vs not, evidence (match,
  time, position), and an optional goal suggestion. Score = strength ×
  √affected × (1 + win-rate gap / 50).
- *Win links* (`correlation.rs`): Mann-Whitney U (AUC + tie-corrected normal
  p-value) of each metric in wins vs losses, ≥ 12 games and ≥ 4 of each;
  early metrics (≤ 15:00) are flagged because late ones mostly reflect the
  result.
- *Sessions*: consecutive games with gaps ≤ `session_gap_min`; a session's id
  and AI report scope is `session:<first match id>:<game count>`.
- *Goals* (`goals` table): metric, comparator, target, N games, optional
  role/champion, baseline at creation. Progress counts the Rift games started
  after `created_at`; a goal auto-completes (achieved = average meets target)
  when N games are played.

The engine checks every 10 minutes (and at start) whether the active
account's latest session has ended and queues `SessionReview` if
`ai.auto_session` is on.

## AI reviews

`rc-coach::review` builds a facts object (`facts.rs`), sends it with the
versioned prompt (`prompts/post_game_review.v2.md`, `prompts/session_review.v1.md`)
via `rc-ai`, and validates
the reply (`validate.rs`): structure, evidence ids, and every number and
mm:ss time in the prose must trace to the facts (the next-game goal may set a
new target). A structural failure retries once with feedback; untraceable
numbers after the retry store the report as `unverified`. Reports live in
`ai_reports` (history kept, keyed by match or by `scope` for sessions); the
newest one for the same prompt version, provider and model is reused unless the
user regenerates. Post-game facts include this game's pattern flags (with
recurrence) and active goals; session facts include each game, session
averages vs the prior baseline, session patterns vs their usual rate, and
goals. Before any call, tokens used in the last 30 days are checked against
`ai.monthly_token_limit` (`CoachError::Budget`).

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
canvas at ~30 fps and stops when hidden, when reduced motion is requested, or
while a game is loading or running.

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
The MCP access token (`RiftCoach:mcp_token`) is the one exception to "no
command returns a key": `mcp_token` hands it to the UI only to put it on the
clipboard, on an explicit click.

## MCP server

`rc-mcp` exposes read-only tools over MCP (JSON-RPC 2.0). `protocol.rs`
handles `initialize`, `ping`, `tools/list` and `tools/call`; `tools.rs` maps
each tool to the same `rc-coach` views the UI uses (`get_match` returns the
post-game AI facts object, so agents see exactly what reviews see: no other
players' Riot IDs). Two transports:

- **stdio** (`rift-coach.exe --mcp-stdio`): Claude Desktop starts its own
  process, which opens the database with `Db::open_read_only` (no migrations,
  refuses a schema-version mismatch) and never starts Tauri, workers or the
  poller. WAL lets it read while the app writes.
- **Streamable HTTP** (`POST /mcp`, JSON responses, no SSE) inside the app,
  restarted by `mcp_host::apply` whenever the `mcp` settings change. Loopback
  by default; LAN mode binds `0.0.0.0` and requires the bearer token. Requests
  with a non-loopback `Origin` are refused, and on loopback a non-loopback
  `Host` too (DNS rebinding). Tokens compare in constant time.

## Desktop shell, updates and data management

- **Single instance**: a second launch focuses the running window.
- **Tray / close to tray**: closing the main window hides it while
  `desktop.close_to_tray` is on (mirrored into an `AtomicBool` for the
  synchronous window event); Quit is on the tray menu.
- **Start with Windows**: `tauri-plugin-autostart` writes the HKCU Run entry
  with `--minimized`, which starts with the main window hidden. Windows is the
  source of truth, so it isn't a setting.
- **Updates**: `tauri-plugin-updater` reads `latest.json` from the newest
  GitHub release and verifies the minisign signature against the public key in
  `tauri.conf.json` before running the NSIS installer (passive mode). The
  private key is never in the repository (a CI secret for releases).
- **Backup / restore / delete** (`rc_store::maintenance`): backup is
  `VACUUM INTO` (consistent while the app runs). Restore and delete are staged
  as files (`restore-pending.db`, `delete-pending`) and applied by
  `apply_pending` at the next start, before the database or log file is
  opened; the app restarts itself to get there, retrying for a few seconds
  while the old process exits.
- **First run**: the setup overlay shows until `desktop.onboarded`; installs
  that already have an account are marked onboarded on start.

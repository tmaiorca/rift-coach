# Rift Coach

Local League of Legends performance coach for Windows. Tauri 2 desktop app
with a Rust backend and a React/TypeScript UI. All data stays on the user's
PC. Full product spec: `SPEC.md`. Architecture notes: `docs/architecture.md`.

## Hard rules

- **Never print, log, write to a file, or commit the Riot API key** (or any
  key/token). It lives in the `RIOT_API_KEY` *user* environment variable
  (later: Windows Credential Manager). Wrap keys in `rc_common::Secret`
  (Debug/Display print `[REDACTED]`, no `Serialize`). Send keys only as
  sensitive headers, never in URLs. Don't store anything derived from a key.
  To check presence in a shell, print "present"/"missing" only.
- **PUUIDs are encrypted per API key.** Never use a PUUID as a primary key.
  Matches are keyed by `match_id`; accounts by internal id + Riot ID. Every
  stored PUUID carries `key_gen_id`. A key change is detected when Account-V1
  returns a different PUUID for a resolved account (`record_resolved_puuid`).
- **Riot policy / game integrity.** Riot bans third-party tools from "drawing
  conclusions for you during gameplay", exposing obfuscated info, and
  automation. Live (in-game) features may show only the player's own factual
  numbers and static pre-game content: no AI reacting to live state, no
  enemy timers/cooldowns, no alerts, no input automation, no memory reading
  (blocked by Riot since 2026-10-06). AI runs only pre-game (after loading
  screen) and post-game. No MMR/ELO estimates. Vision review may only *save*
  screenshots of the game window during a game: nothing reads, analyzes or
  shows anything from them until the game has ended. Flag, don't build, anything
  that crosses these lines. LCU use requires product registration first.
- **AI keys** follow the same rules: stored only in Windows Credential Manager
  (`rc_common::credentials`, entries `RiftCoach:<name>`), sent only as sensitive
  headers, scrubbed from provider error text. `max_tokens` in requests/settings is the answer length; `rc-ai` adds
  `THINKING_ROOM` for frontier providers because thinking counts against their output limit. The AI gets only the facts object
  (`rc_coach::facts`), never raw API dumps or other players' Riot IDs.
- **Keep `README.md` current.** It is both the GitHub README and the in-app
  Guide page (`ui/src/pages/Guide.tsx` imports it). Update it in the same change
  as any user-visible behavior, setting, page, or metric.
- **Tests make no live API calls.** Use `wiremock` and the scrubbed fixtures
  in `tests/fixtures/riot`.
- All Riot calls go through `RiotClient` → the shared `RateLimiter`. Never
  bypass it. A Personal key allows 20 req/1s and 100 req/2min per routing value.

## Commands (PowerShell)

If `cargo` isn't on PATH in an old shell: `$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"`.

| Task | Command |
|---|---|
| Run the desktop app (dev) | `npm run tauri dev` |
| All Rust tests | `cargo test --workspace` |
| Rust lint (CI-strict) | `cargo clippy --workspace --all-targets -- -D warnings` |
| Rust format | `cargo fmt --all` |
| UI lint + typecheck | `npm run lint` |
| UI format | `npm run format` / `npm run format:check` |
| Ingest via CLI | `cargo run -p rc-cli -- ingest "TapRooted#BTC" --count 100` |
| Metrics + percentiles for a game | `cargo run -p rc-cli -- analyze [MATCH_ID]` (`--facts` prints the AI input) |
| Patterns, win links, sessions | `cargo run -p rc-cli -- patterns --games 100 [--role MIDDLE] [--sessions]` |
| Benchmark progress and gaps | `cargo run -p rc-cli -- benchmarks [--collect MINUTES] [--role MIDDLE] [--champion Leblanc]` |
| Mock live game (no League needed) | `cargo run -p rc-cli -- mock-live --speed 10 [--mode CLASSIC] [--champion Ahri]` (defaults: primary account, most played champion), then run the app with `$env:RIFT_COACH_LIVE_URL = "http://127.0.0.1:2998"` |
| MCP over stdio (read-only, real DB) | `cargo build -p rift-coach` then pipe JSON-RPC lines into `target\debug\rift-coach.exe --mcp-stdio` |
| Vision capture cost (any window) | `cargo run --release -p rc-cli -- vision-capture --window "<title part>" [--interval 3] [--width 1280]` |
| Vision review of a recorded game now (real AI call; deletes its frames on success) | `cargo run --release -p rc-cli -- vision-review [MATCH_ID]` |
| Real capture through the poller | `$env:RIFT_COACH_VISION_WINDOW = "<title part>"; cargo test -p rc-jobs --test live -- --ignored vision` (the window must keep redrawing) |
| Installer (release build) | `npm run tauri build` → `target\release\bundle\nsis\`; set `TAURI_SIGNING_PRIVATE_KEY` (contents of `%USERPROFILE%\.tauri\rift-coach-updater.key`) to also sign the updater artifacts |
| Status / role summary | `cargo run -p rc-cli -- status` / `-- summary` |
| Endpoint smoke test (live) | `cargo run -p rc-cli -- smoke "TapRooted#BTC"` |
| Record + scrub fixtures | `cargo run -p rc-cli -- record-fixtures "TapRooted#BTC"` then `-- scrub-fixtures --keep "TapRooted#BTC"` |

Set `RIFT_COACH_HOME` to use a different data folder (e.g. a scratch dir), instead of
`%LOCALAPPDATA%\RiftCoach`. `RUST_LOG` overrides log filters. `RIFT_COACH_LIVE_URL`
points the live poller at another (loopback-only) Live Client URL. `RIFT_COACH_VISION_WINDOW` makes vision capture use
the window whose title contains it instead of the League game window.

## Layout

- `crates/common` (`rc-common`): paths, rotating JSON logging, `Secret`, queue constants
- `crates/riot` (`rc-riot`): routing, `RateLimiter`, `RiotClient`, partial DTOs
- `crates/store` (`rc-store`): SQLite (WAL) + migrations in `crates/store/migrations/`, data access
- `crates/ddragon` (`rc-ddragon`): Data Dragon cache, versioned by patch
- `crates/metrics` (`rc-metrics`): pure metric computation from match/timeline JSON, metric registry (`defs.rs`), percentiles, chart/map views
- `crates/signals` (`rc-signals`): pure cross-game detectors (`rules.rs` per-game rules, `cross.rs`
  percentile/death-zone/session detectors), win correlation, session grouping, goal progress
- `crates/live` (`rc-live`): Live Client Data API client (own entry only), pure game `Tracker`,
  live view math (`view.rs`: pacing, build progress, goal values), footprint sampler, scripted `mock` game
- `crates/ai` (`rc-ai`): LLM providers (Anthropic, OpenAI, Gemini, OpenAI-compatible) behind `AiClient`
- `crates/coach` (`rc-coach`): comparisons vs populations (lobby, personal, benchmark), match detail view,
  patterns/goals/sessions/weeks (loads store data into rc-signals), benchmarks (focus, targets, gaps), pre-game
  brief, AI facts, prompts, validation, report generation (post-game, session, weekly, game plan, token cap)
- `crates/jobs` (`rc-jobs`): persistent job queue engine + handlers (ingest, metrics, auto review,
  post-game linking, rank, weekly report), `build_engine`, the live poller (`live.rs`, `spawn_live`, pre-game
  brief), the benchmark collector (`bench.rs`, `spawn_benchmarks`)
- `crates/vision` (`rc-vision`): game-window capture (Windows Graphics Capture via `windows-capture`, `capture.rs`), game
  clock, downscale + JPEG (`encode.rs`), frames on disk (`frames.rs`), frame selection around moments (`select.rs`).
  Review logic is `rc-coach::vision` (anchors from the timeline, facts, validation, name scrubbing)
- `crates/mcp` (`rc-mcp`): read-only MCP server: tools over `rc-coach` views, JSON-RPC protocol, HTTP transport
  (loopback default, LAN + bearer token), stdio transport (`--mcp-stdio`, read-only DB)
- `cli` (`rc-cli`): dev CLI. `src-tauri` (`rift-coach`): app shell + commands (`commands.rs`), tray/autostart/close-to-tray
  (`desktop.rs`), MCP lifecycle (`mcp_host.rs`), backup/restore/export/delete (`data.rs`), updater (`updates.rs`), vision commands (`vision.rs`). `ui/`: React app
  (`pages/` per screen, `components/` shared, `lib/` formatting + cached Data Dragon images)
- `prompts/`: versioned AI prompts (`<name>.v<N>.md`); bump the version (new file + `PROMPT_VERSION`) when changing one

## Conventions

- Rust 2024 edition, `thiserror` for library errors, `anyhow` only in the CLI.
  No `unwrap()` outside tests (clippy warns). Lock poisoning: `unwrap_or_else(PoisonError::into_inner)`.
- DB timestamps are unix **milliseconds** (`rc_common::time::now_ms`). Schema
  changes are new numbered files in `crates/store/migrations/`; never edit a shipped one.
- Raw Riot responses are stored zstd-compressed in `match_raw` and are the
  source of truth; derived tables can always be rebuilt from them.
- Jobs must be idempotent. Add a `Job` variant + handler; `dedupe_key()` defines uniqueness.
- Async code reaches SQLite via `Db::call` (blocking pool); never hold the lock across `.await`.
- UI types in `ui/src/types.ts` mirror Rust `Serialize` types; keep them in sync.
- Patch = `major.minor` from `gameVersion` (e.g. `16.20`); store it on every match.
- Non-standard queues (ARAM, Arena, etc.) are stored but excluded from Rift
  metrics via `rc_common::queues::is_standard_rift`.
- New metric: add it to `rc_metrics::defs::METRICS` and `compute.rs`, test it on
  fixtures, bump `METRICS_VERSION` (stored matches recompute on next start),
  and document it in README "How the numbers work". Lane-diff metrics are
  `zero_sum` (compared with the player's own games only).
- New pattern: a `Rule` in `rc_signals::rules::RULES` (per-game test + threshold) or a cross-game
  detector in `cross.rs`; unit-test it with synthetic games; document it in README "Patterns and
  what wins your games". Every number in a signal's `detail` must come from the data (the AI may quote it).
- Dev builds use `debug = "line-tables-only"` to keep `target/` small; the machine's C: drive is
  nearly full, so run `cargo clean` if builds fail with "no space on device".
- Benchmark games carry `matches.benchmark_tier`; any "players in your games" query must filter
  `benchmark_tier IS NULL`. Other players' names are stripped from benchmark raw JSON before storing.
  The collector must stay one throttled step at a time and yield to user jobs and games.
- The pre-game brief may use only champions/teams from Spectator (never other players' identities, ranks or
  histories). AI game plans start only on the loading screen.
- Live features: only the player's own entry is parsed (`AllGameData::own`); anything new on the
  live view must be the player's own number or a target fixed before the game. Nothing heavy runs
  while `Engine::in_game()`; a new background task must check it.
- Settings: `rc_store::settings::AppSettings` (serde defaults, so new fields are
  safe) mirrored in `ui/src/types.ts`. UI navigation state is free-form JSON
  under the `ui_state` key. No secrets in settings.
- Python one-off edits on Windows must use `encoding="utf-8", newline=""`.
- Nothing is LeBlanc/mid-specific: focus roles/champions, champion pages and the benchmark tier (`AUTO` = two tiers
  above the viewed account's solo rank, via `benchmarks::effective_tier`) all derive from the player's own games.
  Always use `effective_tier`, never `settings.benchmarks.tier` directly.
- MCP tools are read-only and must never return other players' Riot IDs or any key. Keep outputs compact (they go
  into an agent's context): cap lists like signals/evidence.
- Restore/delete are staged files applied by `rc_store::maintenance::apply_pending` before the DB opens; never
  swap the database under an open connection. New migrations go in `MIGRATIONS` in `crates/store/src/lib.rs`.
- Vision: frames live only in `vision/<live_game_id>/` and are deleted after the review (or a day after the game); never
  store images in the database or send them anywhere but the chosen vision provider after the game. Capture work must
  stay on the capture thread and drop undue frames before any GPU copy; never draw a capture border or overlay on the
  game. Other players' Riot ID names in a vision reply are replaced by champions (`scrub_names`).
- The updater's private signing key lives outside the repo (`%USERPROFILE%\.tauri\rift-coach-updater.key`,
  CI secret `TAURI_SIGNING_PRIVATE_KEY`). Never commit or print it.

## Phase status

- Phase 1 (foundation): done 2026-10-17, reviewed, committed.
- Phase 2 (metrics, baselines, match detail, charts, death map) plus user-requested
  extras (AI post-game review pulled forward from Phase 3, settings page,
  player lookup, ambient background, in-app guide, persisted activity/UI
  state): done 2026-10-17, reviewed.
- Phase 3 (patterns/signals, win correlation, death heat map, focus goals, session
  reviews, auto session review, AI token cap): done 2026-10-18, reviewed.
- Phase 4 (live poller, game-aware scheduling, live view + pop-out window, planned builds,
  post-game linking/analysis/notifications, footprint measurement): done 2026-10-18, reviewed.
- Phase 5 (benchmark collection by tier/patch with focus-champion sampling, benchmark percentiles
  and gaps, weekly pages + AI weekly report, loading-screen brief + AI game plan, ranks): done
  2026-10-18, reviewed.
- Phase 6 (MCP server stdio + HTTP/LAN with token, NSIS installer signed-ready, signed auto-update from
  GitHub releases, first-run setup, tray + start with Windows + single instance, backup/restore/CSV export/delete,
  plus user-requested generalization: Champions page, Auto benchmark tier, looser focus-champion rule): done
  2026-10-20, pushed to GitHub (tmaiorca/rift-coach) with release v0.1.0.
- Phase 7 (user-requested: vision review: screenshots of the game window during games, reviewed by a separate vision
  model after the game around deaths/objectives/checkpoints, then deleted; 0.2.0): done 2026-10-31, pending review.

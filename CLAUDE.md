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
  screen) and post-game. No MMR/ELO estimates. Flag, don't build, anything
  that crosses these lines. LCU use requires product registration first.
- **AI keys** follow the same rules: stored only in Windows Credential Manager
  (`rc_common::credentials`, entries `RiftCoach:<name>`), sent only as sensitive
  headers, scrubbed from provider error text. The AI gets only the facts object
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
| Status / role summary | `cargo run -p rc-cli -- status` / `-- summary` |
| Endpoint smoke test (live) | `cargo run -p rc-cli -- smoke "TapRooted#BTC"` |
| Record + scrub fixtures | `cargo run -p rc-cli -- record-fixtures "TapRooted#BTC"` then `-- scrub-fixtures --keep "TapRooted#BTC"` |

Set `RIFT_COACH_HOME` to use a different data folder (e.g. a scratch dir), instead of
`%LOCALAPPDATA%\RiftCoach`. `RUST_LOG` overrides log filters.

## Layout

- `crates/common` (`rc-common`): paths, rotating JSON logging, `Secret`, queue constants
- `crates/riot` (`rc-riot`): routing, `RateLimiter`, `RiotClient`, partial DTOs
- `crates/store` (`rc-store`): SQLite (WAL) + migrations in `crates/store/migrations/`, data access
- `crates/ddragon` (`rc-ddragon`): Data Dragon cache, versioned by patch
- `crates/metrics` (`rc-metrics`): pure metric computation from match/timeline JSON, metric registry (`defs.rs`), percentiles, chart/map views
- `crates/ai` (`rc-ai`): LLM providers (Anthropic, OpenAI, Gemini, OpenAI-compatible) behind `AiClient`
- `crates/coach` (`rc-coach`): comparisons vs populations, match detail view, AI facts, prompt, validation, report generation
- `crates/jobs` (`rc-jobs`): persistent job queue engine + handlers (ingest, metrics, auto review), `build_engine`
- `cli` (`rc-cli`): dev CLI. `src-tauri` (`rift-coach`): app shell + commands. `ui/`: React app
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
- Settings: `rc_store::settings::AppSettings` (serde defaults, so new fields are
  safe) mirrored in `ui/src/types.ts`. UI navigation state is free-form JSON
  under the `ui_state` key. No secrets in settings.
- Python one-off edits on Windows must use `encoding="utf-8", newline=""`.

## Phase status

- Phase 1 (foundation): done 2026-10-17, reviewed, committed.
- Phase 2 (metrics, baselines, match detail, charts, death map) plus user-requested
  extras (AI post-game review pulled forward from Phase 3, settings page,
  player lookup, ambient background, in-app guide, persisted activity/UI
  state): done 2026-10-17, pending review.
- Next: Phase 3: cross-game signal detection, win-correlation, focus goals, session reviews.

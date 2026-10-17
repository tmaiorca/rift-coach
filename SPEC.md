\# Project: Rift Coach (working name): local League of Legends performance coach for Windows



\## Context

I'm Troy. I play League of Legends (Riot ID: TapRooted#BTC, NA server). I want a Windows desktop

app that runs on my gaming PC and tracks my games like Mobalytics/Porofessor but goes much deeper.

It should explain \*why\* I win or lose and \*what specifically to fix next\*, using AI-generated

insights that are strictly grounded in real match data.



All data stays local on this PC. It should be packaged as a normal Windows installer so another

player could install it, enter their Riot ID and keys, and use it. No Docker, no server, no VM.



\## How to work

1\. Before writing any code, verify the Riot API and policy details in this spec against current

&#x20;  documentation at developer.riotgames.com. They change, and where this spec is wrong, tell me.

2\. Propose the architecture, tech stack, DB schema, and phase plan. Wait for my approval.

3\. Build one phase at a time. Each phase must end with something running that I can test. Stop

&#x20;  for review after each phase.

4\. Create and maintain a CLAUDE.md with conventions, commands, and architecture notes.



\## Data sources

1\. \*\*Live Client Data API (real-time, in-game only).\*\* Local HTTPS endpoint (127.0.0.1:2999,

&#x20;  self-signed cert, e.g. /liveclientdata/allgamedata). It only exists while a match is running.

&#x20;  It provides:

&#x20;  - my full stats, gold, and runes

&#x20;  - all players' items, levels, KDA, CS, and ward score

&#x20;  - an event feed

2\. \*\*Riot Web API (post-game, authoritative).\*\*

&#x20;  - Account-V1: Riot ID → PUUID

&#x20;  - Match-V5: match list, match detail, and \*\*timeline\*\*. The timeline has per-minute frames

&#x20;    (gold, XP, CS, position) and timestamped events (kills with coordinates, item purchases,

&#x20;    wards, objectives, skill-ups).

&#x20;  - League-V4: rank

&#x20;  - Spectator-V5: current-game participants, for pre-game scouting

&#x20;  - Champion-Mastery-V4

3\. \*\*Data Dragon / Community Dragon\*\* for static data (champions, items, runes, icons, minimap).

&#x20;  Cache it locally and version it by patch.



\*\*API key handling:\*\*

\- Start with a development key entered in settings. It expires every 24 hours, so show a clear

&#x20; "key expired" state.

\- Design for a Personal or Production key later.

\- Store keys securely (Windows Credential Manager or equivalent), never in plain text and never

&#x20; hardcoded.

\- Build one central rate limiter that respects both app-level and method-level limits and

&#x20; Retry-After headers.



\## Compliance (hard constraint)

Read Riot's current developer policies and League game-integrity rules, and design to them.

Post-game analysis is the core product.



Anything shown \*during\* a live game must:

\- use only information I legitimately already have

\- avoid enemy cooldown or timer tracking beyond what policy allows

\- involve no input automation



Nothing may de-anonymize players in champ select. If any feature in this spec conflicts with

policy, flag it instead of building it. The LCU (League Client) API is a stretch goal, and only

if it's compliant.



\## Architecture: single local Windows app

\- \*\*App shell:\*\* a desktop app with a system tray icon that starts with Windows (optional

&#x20; setting). Recommend between Tauri, Electron, or a local backend serving a web UI on

&#x20; localhost. Prioritize low resource use while a game is running.

\- \*\*Local backend service:\*\* handles Riot API ingestion, the live poller, the job queue, the

&#x20; analytics engine, and AI calls.

\- \*\*Storage:\*\* SQLite (WAL mode) in the user's AppData folder. Use DuckDB or similar only if

&#x20; analytics queries justify it. Live snapshots are stored per match, and raw snapshots are

&#x20; pruned after processing (configurable).

\- \*\*Job queue:\*\* in-process and persistent, so it survives restarts. No external services.

\- \*\*Game-aware scheduling:\*\*

&#x20; - detect when a match is running (poll the Live Client endpoint, back off when absent)

&#x20; - during a game, ONLY the lightweight live poller runs

&#x20; - heavy work (match fetch, timeline processing, AI analysis, benchmark collection) is queued

&#x20;   until the game ends

&#x20; - include a CPU/memory budget and measure the app's footprint during a game

\- \*\*On game end:\*\* fetch Match-V5 detail and timeline (retry with backoff, since the match takes

&#x20; a few minutes to appear), run analysis, and notify me when the review is ready.

\- \*\*MCP server:\*\*

&#x20; - local MCP endpoint exposing read-only tools (match history, metrics, signals, AI reviews,

&#x20;   goal progress) so Claude Desktop or my AI agent can query my data

&#x20; - bind to localhost by default

&#x20; - optional setting to expose it on the LAN with a token, for my Hermes agent on my home server



\## Analytics engine (the core of the product)



\### Layer 1: Deterministic per-game metrics, computed from timelines

\- \*\*Laning:\*\* CS@10 and @15, CS/gold/XP diff vs my lane opponent at 10 and 15, first-back

&#x20; timing and gold, deaths before 14:00, solo kills and deaths, lane-phase death context

\- \*\*Macro:\*\* kill participation, objective participation (dragons, grubs, herald, baron, towers),

&#x20; time spent dead, unspent gold at death, roam outcomes

\- \*\*Deaths:\*\* location heatmap and clustering, e.g. "deaths in enemy jungle with no nearby

&#x20; ward", "deaths while ahead", "deaths shortly after objectives spawn"

\- \*\*Vision:\*\* wards placed, killed, and control wards per minute; vision score per minute;

&#x20; ward placement map

\- \*\*Itemization:\*\* core item completion timings, and my build vs common builds for the matchup

\- \*\*Fighting:\*\* damage share vs gold share, damage taken, shutdown gold given up, throw

&#x20; detection (lead lost after X minutes)



Note: timeline positions are sampled about once per minute, so label positional analysis as

approximate. Use live snapshots for higher resolution where available.



\### Layer 2: Baselines and benchmarks

\- Rolling \*\*personal baselines\*\* per champion and role.

\- \*\*Higher-elo benchmarks:\*\* a background job samples recent matches from higher-ranked players

&#x20; (League-V4 → PUUIDs → Match-V5) on the same champion, role, and patch. It runs the same

&#x20; metric pipeline and stores the distributions.

&#x20; - runs only when no game is active

&#x20; - throttled well inside rate limits

&#x20; - resumable across restarts

\- Show every metric as a percentile vs my baseline \*and\* vs the benchmark tier.



\### Layer 3: Signal detection (deterministic and unit-tested)

Rules and statistics emit typed signals with evidence. Example:

`{type, severity, games\_affected, evidence: \[{match\_id, timestamp, coords}], metric\_values}`



\- Recurring cross-game patterns rank above one-off anomalies.

\- Include \*\*win-correlation analysis\*\*: which of \*my\* metrics actually move my win rate.



\### Layer 4: AI coach (LLM)

\- \*\*Input:\*\* structured signals, metrics, and context (champion, matchup, rank, patch). Never

&#x20; raw API dumps.

\- \*\*Output:\*\* strict JSON, validated against the data:

&#x20; - top 1–3 prioritized fixes

&#x20; - explanations citing specific evidence (match plus timestamp)

&#x20; - one concrete focus for my next N games

&#x20; - confidence

\- Every numeric claim must trace back to a metric or signal. Reject or regenerate output that

&#x20; invents stats.

\- \*\*Report types:\*\*

&#x20; - post-game review

&#x20; - session review (one play session)

&#x20; - weekly trend report

&#x20; - pre-game brief (from Spectator: my history in this matchup and vs these champions)

\- \*\*Focus goals:\*\* I set a goal (e.g. "average under 1.5 deaths before 14:00"). The app tracks

&#x20; progress game over game, and the AI evaluates whether I'm improving.

\- \*\*Pluggable LLM provider:\*\* Anthropic API by default, with the model configurable. Use a

&#x20; provider interface that also supports any OpenAI-compatible endpoint (e.g. a model I host on

&#x20; another machine on my LAN).

&#x20; - never run inference locally during a game

&#x20; - store prompts as versioned files

&#x20; - cache results per match plus prompt version



\## Live view

A compact window or overlay-free second-monitor view, kept simple and compliant:

\- CS/min pacing vs my target and benchmark

\- gold and progress toward my planned build

\- KDA and current focus-goal reminder



When the match ends, show the post-game review once Riot data is available.



\## UI

\- \*\*Dashboard:\*\* rank, recent games, focus-goal progress, top recurring signals

\- \*\*Match detail:\*\*

&#x20; - gold/XP/CS diff timeline charts

&#x20; - Summoner's Rift minimap overlay of deaths, kills, and wards, with a time scrubber

&#x20; - AI review

\- Champion and role pages, matchup history, trends over time

\- Dark mode, desktop-first



\## Packaging

\- A signed-ready Windows installer (MSI or NSIS) with auto-update support. Plan for code

&#x20; signing later.

\- \*\*First-run wizard:\*\* Riot ID and region, Riot API key, LLM provider and key, and the

&#x20; "start with Windows" option.

\- \*\*Data management:\*\* export and backup of the SQLite database, and a "delete all my data"

&#x20; option.

\- The data model supports multiple tracked Riot accounts (e.g. smurfs or a friend on the same

&#x20; PC).

\- Docs: README, architecture doc, CLAUDE.md, and a short user guide.



\## Engineering standards

\- Typed code, linting, structured logging to a rotating log file.

\- Unit tests for every metric and signal detector, using \*\*recorded fixtures\*\*. Save real API

&#x20; responses from my account as fixtures. Tests must make no live API calls.

\- Include a mock Live Client server for testing the live poller without launching League.

\- Patch-aware: store the patch on every match and segment benchmarks by patch.



\## Phases

1\. \*\*Foundation:\*\* project scaffold, SQLite schema, Riot client with rate limiter, Data Dragon

&#x20;  cache. Ingest my last 100 matches and timelines for TapRooted#BTC. Run it as a dev app.

2\. \*\*Metrics and baselines,\*\* plus basic UI: match list, match detail, timeline charts, death map.

3\. \*\*Signal detection,\*\* AI post-game review, focus goals.

4\. \*\*Live poller,\*\* game-aware scheduling, live view, auto-review on game end, footprint

&#x20;  measurement.

5\. \*\*Benchmark collection,\*\* percentile comparisons, weekly reports, pre-game brief.

6\. \*\*MCP server,\*\* installer, first-run wizard, auto-update, docs.



\## Before you start, ask me about

\- Which roles and champions I play most, to prioritize benchmark sampling

\- Language/framework preferences, if any

\- LLM budget: per-match analysis on every game, or batched

\- Whether I want MCP exposed to my LAN from day one


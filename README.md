# Rift Coach

A local League of Legends coach for Windows. It downloads your games from Riot,
computes detailed metrics for every one of them, compares you with the players
in your own games and with your own history, and (with your own AI key)
writes a grounded review of any game: what went well, what to fix, and one
focus for your next games. Everything stays on your PC.

> This guide is also the **Guide** page inside the app. It's one file
> (`README.md`), so the app and GitHub always show the same text.

**Status:** Phase 2 of 6 (metrics, baselines, match detail, AI post-game
review). See [Roadmap](#roadmap).

## Contents

- [Getting started](#getting-started)
- [Pages](#pages)
- [How the numbers work](#how-the-numbers-work)
- [The AI coach](#the-ai-coach)
- [Settings](#settings)
- [Your data, keys and privacy](#your-data-keys-and-privacy)
- [Riot's rules and what the app won't do](#riots-rules-and-what-the-app-wont-do)
- [Troubleshooting](#troubleshooting)
- [For developers](#for-developers)
- [Roadmap](#roadmap)

## Getting started

1. **Riot API key.** Get one at [developer.riotgames.com](https://developer.riotgames.com).
   Either paste it in **Settings → Riot API key** (stored in Windows Credential
   Manager) or set the `RIOT_API_KEY` user environment variable.
2. **Add yourself.** Open **Players**, enter your Riot ID (e.g. `TapRooted#BTC`),
   pick your region, choose how many games to look back, and click **Look up**.
3. **Wait a few minutes.** With a personal key Riot allows about 100 requests
   every 2 minutes, so 100 games plus their timelines take about 4–5 minutes.
   The dashboard shows progress. Closing the app is safe: work resumes when
   you reopen it, and games already stored are never downloaded twice.
4. **Optional: AI reviews.** In **Settings → AI coach**, pick a provider, save
   your API key, choose a model, and click **Test connection**. Then open any
   match and click **Review this game**.

## Pages

**Dashboard.** Your last *N* Summoner's Rift games (*N* is the look-back
window in Settings): win rate, KDA, key metric averages with your last 5 games
alongside, a trend chart for any metric with a 5-game rolling average, your
champions, recent games, sync progress, Riot rate-limit meters, and the
activity log. The log is saved, so it's still there after a restart.

**Matches.** Every stored game, newest first, filterable by champion, role and
result. ARAM, Arena and other modes are shown dimmed (you can hide them) and
never count toward metrics. ✦ marks games with an AI review. Click a row to
open it.

**Match detail.**
- *AI coach*: the review for this game (see [The AI coach](#the-ai-coach)).
- *Metrics*: every metric for this game, grouped (laning, macro, fighting,
  economy, vision, items), each with two comparisons (see below).
- *Timeline*: gold, XP and CS minute by minute, as a difference against your
  lane opponent or both lines together, plus your team's gold lead. Red lines
  mark your deaths.
- *Map*: Summoner's Rift with your deaths, kills, assists, other kills,
  objectives and towers. Drag the time slider or press **Play** to watch the
  game unfold. Click a marker for details. Your deaths are listed with
  context: how many enemies were involved, laning phase or not, enemy side of
  the map, gold versus your opponent, and unspent gold.
- *Scoreboard*: all ten players with items, spells, damage and vision. Click
  another player's Riot ID to look them up.

**Players.** Track any number of Riot IDs (smurfs, friends, opponents). Switch
the player you're viewing from the top bar. "This is me" marks your own account.
Removing a player deletes their reviews but keeps the stored matches.

**AI reports.** The full history of every review, including regenerated ones,
with token usage.

**Settings.** See [Settings](#settings).

## How the numbers work

Metrics are computed by deterministic code from Riot's match and timeline
data for **all ten players** in every stored game, so they can be recomputed at
any time and are covered by unit tests.

| Group | Metrics |
|---|---|
| Laning | CS, gold and XP at 10 and 15 minutes; CS/gold/XP difference vs your lane opponent at 10 and 15; deaths before 14:00; solo kills and deaths; first back time and gold spent |
| Macro | Kill participation; objective participation (dragons, grubs, heralds, barons, towers); time dead; unspent gold at death; shutdown gold given up; biggest team gold lead |
| Fighting | KDA, kills, deaths, assists; damage per minute; damage share; gold share; damage share ÷ gold share; damage taken share |
| Economy | CS per minute; gold per minute |
| Vision | Vision score per minute; wards placed and cleared per minute; control wards |
| Items | Time of first completed (non-boots) item |

**Comparisons.** Each metric shows "better than X%" against two groups:

- **Players in your role in your games.** Everyone else who played that role
  in your stored games. These are mostly your lane opponents, so they're
  roughly your skill bracket. It's not a global ladder.
- **Your games.** Your previous games in that role, within the look-back window.

For metrics where lower is better (deaths, time dead), "better" means lower.
Lane differences (CS/gold/XP diff) are compared only with your own games,
because your opponents' differences are the mirror image of yours. Metrics
marked ≈ are approximate, since Riot samples positions and gold about once per
minute. Ward positions aren't in Riot's data, so the map can't show wards.

## The AI coach

- **What it sees.** Only the computed facts for that game: the metrics, the
  comparisons, your deaths with context, champions, result and items. Click
  **Facts sent** on any match to see exactly what's sent. Other players' Riot
  IDs and raw API data are never sent.
- **What it returns.** A summary, what went well, what to improve, 1–3
  priority fixes with a concrete drill, one focus for your next games, and a
  confidence level. Each point cites the facts it's based on.
- **How it's checked.** Every number in the review is compared against the
  facts. If the reply is malformed, or cites facts that don't exist, it's
  rejected and the AI is asked again. If a number still can't be traced, the
  review is saved with an "unverified numbers" warning.
- **Caching and history.** A review is generated once per game, prompt version
  and model. **Regenerate** makes a new one, and every version is kept on the
  AI reports page.
- **Providers.** Anthropic (Claude), OpenAI, Google Gemini, or any
  OpenAI-compatible server, such as Ollama, LM Studio or vLLM on your LAN. You
  use your own key and pay your provider directly. A review uses roughly 5,000
  tokens (about 4,000 in, 1,000 out); a retry doubles that.
- **Automatic reviews (optional).** When on, each game you just played (ended
  within the last 6 hours) is reviewed once its timeline arrives. Older games
  pulled in by a sync are never auto-reviewed.

## Settings

| Setting | What it does |
|---|---|
| Riot API key | Save, replace or remove the key in Windows Credential Manager; reload after changing the environment variable |
| AI provider, key, model, endpoint | Which AI to use. **Load models** lists the models your key can use; **Test connection** sends a one-word prompt |
| Max output tokens | Upper bound per review (default 4000) |
| Automatically review each new game | See above |
| Look-back window | How many of your recent games feed baselines, the dashboard and trends (default 30) |
| Matches per sync / lookup | How many recent games a sync fetches (default 100, up to 1000) |
| Show other modes | Show or hide ARAM/Arena/etc. in the match list |
| Animated background | Splash art of the open match's champion (else your most played), a slow hextech ring and floating particles that react to your mouse. Choose intensity or a fixed champion, or turn it off. It pauses when Windows "reduce motion" is on |

## Your data, keys and privacy

- Everything is stored in `%LOCALAPPDATA%\RiftCoach`: the database (matches,
  timelines, metrics, reviews, settings, activity log, last page you had
  open), the Data Dragon image cache, and logs. Closing and reopening the app,
  or switching between the dev build and an installed build, keeps it all.
  The window also reopens at its last size and position.
- **Keys** (Riot and AI) are stored in Windows Credential Manager under
  `RiftCoach:*` and are never written to the database, settings, logs or files.
  The app never displays a saved key.
- Matches are stored by match ID and Riot ID rather than PUUID, because
  PUUIDs are encrypted per API key. Switching keys keeps your history.

## Riot's rules and what the app won't do

Riot doesn't allow third-party tools to "draw conclusions for you during
gameplay", reveal hidden information, or automate input. So Rift Coach:

- runs the AI only **after** a game (and, from Phase 5, from the loading
  screen as a static game plan), never in reaction to a live game;
- won't track enemy cooldowns or timers, give live alerts, read game memory,
  or estimate MMR;
- will show only your own factual numbers during a game (Phase 4).

## Troubleshooting

| Problem | Fix |
|---|---|
| "No Riot key" or "Riot key rejected" | Save a valid key in Settings, or update `RIOT_API_KEY` and click **Reload** |
| A sync seems slow | That's Riot's rate limit (see the meters on the dashboard); it keeps going in the background |
| "Not found (yet)" retries | A just-finished game takes a few minutes to appear at Riot; it retries automatically |
| AI error about the key or model | Settings → AI coach: save the key, **Load models**, pick one, **Test connection** |
| Page at localhost:1420 in a browser says to use the app | That's the dev server; use the Rift Coach window |

## For developers

**Requirements:** Windows 10/11 with WebView2, [Rust](https://rustup.rs)
(stable, MSVC) with Visual Studio C++ Build Tools, Node.js 20+.

```powershell
npm install
npm run tauri dev                    # the app, with hot reload for the UI
cargo test --workspace               # offline: mocks + recorded, scrubbed fixtures
cargo clippy --workspace --all-targets -- -D warnings
npm run lint                         # eslint + TypeScript
cargo run -p rc-cli -- analyze       # print the newest game's metrics and percentiles
```

Architecture: `docs/architecture.md`. Conventions and commands: `CLAUDE.md`.
AI prompts are versioned files in `prompts/`. Test fixtures in
`tests/fixtures/riot` are real responses with every other player's identity
replaced.

## Roadmap

1. ✅ Foundation: Riot client and rate limiter, SQLite, job queue, Data Dragon cache.
2. ✅ Metrics and baselines, match detail with charts and death map, AI post-game review, settings, player lookup.
3. Signal detection across games (recurring patterns, win-correlation), focus goals, session reviews.
4. Live companion (your own numbers only), game-aware scheduling, auto review at game end.
5. Higher-rank benchmarks, weekly reports, loading-screen game plan.
6. MCP server for AI agents, installer, first-run wizard, auto-update.

# Rift Coach

A local League of Legends coach for Windows. It downloads your games from Riot,
computes detailed metrics for every one of them, compares you with the players
in your own games and with your own history, finds the habits that repeat
across your games and which of your numbers actually go with winning, tracks
focus goals game by game, and (with your own AI key) writes grounded reviews
of single games, play sessions and whole weeks. It also collects games of
higher-ranked players in the background, so you can see where your numbers
would rank among them. While you play it stays out of the
way: it shows only your own numbers on a second screen, pauses its background
work, and analyzes the game as soon as Riot publishes it. Everything stays on
your PC.

> This guide is also the **Guide** page inside the app. It's one file
> (`README.md`), so the app and GitHub always show the same text.

**Status:** Phase 5 of 6 (higher-rank benchmarks, weekly reports, loading-screen
brief, rank). See [Roadmap](#roadmap).

## Contents

- [Getting started](#getting-started)
- [Pages](#pages)
- [How the numbers work](#how-the-numbers-work)
- [Patterns and what wins your games](#patterns-and-what-wins-your-games)
- [Focus goals](#focus-goals)
- [Higher-rank benchmarks](#higher-rank-benchmarks)
- [Weekly reports](#weekly-reports)
- [Live games](#live-games)
- [The loading-screen brief](#the-loading-screen-brief)
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
window in Settings): win rate, KDA, your top 3 recurring problems ("Fix these
first"), your focus goals with progress, key metric averages with your last 5
games alongside, a trend chart for any metric with a 5-game rolling average,
your champions, recent games, sync progress, Riot rate-limit meters, and the
activity log. The log is saved, so it's still there after a restart.

**Dashboard rank.** Your Solo and Flex rank (League-V4) shows under your name,
refreshed after each sync and every 6 hours.

**Live.** On the loading screen: the pre-game brief (see
[The loading-screen brief](#the-loading-screen-brief)). While a game runs: the
game clock, your KDA, CS and gold, CS pacing
against your target, gold toward the next item of your planned build, and your
focus goals' numbers this game (see [Live games](#live-games)). **Pop out
window** opens a small window for a second monitor. Below that: your last
game's post-game status (with a button to open it once it's analyzed), the
**Planned build** editor, and the app's measured footprint during each game.

**Matches.** Every stored game, newest first, filterable by champion, role and
result. ARAM, Arena and other modes are shown dimmed (you can hide them) and
never count toward metrics. ✦ marks games with an AI review. Click a row to
open it.

**Match detail.**
- *AI coach*: the review for this game (see [The AI coach](#the-ai-coach)).
  **Track as goal** turns its next-game focus into a focus goal.
- *Patterns in this game*: which of the pattern rules this game tripped
  (e.g. 2+ deaths before 14:00), with the moments involved, and whether it's a
  **habit** (it recurs in your recent games) or a one-off.
- *Metrics*: every metric for this game, grouped (laning, macro, fighting,
  economy, vision, items), each with three comparisons (see below).
- *Timeline*: gold, XP and CS minute by minute, as a difference against your
  lane opponent or both lines together, plus your team's gold lead. Red lines
  mark your deaths.
- *Map*: Summoner's Rift with your deaths, kills, assists, other kills,
  objectives and towers. Drag the time slider or press **Play** to watch the
  game unfold. Click a marker for details. Your deaths are listed with
  context: how many enemies were involved, laning phase or not, map area,
  whether the enemy took an objective right after, gold versus your opponent,
  and unspent gold.
- *Scoreboard*: all ten players with items, spells, damage and vision. Click
  another player's Riot ID to look them up.

**Patterns.** Recurring weaknesses and strengths across your last *N* games
(pick 20–200 and a role), a heat map of where you die, and "What wins your
games". See [Patterns and what wins your games](#patterns-and-what-wins-your-games).

**Sessions.** Your games grouped into play sessions (games less than the
session gap apart, 60 minutes by default). For each session: results in order,
each game's pattern tags, the session's averages against your usual numbers,
which patterns showed up more than usual, and an AI session review.

**Weekly.** Your weeks (Monday to Sunday): results, champions played, the
week's averages against the four weeks before, patterns, an 8-week trend of key
numbers, the week against higher ranks, and an AI weekly report. See
[Weekly reports](#weekly-reports).

**Benchmarks.** Collection progress, and your typical numbers in a role (or
on one champion) against the benchmark tier, worst first. See
[Higher-rank benchmarks](#higher-rank-benchmarks).

**Goals.** Create, follow, finish or drop focus goals. See [Focus goals](#focus-goals).

**Players.** Track any number of Riot IDs (smurfs, friends, opponents). Switch
the player you're viewing from the top bar. "This is me" marks your own account.
Removing a player deletes their reviews but keeps the stored matches.

**AI reports.** The full history of every report (games, sessions, weeks and
game plans), including regenerated ones, with token usage.

**Settings.** See [Settings](#settings).

## How the numbers work

Metrics are computed by deterministic code from Riot's match and timeline
data for **all ten players** in every stored game, so they can be recomputed at
any time and are covered by unit tests.

| Group | Metrics |
|---|---|
| Laning | CS, gold and XP at 10 and 15 minutes; CS/gold/XP difference vs your lane opponent at 10 and 15; deaths before 14:00; solo kills and deaths; first back time and gold spent |
| Macro | Kill participation; objective participation (dragons, grubs, heralds, barons, towers); time dead; unspent gold at death; shutdown gold given up; deaths while 500+ gold ahead of your lane opponent; deaths followed within 60 s by an enemy dragon/grubs/Herald/Baron/Atakhan; deaths on the enemy side of the map; biggest team gold lead |
| Fighting | KDA, kills, deaths, assists; damage per minute; damage share; gold share; damage share ÷ gold share; damage taken share |
| Economy | CS per minute; gold per minute |
| Vision | Vision score per minute; wards placed and cleared per minute; control wards |
| Items | Time of first completed (non-boots) item |

**Comparisons.** Each metric shows "better than X%" against three groups:

- **Players in your role in your games.** Everyone else who played that role
  in your stored games. These are mostly your lane opponents, so they're
  roughly your skill bracket. It's not a global ladder.
- **Your games.** Your previous games in that role, within the look-back window.
- **Higher ranks.** Players of the benchmark tier (Diamond by default) in that
  role on the same patch, or on your champion when at least 15 were collected.
  It needs 30 players in the role first; until then it says "not enough
  data". See [Higher-rank benchmarks](#higher-rank-benchmarks).

For metrics where lower is better (deaths, time dead), "better" means lower.
Lane differences (CS/gold/XP diff) are compared only with your own games,
because your opponents' differences are the mirror image of yours. Metrics
marked ≈ are approximate, since Riot samples positions and gold about once per
minute. Ward positions aren't in Riot's data, so the map can't show wards.

**Map areas.** Deaths are labelled with a rough area (your/enemy base, top,
mid or bot lane, top/bot river, your/enemy jungle) from straight-line bands
over the map, so treat them as approximate.

## Patterns and what wins your games

Patterns are found by fixed, unit-tested rules, never by the AI. Each one
says how many of your games it affected, your win rate when it happens versus
when it doesn't, and links to examples (game and time).

- **Rule patterns.** A per-game test that becomes a pattern when it recurs:
  2+ deaths before 14:00 (35%+ of games), 2+ solo deaths (30%+), dying while
  500+ gold ahead in lane (30%+), a death followed by an enemy objective within
  60 s (35%+), dying with 1,500+ unspent gold (30%+), giving up 700+ shutdown
  gold (25%+), losing after your team led by 3,000+ gold (30%+ of those games),
  10+ CS behind at 10:00 (45%+; not for supports). Strengths: 10+ CS ahead at
  10:00 (50%+), no deaths before 14:00 (60%+). A pattern needs at least 5
  games with the data and 3 affected games.
- **Below / ahead of most players in your role.** For each metric, your
  "better than X%" against players in your role in your games, per game. If
  your typical (median) game is at or below 30%, it's a weakness; at or above
  70%, a strength. Needs 8+ games in that role.
- **Where you die.** If an area of the map holds 20%+ of your deaths and at
  least 10 points more than for players in your role, it's flagged. The heat
  map shows every death in the window; by default red-side games are mirrored
  so your base is always bottom-left.
- **Sessions.** If you win 15+ points less often in games queued right after a
  loss (same session), or from the 4th game of a session on, it's flagged.

Patterns are ranked by how often they happen, how many games they cover, and
how much worse your win rate is when they happen. **Make this a goal** turns a
pattern into a focus goal with a sensible target.

**What wins your games.** For every metric, how it differs between your wins
and losses: the median in each, your win rate above versus below your own
median, and how likely the difference is to be chance (a Mann-Whitney test).
It's correlation, not cause: numbers measured late in a game mostly reflect
who was already winning, so the **Early game** table (measured by about 15:00)
is the one to act on.

## Focus goals

A goal is "a metric, at most / at least a target, on average over the next *N*
games", optionally in one role (e.g. *Deaths before 14:00 at most 1 over 10
games*). It counts your Summoner's Rift games played after you create it.

- Each game shows as a green (target met) or red square; click one to open it.
- **Average so far** is compared with **Before the goal**, your average over
  your look-back window when you started it, so you can see if you're
  improving.
- When all *N* games are played, the goal completes as **Achieved** (the
  average met the target) or **Missed**. You can also finish, drop, reopen or
  delete a goal. Up to 3 can be active at once.
- AI reviews (game and session) see your active goals and comment on them.

## Higher-rank benchmarks

"Better than most players in your games" only tells you where you stand in
your own bracket. Benchmarks show what the next level looks like.

- **What's collected.** Recent solo queue games of players in the benchmark
  tier (Emerald, Diamond or Master+; Settings → Benchmarks). Rift Coach reads
  the ladder (League-V4), lists those players' games from the last two weeks,
  downloads each match and timeline, and runs the same metrics on all ten
  players. Benchmarks are kept per patch (the newest two patches are kept).
- **Your champions first.** It checks players' top champion masteries and
  samples those who main your focus champions first (by default your most
  played champions with 5+ games in your last 50, LeBlanc for you). Every game
  also adds samples for every role in it.
- **Targets.** 300 players in each of your main roles and 50 on each focus
  champion, per patch. When they're met, collection stops until the next patch
  or the weekly ladder refresh.
- **Polite.** One step every 5 seconds (1–2 Riot calls each), which is under
  half your key's limit. It never runs during a game, while your own games are
  downloading, or with a key problem. It picks up where it left off after a
  restart.
- **Private.** Other players' names are removed from benchmark games before
  they're stored. Benchmark games never count as "your games" or "players in
  your games".
- **Disk.** About 60 KB per game, up to 2,000 games by default (about 120 MB).
- **Where it shows.** A third comparison on every match, the Benchmarks page,
  the Weekly page, the live view's CS benchmark (the median for your champion
  or role), the loading-screen brief's targets, and the AI facts.

The comparison is with players of that tier in that role, sampled from games
of ladder players in it. Their lobbies are mostly the same tier but not
exclusively, so treat it as "players around that rank", not an exact ladder.

## Weekly reports

The Weekly page groups your games by week (Monday to Sunday, local time). For
each week: results and champions, the week's averages against your four weeks
before, which patterns showed up and how often they usually do, an 8-week
trend for CS at 10:00, gold diff at 15:00, deaths before 14:00, solo deaths,
kill participation and vision, and your week against the benchmark tier.

**Write the weekly report** asks the AI for a trend report: what improved or
slipped, recurring habits, the biggest early-game gaps to higher ranks, and
one focus for next week (which you can track as a goal). It's checked like any
other report. With **Automatically write a weekly report** on, it's written
after each week with 3+ Summoner's Rift games.

## Live games

Rift Coach checks every few seconds whether a game is running (it asks the
game client's local Live Client Data API at `127.0.0.1:2999`; when nothing
runs it backs off to every 15 seconds). It notices the loading screen,
the game, and the end screen.

**While you play**

- **Background work pauses.** Match fetching, metric calculation and AI
  reviews wait until the game ends (a job already running finishes first).
  The animated background freezes. Only the small live poller runs. Turn this
  off in Settings → Live if you prefer.
- **The live view** (Live page or the pop-out window) shows only your own
  numbers from the game client and targets you set before the game:
  - **CS pacing** from 5:00: your CS against *target × minutes played*. The
    target is, in order: the one in Settings → Live, an active "CS per minute
    at least" goal, or your own average in that role. The benchmark line is
    the median of the benchmark tier on your champion (or in your role), or,
    until enough are collected, the top 30% of players in your role in your
    own games.
  - **Build:** gold toward the next item you don't own yet in your planned
    build, counting the components of it you already hold. With no saved plan
    it uses your most common finished items on that champion.
  - **KDA, CS, gold, level, and focus goals.** Goals on CS/min, CS at 10:00,
    deaths, deaths before 14:00, kills, assists, KDA or vision show this
    game's value; others show after the game.
  - No AI, no advice, no alerts or sounds, and nothing about other players.
    The window never draws over the game.

**When the game ends**

1. Work resumes on the end screen.
2. About a minute later the app asks Riot for your newest games and links the
   one that started when your live game did. Riot usually publishes a game
   1–5 minutes after it ends; until then it keeps checking (for up to about
   1.5 hours).
3. The match and timeline are fetched and analyzed, then an automatic AI
   review runs if that's turned on.
4. A Windows notification says the game is analyzed (or that its AI review is
   ready), and the Live page's **Last game** card opens it.

Only games of your tracked Riot IDs are analyzed. Practice tool games are
recorded but never reach Riot's match history, so they're not looked up.

**Snapshots and footprint.** Your numbers are saved every 15 seconds of game
time, and deleted after a game is analyzed (default after 7 days; Settings →
Live). The app's own CPU and memory are measured every 5 seconds during each
game: its backend plus its windows (WebView2), counting private memory. The
budget is **1% CPU on average** (a share of the whole processor) and **400 MB
memory**, and the Live page shows each game against it. Closing the pop-out
window (or minimizing the main one) saves the most.

**Planned builds.** On the Live page, pick a champion, then add, reorder
or remove items (finished items and boots for Summoner's Rift, up to 8) and
**Save plan**. **Use my usual items instead** removes the saved plan.

## The loading-screen brief

When the loading screen appears, Rift Coach asks Riot's Spectator API which
champions are in your game (one call), and builds a brief on the Live page and
the pop-out window:

- **Your record** on your champion (games, wins, CS at 10, lane diffs, early
  deaths).
- **Likely lane opponent.** Spectator doesn't report roles, so it's the enemy
  champion most often played in your role across stored games. You see your
  past games against it in lane.
- **Your record against each enemy champion.**
- **Watch:** your top two recurring habits, with a tip, and your focus goals.
- **Targets:** benchmark medians for CS at 10, CS per minute, early deaths and
  kill participation on your champion, with your own averages.

**AI game plan (optional).** With **Write an AI game plan at the loading
screen** on (Settings → AI coach), or with the button while you're still
loading, the AI writes 2–4 plan points, up to 3 things to watch out for, and
one focus, from the brief alone. It's written once and never updated during
the game.

The brief uses champions only: never other players' names, ranks or histories.
If the app opens mid-game you still get the brief, but no AI plan.

## The AI coach

- **What it sees.** Only computed facts: for a game, the metrics, the
  comparisons, your deaths with context, champions, result and items, the
  patterns this game showed (and whether they're habits), and your active
  goals; for a session, each game's key numbers and patterns, the session's
  averages against your usual, and your goals; for a week, the week's averages
  against the four before, champions, patterns, the 8-week trend and benchmark
  gaps; for a game plan, the brief. Click **Facts sent** to see exactly what's
  sent. Other players' Riot IDs and raw API data are never sent.
- **What it returns.** A summary, what went well, what to improve, 1–3
  priority fixes with a concrete drill, one focus for your next games (or
  session), how your goals went, and a confidence level. Each point cites the
  facts it's based on. When the focus maps to a metric, **Track as goal**
  creates it.
- **How it's checked.** Every number in the review is compared against the
  facts. If the reply is malformed, or cites facts that don't exist, it's
  rejected and the AI is asked again. If a number still can't be traced, the
  review is saved with an "unverified numbers" warning.
- **Caching and history.** A review is generated once per game (or session),
  prompt version and model. **Regenerate** makes a new one, and every version
  is kept on the AI reports page. A session that gains another game becomes a
  new session to review.
- **Providers.** Anthropic (Claude), OpenAI, Google Gemini, or any
  OpenAI-compatible server, such as Ollama, LM Studio or vLLM on your LAN. You
  use your own key and pay your provider directly. A review uses roughly 5,000
  tokens (about 4,000 in, 1,000 out); a session review about the same; a retry
  doubles that. An optional **token cap** stops all reviews once the last 30
  days used that many tokens.
- **Automatic reviews (optional).** Game reviews: each game you just played
  (ended within the last 6 hours) is reviewed once its timeline arrives.
  Session reviews: once no game has been played for the session gap, the
  session that just ended is reviewed (2+ Summoner's Rift games, ended in the
  last day; checked every 10 minutes). Older games pulled in by a sync are
  never auto-reviewed.

## Settings

| Setting | What it does |
|---|---|
| Riot API key | Save, replace or remove the key in Windows Credential Manager; reload after changing the environment variable |
| AI provider, key, model, endpoint | Which AI to use. **Load models** lists the models your key can use; **Test connection** sends a one-word prompt |
| Max output tokens | Upper bound per review (default 4000) |
| Automatically write a weekly report | After each week with 3+ Summoner's Rift games (default off) |
| Write an AI game plan at the loading screen | One short static plan per game (default off) |
| Collect higher-rank games | Background benchmark collection (default on) |
| Compare with | Benchmark tier: Emerald, Diamond (default) or Master and above |
| Champions to sample first | Up to 3; empty = your most played |
| Pace | Seconds between collection steps (default 5; each step makes 1–2 Riot calls) |
| Keep up to N games | Benchmark games kept on disk (default 2,000, about 120 MB) |
| Detect running games | Watch for a game for the live view and post-game analysis (default on) |
| Pause background work during games | Hold fetching, analysis and AI work while a game runs (default on) |
| Notify me when a finished game is analyzed | Windows notification when your last game (or its AI review) is ready (default on) |
| CS per minute target | Target for live CS pacing. Empty = your CS/min goal, else your own average in the role |
| Keep live snapshots | Days to keep a game's 15-second snapshots after it's analyzed (default 7; 0 = delete right away) |
| Automatically review each new game / each play session | See above |
| Token cap (last 30 days) | Stop generating reviews once this many tokens were used in the last 30 days; shows your usage. Empty = no cap |
| Look-back window | How many of your recent games feed baselines, patterns, the dashboard and trends (default 30) |
| Session gap | Games less than this many minutes apart are one session (default 60) |
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

- runs the AI only **after** a game, or once on the loading screen for a
  static game plan built from champions and your own history, never in
  reaction to a live game;
- uses only champions from Spectator for the brief: no other player's name,
  rank or history (Riot forbids de-anonymizing players, and the app does no
  scouting of individuals);
- won't track enemy cooldowns or timers, give live alerts, read game memory,
  or estimate MMR;
- shows only your own factual numbers during a game, next to targets you set
  beforehand. The live data includes other players' scoreboard entries; the
  app drops them while reading and never stores or shows them;
- never draws an overlay on the game or sends input to it.

## Troubleshooting

| Problem | Fix |
|---|---|
| "No Riot key" or "Riot key rejected" | Save a valid key in Settings, or update `RIOT_API_KEY` and click **Reload** |
| A sync seems slow | That's Riot's rate limit (see the meters on the dashboard); it keeps going in the background |
| "Not found (yet)" retries | A just-finished game takes a few minutes to appear at Riot; it retries automatically |
| AI error about the key or model | Settings → AI coach: save the key, **Load models**, pick one, **Test connection** |
| "The AI token cap is reached" | Raise or clear the token cap in Settings → AI coach |
| No patterns shown | Patterns need 5+ games with timelines (8+ in a role for "below most players"); sync more games or widen the window |
| Page at localhost:1420 in a browser says to use the app | That's the dev server; use the Rift Coach window |
| The Live page doesn't notice my game | Check Settings → Live → Detect running games. The game client must be running a game (not the lobby or champ select) |
| "Waiting for Riot to publish the match" for a long time | Riot occasionally takes longer; it keeps checking for about 1.5 hours. A sync also picks it up |
| My game wasn't analyzed | Only tracked Riot IDs are analyzed; add the account on the Players page |
| The higher-ranks column says "not enough data" | Benchmarks are still being collected (Benchmarks page shows progress); it needs 30 players in your role on the patch |
| Benchmarks say "Waiting" | They pause during games, while your own games download, and when the Riot key has a problem |
| No brief on the loading screen | The brief needs Riot's Spectator API to see your game, which can lag a few seconds; it shows "None of your tracked Riot IDs is in a game" when it can't |
| CS in the live view jumps in steps | That's how the game client reports it; the number catches up |

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
cargo run -p rc-cli -- patterns --games 100 --sessions   # patterns, win links, sessions
cargo run -p rc-cli -- mock-live --speed 10              # a scripted game for the live view
cargo run -p rc-cli -- benchmarks [--collect 5]          # benchmark progress and gaps (collect for 5 min first)
```

To try the live view without League, run `mock-live` in one terminal and the
app pointed at it in another:
`$env:RIFT_COACH_LIVE_URL = "http://127.0.0.1:2998"; npm run tauri dev`.
The mock defaults to a practice tool game (no Match-V5 lookup afterwards);
`--mode CLASSIC` exercises the post-game lookup too.

Architecture: `docs/architecture.md`. Conventions and commands: `CLAUDE.md`.
AI prompts are versioned files in `prompts/`. Test fixtures in
`tests/fixtures/riot` are real responses with every other player's identity
replaced.

## Roadmap

1. ✅ Foundation: Riot client and rate limiter, SQLite, job queue, Data Dragon cache.
2. ✅ Metrics and baselines, match detail with charts and death map, AI post-game review, settings, player lookup.
3. ✅ Patterns across games, win correlation, death heat map, focus goals, session reviews, AI token cap.
4. ✅ Live companion (your own numbers only), game-aware scheduling, analysis and review at game end, footprint measurement.
5. ✅ Higher-rank benchmarks by patch, benchmark percentiles, weekly reports, loading-screen brief and AI game plan, rank.
6. MCP server for AI agents, installer, first-run wizard, auto-update.

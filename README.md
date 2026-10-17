# Rift Coach

A local League of Legends performance coach for Windows. It ingests your
matches and timelines from the Riot API, stores everything on your PC, and
(in later phases) explains why you win or lose with deterministic metrics and
AI reviews grounded in that data.

**Status:** Phase 1 (foundation) of 6. See `SPEC.md` for the full plan.

## Requirements

- Windows 10/11 with WebView2 (preinstalled on Windows 11)
- [Rust](https://rustup.rs) (stable, MSVC) and Visual Studio C++ Build Tools
- Node.js 20+
- A Riot API key in the `RIOT_API_KEY` user environment variable

## Run

```powershell
npm install
npm run tauri dev
```

Enter your Riot ID (e.g. `TapRooted#BTC`) and click **Ingest**. With a Personal
key, 100 matches plus timelines take about 4–5 minutes, because Riot limits
Personal keys to 100 requests per 2 minutes. Closing the app is safe: work
resumes on the next start.

Headless alternative: `cargo run -p rc-cli -- ingest "TapRooted#BTC"`.

## Develop

See `CLAUDE.md` for commands and conventions, and `docs/architecture.md` for design.

```powershell
cargo test --workspace          # offline; uses mocks and recorded fixtures
cargo clippy --workspace --all-targets -- -D warnings
npm run lint
```

## Data and privacy

Everything is stored under `%LOCALAPPDATA%\RiftCoach`. Test fixtures in
`tests/fixtures/riot` are real responses with every other player's Riot ID and
all PUUIDs replaced by fakes.

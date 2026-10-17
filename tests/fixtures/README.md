# Test fixtures

`riot/` contains real Riot API responses for TapRooted#BTC (recorded
2026-10-17, patch 16.20), scrubbed for committing:

- every PUUID → `FIXTURE-PUUID-SELF` / `FIXTURE-PUUID-NNN`
- every summoner ID → `FIXTURE-SUMMONER-N`
- other players' Riot IDs → `PlayerNNN#FIX` (yours is kept)

The scrubber refuses to write a file if any original identifier survives.

To refresh (raw captures go to gitignored `raw-captures/`):

```powershell
cargo run -p rc-cli -- record-fixtures "TapRooted#BTC" --matches 3
cargo run -p rc-cli -- scrub-fixtures --keep "TapRooted#BTC"
```

Tests that read these: `crates/riot/tests/fixtures.rs` and `crates/jobs/tests/ingest.rs`.

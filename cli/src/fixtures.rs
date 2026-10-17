//! Fixture recording and scrubbing.
//!
//! `record` saves verbatim responses to a gitignored folder. `scrub_dir`
//! rewrites them for committing: every PUUID and summoner ID becomes a stable
//! fake, and every other player's Riot ID becomes `PlayerNN#FIX`. The kept
//! Riot ID (yours) stays readable, but its PUUID is still replaced, because
//! PUUIDs are tied to the API key.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{Context, bail};
use rc_riot::client::MatchIdQuery;
use rc_riot::{Platform, RiotClient, RiotId};
use serde_json::Value;

pub async fn record(riot: &RiotClient, riot_id: &RiotId, platform: Platform, n: u32, out: &Path) -> anyhow::Result<()> {
    let region = platform.match_region();
    std::fs::create_dir_all(out.join("matches"))?;
    std::fs::create_dir_all(out.join("timelines"))?;
    let save = |name: &str, body: &[u8]| -> anyhow::Result<()> {
        // Re-indent so fixtures diff nicely.
        let v: Value = serde_json::from_slice(body)?;
        std::fs::write(out.join(name), serde_json::to_vec_pretty(&v)?)?;
        Ok(())
    };

    let account = riot.account_by_riot_id(platform.account_region(), riot_id).await?;
    save("account.json", &serde_json::to_vec(&account)?)?;
    let ids = riot.match_ids(region, &account.puuid, &MatchIdQuery { count: n, ..Default::default() }).await?;
    save("match-ids.json", &serde_json::to_vec(&ids)?)?;
    for id in &ids {
        save(&format!("matches/{id}.json"), &riot.match_raw(region, id).await?)?;
        save(&format!("timelines/{id}.json"), &riot.timeline_raw(region, id).await?)?;
        println!("  recorded {id}");
    }
    save("league-entries.json", &serde_json::to_vec(&riot.league_entries(platform, &account.puuid).await?)?)?;
    save("mastery-top.json", &serde_json::to_vec(&riot.mastery_top(platform, &account.puuid, 5).await?)?)?;
    println!("Raw responses in {} (gitignored). Next: rc-cli scrub-fixtures --keep \"{riot_id}\"", out.display());
    Ok(())
}

#[derive(Debug, Default)]
pub struct ScrubReport {
    pub files: usize,
    pub players_replaced: usize,
}

/// Identity replacements, built from every participant seen across files.
#[derive(Debug, Default)]
struct Identities {
    /// Exact string → replacement (PUUIDs, summoner IDs).
    ids: HashMap<String, String>,
    /// PUUID → replacement (game name, tag line), for other players.
    names: HashMap<String, (String, String)>,
    self_puuids: Vec<String>,
    next: usize,
}

impl Identities {
    fn see_participant(&mut self, p: &serde_json::Map<String, Value>, keep: &RiotId) {
        let Some(puuid) = p.get("puuid").and_then(Value::as_str) else { return };
        if self.ids.contains_key(puuid) {
            return;
        }
        let name = p.get("riotIdGameName").and_then(Value::as_str).unwrap_or("");
        let tag = p.get("riotIdTagline").and_then(Value::as_str).unwrap_or("");
        if keep.matches(name, tag) {
            self.ids.insert(puuid.to_owned(), "FIXTURE-PUUID-SELF".to_owned());
            self.self_puuids.push(puuid.to_owned());
        } else {
            self.next += 1;
            let n = self.next;
            self.ids.insert(puuid.to_owned(), format!("FIXTURE-PUUID-{n:03}"));
            self.names.insert(puuid.to_owned(), (format!("Player{n:03}"), "FIX".to_owned()));
        }
        if let Some(sid) = p.get("summonerId").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            let fake = format!("FIXTURE-SUMMONER-{}", self.ids.len());
            self.ids.entry(sid.to_owned()).or_insert(fake);
        }
    }
}

fn collect(v: &Value, keep: &RiotId, ids: &mut Identities) {
    match v {
        Value::Object(map) => {
            if map.contains_key("puuid") && (map.contains_key("riotIdGameName") || map.contains_key("participantId")) {
                ids.see_participant(map, keep);
            }
            map.values().for_each(|c| collect(c, keep, ids));
        }
        Value::Array(items) => items.iter().for_each(|c| collect(c, keep, ids)),
        _ => {}
    }
}

const NAME_FIELDS: [&str; 3] = ["riotIdGameName", "riotIdName", "summonerName"];

fn rewrite(v: &mut Value, ids: &Identities) {
    match v {
        Value::Object(map) => {
            let puuid = map.get("puuid").and_then(Value::as_str).map(str::to_owned);
            if let Some((name, tag)) = puuid.as_deref().and_then(|p| ids.names.get(p)) {
                for f in NAME_FIELDS {
                    if map.contains_key(f) {
                        map.insert(f.to_owned(), Value::String(name.clone()));
                    }
                }
                if map.contains_key("riotIdTagline") {
                    map.insert("riotIdTagline".to_owned(), Value::String(tag.clone()));
                }
            }
            map.values_mut().for_each(|c| rewrite(c, ids));
        }
        Value::Array(items) => items.iter_mut().for_each(|c| rewrite(c, ids)),
        Value::String(s) => {
            if let Some(fake) = ids.ids.get(s.as_str()) {
                *s = fake.clone();
            }
        }
        _ => {}
    }
}

fn json_files(dir: &Path, out: &mut BTreeMap<std::path::PathBuf, Value>, root: &Path) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            json_files(&path, out, root)?;
        } else if path.extension().is_some_and(|e| e == "json") {
            let v: Value = serde_json::from_slice(&std::fs::read(&path)?)
                .with_context(|| format!("parsing {}", path.display()))?;
            out.insert(path.strip_prefix(root)?.to_path_buf(), v);
        }
    }
    Ok(())
}

pub fn scrub_dir(input: &Path, output: &Path, keep: &RiotId) -> anyhow::Result<ScrubReport> {
    let mut files = BTreeMap::new();
    json_files(input, &mut files, input)?;
    if files.is_empty() {
        bail!("no .json files in {}", input.display());
    }
    let mut ids = Identities::default();
    for v in files.values() {
        collect(v, keep, &mut ids);
    }
    // The account/league/mastery files carry your PUUID without a Riot ID
    // next to it; account.json tells us which one it is.
    if let Some(p) = files.get(Path::new("account.json")).and_then(|a| a.get("puuid")).and_then(Value::as_str) {
        ids.ids.entry(p.to_owned()).or_insert_with(|| "FIXTURE-PUUID-SELF".to_owned());
        ids.self_puuids.push(p.to_owned());
    }
    if ids.self_puuids.is_empty() {
        bail!("never saw {keep} in the input; refusing to guess which player to keep");
    }

    let mut report = ScrubReport { files: 0, players_replaced: ids.names.len() };
    for (rel, mut v) in files {
        rewrite(&mut v, &ids);
        let dest = output.join(&rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(&v)?;
        // Belt and braces: no original identifier may survive.
        if let Some(leak) = ids.ids.keys().find(|orig| text.contains(orig.as_str())) {
            bail!(
                "scrub failed: an original identifier survived in {} ({}...)",
                rel.display(),
                &leak[..8.min(leak.len())]
            );
        }
        std::fs::write(&dest, text)?;
        report.files += 1;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scrubs_others_and_keeps_self_name() {
        let input = tempfile::tempdir().expect("tmp");
        let output = tempfile::tempdir().expect("tmp");
        let m = json!({
            "metadata": {"matchId": "NA1_1", "participants": ["real-me", "real-other"]},
            "info": {"participants": [
                {"participantId": 1, "puuid": "real-me", "riotIdGameName": "TapRooted", "riotIdTagline": "BTC",
                 "summonerId": "sid-me", "summonerName": ""},
                {"participantId": 2, "puuid": "real-other", "riotIdGameName": "Stranger", "riotIdTagline": "NA1",
                 "summonerId": "sid-other", "summonerName": "Stranger"}
            ]}
        });
        let tl = json!({"metadata": {"participants": ["real-me", "real-other"]},
                        "info": {"participants": [{"participantId": 2, "puuid": "real-other"}]}});
        std::fs::create_dir_all(input.path().join("matches")).expect("dir");
        std::fs::write(input.path().join("matches/NA1_1.json"), m.to_string()).expect("w");
        std::fs::write(input.path().join("tl.json"), tl.to_string()).expect("w");
        std::fs::write(input.path().join("account.json"), json!({"puuid": "real-me"}).to_string()).expect("w");

        let keep: RiotId = "taprooted#btc".parse().expect("id");
        let report = scrub_dir(input.path(), output.path(), &keep).expect("scrub");
        assert_eq!((report.files, report.players_replaced), (3, 1));

        let out: Value =
            serde_json::from_slice(&std::fs::read(output.path().join("matches/NA1_1.json")).expect("r")).expect("j");
        let ps = &out["info"]["participants"];
        assert_eq!(ps[0]["riotIdGameName"], "TapRooted");
        assert_eq!(ps[0]["puuid"], "FIXTURE-PUUID-SELF");
        assert_eq!(ps[1]["riotIdGameName"], "Player001");
        assert_eq!(ps[1]["riotIdTagline"], "FIX");
        assert_eq!(ps[1]["summonerName"], "Player001");
        assert_eq!(out["metadata"]["participants"][1], "FIXTURE-PUUID-001");

        let all = [
            std::fs::read_to_string(output.path().join("matches/NA1_1.json")).expect("r"),
            std::fs::read_to_string(output.path().join("tl.json")).expect("r"),
            std::fs::read_to_string(output.path().join("account.json")).expect("r"),
        ]
        .concat();
        for secret in ["real-me", "real-other", "Stranger", "sid-other", "sid-me"] {
            assert!(!all.contains(secret), "{secret} leaked");
        }
    }

    #[test]
    fn refuses_when_kept_player_absent() {
        let input = tempfile::tempdir().expect("tmp");
        let output = tempfile::tempdir().expect("tmp");
        std::fs::write(
            input.path().join("m.json"),
            json!({"info": {"participants": [{"participantId": 1, "puuid": "x", "riotIdGameName": "A", "riotIdTagline": "B"}]}})
                .to_string(),
        )
        .expect("w");
        let keep: RiotId = "Me#NA1".parse().expect("id");
        assert!(scrub_dir(input.path(), output.path(), &keep).is_err());
    }
}

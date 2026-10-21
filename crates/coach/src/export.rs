//! Export your games as CSV (one row per Summoner's Rift game, one column
//! per metric) for spreadsheets.

use chrono::{Local, TimeZone};
use rc_metrics::defs::METRICS;
use rc_store::metrics::{GameFilter, game_metrics};
use rc_store::rusqlite::Connection;

fn field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_owned() }
}

pub fn games_csv(conn: &Connection, account_id: i64) -> rc_store::Result<String> {
    let games = game_metrics(conn, account_id, &GameFilter { limit: u32::MAX, ..Default::default() })?;
    let mut header: Vec<String> =
        ["match_id", "date", "patch", "champion", "role", "win", "duration_min"].map(str::to_owned).to_vec();
    header.extend(METRICS.iter().map(|d| d.key.to_owned()));
    let mut out = header.join(",");
    out.push('\n');
    for g in games {
        let date = Local
            .timestamp_millis_opt(g.game_start_ms)
            .single()
            .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default();
        let patch = rc_store::matches::patch_of_match(conn, &g.match_id)?.unwrap_or_default();
        let mut row = vec![
            field(&g.match_id),
            date,
            field(&patch),
            field(&g.champion_name),
            field(g.team_position.as_deref().unwrap_or("")),
            (if g.win { "1" } else { "0" }).to_owned(),
            format!("{:.1}", g.duration_s as f64 / 60.0),
        ];
        row.extend(METRICS.iter().map(|d| g.metrics.get(d.key).map(|v| format!("{v:.3}")).unwrap_or_default()));
        out.push_str(&row.join(","));
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn quotes_fields_that_need_it() {
        assert_eq!(super::field("Leblanc"), "Leblanc");
        assert_eq!(super::field("a,b"), "\"a,b\"");
        assert_eq!(super::field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }
}

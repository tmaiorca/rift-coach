import { useEffect, useState } from "react";
import { api } from "../api";
import type { MatchRow } from "../types";

const ROLE_LABEL: Record<string, string> = {
  TOP: "Top",
  JUNGLE: "Jungle",
  MIDDLE: "Mid",
  BOTTOM: "ADC",
  UTILITY: "Support",
};

export function MatchTable({ matches }: { matches: MatchRow[] }) {
  return (
    <section className="panel matches">
      <h2>Matches {matches.length > 0 && <span className="muted">({matches.length})</span>}</h2>
      {matches.length === 0 ? (
        <p className="muted">No matches yet. Enter your Riot ID and click Ingest.</p>
      ) : (
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th>Champion</th>
                <th>Result</th>
                <th>Queue</th>
                <th>Role</th>
                <th className="num">K / D / A</th>
                <th className="num">CS</th>
                <th className="num">Vision</th>
                <th className="num">Length</th>
                <th>Patch</th>
                <th>Played</th>
                <th title="Timeline fetched">TL</th>
              </tr>
            </thead>
            <tbody>
              {matches.map((m) => (
                <Row key={m.match_id} m={m} />
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

function Row({ m }: { m: MatchRow }) {
  const minutes = m.duration_s / 60;
  const result = m.is_remake ? "Remake" : m.win ? "Win" : "Loss";
  return (
    <tr className={m.is_standard_rift ? "" : "dim"} title={m.match_id}>
      <td>
        <span className="champ">
          <ChampionIcon championId={m.champion_id} patch={m.patch} />
          {m.champion_name}
        </span>
      </td>
      <td>
        <span className={`result ${result.toLowerCase()}`}>{result}</span>
      </td>
      <td>{m.queue_name}</td>
      <td>{m.team_position ? (ROLE_LABEL[m.team_position] ?? m.team_position) : "-"}</td>
      <td className="num">
        {m.kills} / <span className="deaths">{m.deaths}</span> / {m.assists}
      </td>
      <td className="num">
        {m.cs} <span className="muted">({minutes > 0 ? (m.cs / minutes).toFixed(1) : "0"})</span>
      </td>
      <td className="num">{m.vision_score}</td>
      <td className="num">
        {Math.floor(m.duration_s / 60)}:{String(m.duration_s % 60).padStart(2, "0")}
      </td>
      <td>{m.patch}</td>
      <td>
        {new Date(m.game_start_ms).toLocaleString(undefined, { dateStyle: "short", timeStyle: "short" })}
      </td>
      <td>{m.has_timeline ? "✓" : <span className="muted">…</span>}</td>
    </tr>
  );
}

const iconCache = new Map<string, Promise<string>>();

function ChampionIcon({ championId, patch }: { championId: number; patch: string }) {
  const key = `${patch}:${championId}`;
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    let p = iconCache.get(key);
    if (!p) {
      p = api.championIcon(championId, patch);
      iconCache.set(key, p);
      // Allow a retry later if this fails (e.g. offline).
      p.catch(() => iconCache.delete(key));
    }
    p.then((s) => alive && setSrc(s)).catch(() => {});
    return () => {
      alive = false;
    };
  }, [key, championId, patch]);
  return src ? <img className="champ-icon" src={src} alt="" /> : <span className="champ-icon placeholder" />;
}

import { useMemo, useState } from "react";
import { ChampionIcon, useChampionName } from "../lib/assets";
import { clock, dateTime, roleLabel } from "../lib/format";
import type { MatchRow } from "../types";

interface Props {
  matches: MatchRow[];
  onOpen: (matchId: string) => void;
}

export function Matches({ matches, onOpen }: Props) {
  const name = useChampionName();
  const [champ, setChamp] = useState("");
  const [role, setRole] = useState("");
  const [result, setResult] = useState("");

  const champs = useMemo(
    () =>
      [...new Map(matches.map((m) => [m.champion_id, name(m.champion_id, m.champion_name)])).entries()].sort(
        (a, b) => a[1].localeCompare(b[1]),
      ),
    [matches, name],
  );
  const rows = matches.filter(
    (m) =>
      (!champ || String(m.champion_id) === champ) &&
      (!role || m.team_position === role) &&
      (!result || (result === "win" ? m.win && !m.is_remake : !m.win && !m.is_remake)),
  );
  const rift = rows.filter((m) => m.is_standard_rift && !m.is_remake);
  const wins = rift.filter((m) => m.win).length;

  return (
    <div className="page">
      <section className="panel">
        <div className="panel-head">
          <h2>
            Matches <span className="muted">({rows.length})</span>
          </h2>
          <div className="spacer" />
          <select value={champ} onChange={(e) => setChamp(e.target.value)} aria-label="Champion">
            <option value="">All champions</option>
            {champs.map(([id, n]) => (
              <option key={id} value={id}>
                {n}
              </option>
            ))}
          </select>
          <select value={role} onChange={(e) => setRole(e.target.value)} aria-label="Role">
            <option value="">All roles</option>
            {["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "UTILITY"].map((r) => (
              <option key={r} value={r}>
                {roleLabel(r)}
              </option>
            ))}
          </select>
          <select value={result} onChange={(e) => setResult(e.target.value)} aria-label="Result">
            <option value="">Wins and losses</option>
            <option value="win">Wins</option>
            <option value="loss">Losses</option>
          </select>
        </div>
        {rift.length > 0 && (
          <p className="muted">
            Summoner's Rift: {wins}W {rift.length - wins}L ({Math.round((wins / rift.length) * 100)}%)
          </p>
        )}
        {matches.length === 0 ? (
          <p className="muted">No matches yet. Add your Riot ID on the Players page and sync.</p>
        ) : (
          <div className="table-wrap">
            <table className="clickable">
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
                  <th title="AI review">AI</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((m) => (
                  <Row key={m.match_id} m={m} name={name(m.champion_id, m.champion_name)} onOpen={onOpen} />
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </div>
  );
}

function Row({ m, name, onOpen }: { m: MatchRow; name: string; onOpen: (id: string) => void }) {
  const minutes = m.duration_s / 60;
  const result = m.is_remake ? "Remake" : m.win ? "Win" : "Loss";
  return (
    <tr
      className={m.is_standard_rift ? "" : "dim"}
      title={`${m.match_id}${m.has_timeline ? "" : " (timeline pending)"}`}
      onClick={() => onOpen(m.match_id)}
      tabIndex={0}
      onKeyDown={(e) => e.key === "Enter" && onOpen(m.match_id)}
    >
      <td>
        <span className="champ">
          <ChampionIcon id={m.champion_id} patch={m.patch} />
          {name}
        </span>
      </td>
      <td>
        <span className={`result ${result.toLowerCase()}`}>{result}</span>
      </td>
      <td>{m.queue_name}</td>
      <td>{roleLabel(m.team_position)}</td>
      <td className="num">
        {m.kills} / <span className="deaths">{m.deaths}</span> / {m.assists}
      </td>
      <td className="num">
        {m.cs} <span className="muted">({minutes > 0 ? (m.cs / minutes).toFixed(1) : "0"})</span>
      </td>
      <td className="num">{m.vision_score}</td>
      <td className="num">{clock(m.duration_s)}</td>
      <td>{m.patch}</td>
      <td>{dateTime(m.game_start_ms)}</td>
      <td>
        {m.has_review ? (
          <span className="spark" title="Reviewed">
            ✦
          </span>
        ) : (
          ""
        )}
      </td>
    </tr>
  );
}

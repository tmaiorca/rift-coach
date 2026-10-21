import { useEffect, useState } from "react";
import { api } from "../api";
import { GapsTable } from "../components/Gaps";
import { SignalCard } from "../components/SignalCard";
import { ChampionIcon, useChampionName } from "../lib/assets";
import { ago, dateTime, formatMetric, roleLabel, signed } from "../lib/format";
import type { ChampionDetail, ChampionSummary, GoalInput, MetricDef } from "../types";

const pct = (wins: number, games: number) => (games ? Math.round((wins / games) * 100) : 0);

export function Champions({
  refreshKey,
  champion,
  onChampion,
  onOpenMatch,
  onMakeGoal,
}: {
  refreshKey: number;
  champion: string | null;
  onChampion: (champion: string | null) => void;
  onOpenMatch: (id: string) => void;
  onMakeGoal: (draft: GoalInput) => void;
}) {
  const [list, setList] = useState<ChampionSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const name = useChampionName();
  useEffect(() => {
    api.championList().then(setList, (e) => setError(String(e)));
  }, [refreshKey]);
  const current = list?.find((c) => c.champion === champion) ?? list?.[0] ?? null;

  return (
    <div className="page sessions">
      <div className="sessions-grid">
        <section className="panel session-list">
          <h2>Champions</h2>
          <p className="muted small-text">Your last 300 Summoner's Rift games, most played first.</p>
          {error && <div className="banner error">{error}</div>}
          {list?.length === 0 && <p className="muted">No Summoner's Rift games yet.</p>}
          {list?.map((c) => (
            <button
              key={c.champion}
              className={`session-row champ-row ${current?.champion === c.champion ? "on" : ""}`}
              onClick={() => onChampion(c.champion)}
            >
              <span className="session-when">
                <ChampionIcon id={c.champion_id} patch="latest" size={22} /> {name(c.champion_id, c.champion)}
                {c.focus && (
                  <span className="tag" title="Benchmarks sample players who main this champion first">
                    focus
                  </span>
                )}
              </span>
              <span className="muted small-text">
                {c.games} {c.games === 1 ? "game" : "games"} · {c.roles.map(([r]) => roleLabel(r)).join(", ")}
              </span>
              <span className="session-score">
                <span className={pct(c.wins, c.games) >= 50 ? "good" : "bad"}>{pct(c.wins, c.games)}%</span>
              </span>
            </button>
          ))}
        </section>
        {current && (
          <ChampionView
            key={current.champion}
            champion={current.champion}
            refreshKey={refreshKey}
            onOpenMatch={onOpenMatch}
            onMakeGoal={onMakeGoal}
          />
        )}
      </div>
    </div>
  );
}

function ChampionView({
  champion,
  refreshKey,
  onOpenMatch,
  onMakeGoal,
}: {
  champion: string;
  refreshKey: number;
  onOpenMatch: (id: string) => void;
  onMakeGoal: (draft: GoalInput) => void;
}) {
  const [d, setD] = useState<ChampionDetail | null>(null);
  const [defs, setDefs] = useState<MetricDef[]>([]);
  const [error, setError] = useState<string | null>(null);
  const name = useChampionName();
  useEffect(() => {
    api.championDetail(champion).then(
      (x) => (setD(x), setError(null)),
      (e) => setError(String(e)),
    );
  }, [champion, refreshKey]);
  useEffect(() => {
    api.metricDefinitions().then(setDefs, () => {});
  }, []);

  if (error && !d) return <div className="banner error">{error}</div>;
  if (!d) return <div className="muted">Loading {champion}…</div>;
  const s = d.summary;
  const fmt = (k: string, v: number) => {
    const def = defs.find((x) => x.key === k);
    return def ? formatMetric(v, def.unit, Math.max(def.decimals, 1)) : v.toFixed(1);
  };
  const weak = d.signals.filter((x) => x.polarity === "weakness");
  const strong = d.signals.filter((x) => x.polarity === "strength");

  return (
    <div className="stack">
      <section className="panel">
        <div className="panel-head">
          <ChampionIcon id={s.champion_id} patch="latest" size={44} />
          <div>
            <h2>{name(s.champion_id, s.champion)}</h2>
            <div className="muted">
              {s.games} {s.games === 1 ? "game" : "games"} · {s.wins}W {s.games - s.wins}L (
              {pct(s.wins, s.games)}%) · {s.roles.map(([r, n]) => `${roleLabel(r)} ${n}`).join(", ")} · last
              played {ago(s.last_played_ms)}
            </div>
          </div>
        </div>
      </section>

      {d.role && d.vs_role.length > 0 && (
        <section className="panel">
          <h2>
            On {name(s.champion_id, s.champion)} vs your other {roleLabel(d.role)} champions
          </h2>
          {d.role_games === 0 ? (
            <p className="muted">
              You haven't played other champions in this role, so there's nothing to compare.
            </p>
          ) : (
            <table>
              <thead>
                <tr>
                  <th>Metric</th>
                  <th className="num">On this champion</th>
                  <th className="num">Other champions</th>
                </tr>
              </thead>
              <tbody>
                {d.vs_role.map((v) => (
                  <tr key={v.metric}>
                    <td>{v.label}</td>
                    <td className={`num strong ${v.better === null ? "" : v.better ? "good" : "bad"}`}>
                      {fmt(v.metric, v.on_champion)}
                    </td>
                    <td className="num muted">
                      {v.role_average === null ? "-" : fmt(v.metric, v.role_average)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {d.role_games > 0 && (
            <p className="muted small-text">
              Averages; "other champions" is your {d.role_games} other {roleLabel(d.role)} games in the same
              window.
            </p>
          )}
        </section>
      )}

      <section className="panel">
        <h2>Lane matchups</h2>
        {d.matchups.length === 0 ? (
          <p className="muted">No lane opponents recorded yet.</p>
        ) : (
          <table>
            <thead>
              <tr>
                <th>Against</th>
                <th className="num">Games</th>
                <th className="num">Win rate</th>
                <th className="num" title="Your gold difference with them at 15:00">
                  Gold @15
                </th>
                <th className="num" title="Your CS difference with them at 10:00">
                  CS @10
                </th>
              </tr>
            </thead>
            <tbody>
              {d.matchups.map((m) => (
                <tr key={m.champion}>
                  <td>
                    <span className="champ-cell">
                      <ChampionIcon id={m.champion_id} patch="latest" size={20} />{" "}
                      {name(m.champion_id, m.champion)}
                    </span>
                  </td>
                  <td className="num">{m.games}</td>
                  <td className={`num ${pct(m.wins, m.games) >= 50 ? "good" : "bad"}`}>
                    {pct(m.wins, m.games)}%
                  </td>
                  <td className="num">
                    {m.gold_diff_15 === null ? "-" : signed(Math.round(m.gold_diff_15))}
                  </td>
                  <td className="num">{m.cs_diff_10 === null ? "-" : signed(Math.round(m.cs_diff_10))}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <p className="muted small-text">The enemy in your lane (same position), averaged over your games.</p>
      </section>
      <section className="panel">
        <h2>Recent games</h2>
        <table className="clickable">
          <tbody>
            {d.recent.map((g) => (
              <tr key={g.match_id} onClick={() => onOpenMatch(g.match_id)}>
                <td className={g.win ? "good" : "bad"}>{g.win ? "Win" : "Loss"}</td>
                <td className="muted small-text">{dateTime(g.start_ms)}</td>
                <td>{roleLabel(g.role)}</td>
                <td>
                  {g.opponent_id !== null && (
                    <span className="champ-cell">
                      vs <ChampionIcon id={g.opponent_id} patch="latest" size={18} />
                    </span>
                  )}
                </td>
                <td className="num mono">
                  {g.kills ?? "-"}/{g.deaths ?? "-"}/{g.assists ?? "-"}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section className="panel">
        <h2>Patterns on this champion</h2>
        {d.signals.length === 0 && (
          <p className="muted">
            No recurring patterns yet{s.games < 5 ? ": play a few more games on it" : ""}.
          </p>
        )}
        {weak.length > 0 && <h3>To fix</h3>}
        {weak.slice(0, 5).map((x) => (
          <SignalCard key={x.id} s={x} onOpenMatch={onOpenMatch} onMakeGoal={onMakeGoal} />
        ))}
        {strong.length > 0 && <h3>Strengths</h3>}
        {strong.slice(0, 3).map((x) => (
          <SignalCard key={x.id} s={x} onOpenMatch={onOpenMatch} />
        ))}
      </section>

      <section className="panel">
        <h2>vs higher ranks on {name(s.champion_id, s.champion)}</h2>
        {d.benchmark ? (
          <GapsTable view={d.benchmark} limit={12} />
        ) : (
          <p className="muted">
            Not enough higher-rank games collected for this role yet. Benchmarks collect your most played
            champions first; pick others in Settings → Benchmarks.
          </p>
        )}
      </section>
    </div>
  );
}

import { useContext, useEffect, useMemo, useState } from "react";
import { api } from "../api";
import { LineChart } from "../components/LineChart";
import { ChampionIcon, ChampionsContext, useChampionName } from "../lib/assets";
import { ago, formatMetric, roleLabel } from "../lib/format";
import type {
  ActivityEntry,
  BucketSnapshot,
  ChampionEntry,
  DashboardView,
  MetricDef,
  StatusView,
} from "../types";

interface Props {
  status: StatusView | null;
  activity: ActivityEntry[];
  refreshKey: number;
  ingestCount: number;
  onOpenMatch: (id: string) => void;
  onGoPlayers: () => void;
  onSync: () => Promise<void>;
}

const CARDS = ["cs_10", "cspm", "deaths_pre14", "kp", "dmg_share", "vision_pm", "gold_diff_15", "obj_part"];

export function Dashboard({
  status,
  activity,
  refreshKey,
  ingestCount,
  onOpenMatch,
  onGoPlayers,
  onSync,
}: Props) {
  const [view, setView] = useState<DashboardView | null>(null);
  const [defs, setDefs] = useState<MetricDef[]>([]);
  const [trend, setTrend] = useState("cs_10");
  const [error, setError] = useState<string | null>(null);
  const name = useChampionName();
  const champions = useContext(ChampionsContext);

  useEffect(() => {
    api.dashboard().then(setView, (e) => setError(String(e)));
  }, [refreshKey, status?.account?.id]);
  useEffect(() => {
    api.metricDefinitions().then(setDefs, () => {});
  }, []);

  const def = useMemo(() => new Map(defs.map((d) => [d.key, d])), [defs]);
  const games = useMemo(() => [...(view?.games ?? [])].reverse(), [view]); // oldest → newest

  const account = status?.account;
  if (!account) {
    return (
      <div className="page">
        <section className="panel hero-empty">
          <h1>Welcome to Rift Coach</h1>
          <p>Add your Riot ID to pull in your recent games. Everything stays on this PC.</p>
          <button onClick={onGoPlayers}>Add my Riot ID</button>
        </section>
      </div>
    );
  }

  const wins = games.filter((g) => g.win).length;
  const avg = (key: string, list = games) => {
    const v = list.map((g) => g.metrics[key]).filter((x): x is number => x !== undefined);
    return v.length ? v.reduce((a, b) => a + b, 0) / v.length : NaN;
  };
  const jobs = status?.engine.jobs;
  const pending = jobs ? jobs.ready + jobs.scheduled + jobs.running : 0;
  const trendDef = def.get(trend);
  const trendValues = games.map((g) => g.metrics[trend] ?? NaN);
  const rolling = trendValues.map((_, i) => {
    const w = trendValues.slice(Math.max(0, i - 4), i + 1).filter(Number.isFinite);
    return w.length ? w.reduce((a, b) => a + b, 0) / w.length : NaN;
  });
  const champs = aggregateChampions(view, champions);

  return (
    <div className="page dashboard">
      {error && <div className="banner error">{error}</div>}
      <section className="panel hero">
        <div>
          <div className="muted">
            Last {games.length} Summoner's Rift games
            {view && games.length < view.baseline_games ? ` (look-back window ${view.baseline_games})` : ""}
          </div>
          <h1>
            {account.game_name}
            <span className="muted">#{account.tag_line}</span>
          </h1>
        </div>
        <div className="hero-stats">
          <Big
            label="Win rate"
            value={games.length ? `${Math.round((wins / games.length) * 100)}%` : "-"}
            sub={`${wins}W ${games.length - wins}L`}
          />
          <Big label="KDA" value={Number.isFinite(avg("kda")) ? avg("kda").toFixed(2) : "-"} sub="average" />
          <Big
            label="Matches stored"
            value={String(status?.counts?.matches ?? 0)}
            sub={`${status?.counts?.with_timeline ?? 0} with timeline`}
          />
        </div>
      </section>

      <div className="cards">
        {CARDS.map((k) => {
          const d = def.get(k);
          if (!d) return null;
          const all = avg(k);
          const recent = avg(k, games.slice(-5));
          const before = avg(k, games.slice(0, -5));
          const delta = recent - before;
          const better =
            d.higher_is_better === null || !Number.isFinite(delta) ? null : delta > 0 === d.higher_is_better;
          return (
            <button
              key={k}
              className={`card ${trend === k ? "on" : ""}`}
              onClick={() => setTrend(k)}
              title={d.description}
            >
              <div className="card-label">{d.label}</div>
              <div className="card-value">{formatMetric(all, d.unit, d.decimals)}</div>
              {Number.isFinite(delta) && games.length > 5 && (
                <div className={`card-delta ${better === null ? "" : better ? "good" : "bad"}`}>
                  last 5: {formatMetric(recent, d.unit, d.decimals)}
                </div>
              )}
            </button>
          );
        })}
      </div>

      <div className="grid-2">
        <section className="panel">
          <div className="panel-head">
            <h2>Trend</h2>
            <div className="spacer" />
            <select value={trend} onChange={(e) => setTrend(e.target.value)} aria-label="Metric">
              {defs.map((d) => (
                <option key={d.key} value={d.key}>
                  {d.label}
                </option>
              ))}
            </select>
          </div>
          {games.length > 1 && trendDef ? (
            <LineChart
              x={games.map((_, i) => i + 1)}
              series={[
                {
                  label: trendDef.label,
                  color: "#40c8c4",
                  values: trendValues.map((v) => (Number.isFinite(v) ? v : 0)),
                },
                {
                  label: "5-game average",
                  color: "#d6ac52",
                  values: rolling.map((v) => (Number.isFinite(v) ? v : 0)),
                  dashed: true,
                },
              ]}
              formatX={(v) => `#${Math.round(v)}`}
              formatY={(v) => formatMetric(v, trendDef.unit, trendDef.decimals)}
            />
          ) : (
            <p className="muted">Play or sync a few games to see trends.</p>
          )}
          <p className="muted small-text">Oldest to newest. Change the look-back window in Settings.</p>
        </section>

        <section className="panel">
          <h2>Champions</h2>
          <table>
            <thead>
              <tr>
                <th>Champion</th>
                <th>Role</th>
                <th className="num">Games</th>
                <th className="num">Win rate</th>
              </tr>
            </thead>
            <tbody>
              {champs.slice(0, 8).map((c) => (
                <tr key={c.role + c.name}>
                  <td>
                    <span className="champ">
                      {c.id !== null && <ChampionIcon id={c.id} patch={c.patch} size={24} />}
                      {c.id !== null ? name(c.id, c.name) : c.name}
                    </span>
                  </td>
                  <td>{roleLabel(c.role)}</td>
                  <td className="num">{c.games}</td>
                  <td className="num">{Math.round((c.wins / c.games) * 100)}%</td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="muted small-text">All stored Summoner's Rift games.</p>
        </section>
      </div>

      <div className="grid-2">
        <section className="panel">
          <h2>Recent games</h2>
          {[...games]
            .reverse()
            .slice(0, 6)
            .map((g) => (
              <button key={g.match_id} className="recent" onClick={() => onOpenMatch(g.match_id)}>
                <ChampionIcon id={g.champion_id} patch="latest" size={32} />
                <span className={`result ${g.win ? "win" : "loss"}`}>{g.win ? "Win" : "Loss"}</span>
                <span>{name(g.champion_id, g.champion_name)}</span>
                <span className="muted">{roleLabel(g.team_position)}</span>
                <span className="mono">
                  {g.metrics.kills ?? 0}/{g.metrics.deaths ?? 0}/{g.metrics.assists ?? 0}
                </span>
                <span className="muted">{ago(g.game_start_ms)}</span>
              </button>
            ))}
        </section>

        <section className="panel">
          <div className="panel-head">
            <h2>Sync & activity</h2>
            <div className="spacer" />
            <button disabled={pending > 0} onClick={onSync}>
              {pending > 0 ? `Working… ${pending} jobs` : `Sync newest ${ingestCount}`}
            </button>
          </div>
          {(status?.engine.rate_limits ?? [])
            .filter((b) => b.scope === "app")
            .map((b) => (
              <LimitRow key={b.routing} bucket={b} />
            ))}
          <div className="activity">
            {activity.length === 0 && <p className="muted">Nothing yet.</p>}
            {activity.slice(0, 40).map((a) => (
              <div key={a.id} className={`act ${a.level}`}>
                <span className="muted">
                  {new Date(a.at).toLocaleString(undefined, { dateStyle: "short", timeStyle: "short" })}
                </span>
                {a.message}
              </div>
            ))}
          </div>
        </section>
      </div>
    </div>
  );
}

function Big({ label, value, sub }: { label: string; value: string; sub: string }) {
  return (
    <div className="big-stat">
      <div className="big-value">{value}</div>
      <div className="big-label">{label}</div>
      <div className="muted small-text">{sub}</div>
    </div>
  );
}

function LimitRow({ bucket }: { bucket: BucketSnapshot }) {
  return (
    <div className="limit-row">
      <span className="routing">{bucket.routing}</span>
      {bucket.windows.map((w) => (
        <span
          key={w.period_s}
          className="meter"
          title={`${w.used} of ${w.limit} requests per ${w.period_s}s`}
        >
          <span className="meter-fill" style={{ width: `${Math.min(100, (w.used / w.limit) * 100)}%` }} />
          <span className="meter-text">
            {w.used}/{w.limit} per {w.period_s}s
          </span>
        </span>
      ))}
    </div>
  );
}

function aggregateChampions(view: DashboardView | null, champions: Map<number, ChampionEntry>) {
  if (!view) return [];
  // Riot's internal champion name is the Data Dragon id ("Leblanc").
  const idOf = new Map([...champions.values()].map((c) => [c.id.toLowerCase(), c.key]));
  for (const g of view.games) idOf.set(g.champion_name.toLowerCase(), g.champion_id);
  return view.roles.map((r) => ({
    name: r.champion_name,
    role: r.team_position,
    games: r.games,
    wins: r.wins,
    id: idOf.get(r.champion_name.toLowerCase()) ?? null,
    patch: "latest",
  }));
}

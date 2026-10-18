import { useEffect, useMemo, useState } from "react";
import { api } from "../api";
import { GapsTable } from "../components/Gaps";
import { LineChart } from "../components/LineChart";
import { ReviewBody } from "../components/ReviewBody";
import { ChampionIcon } from "../lib/assets";
import { dateTime, formatMetric } from "../lib/format";
import type { GoalInput, MetricDef, WeekDetail, WeekSummary } from "../types";

const TREND_COLORS = ["#d6ac52", "#40c8c4", "#d9605a", "#9c8cf0", "#4fb07a", "#e0a84a"];

function weekLabel(week: string): string {
  const d = new Date(`${week}T00:00:00`);
  return `Week of ${d.toLocaleDateString(undefined, { month: "short", day: "numeric" })}`;
}

export function Weekly({
  refreshKey,
  week,
  onWeek,
  onOpenSettings,
  onTrackFocus,
}: {
  refreshKey: number;
  week: string | null;
  onWeek: (week: string | null) => void;
  onOpenSettings: () => void;
  onTrackFocus: (draft: GoalInput) => void;
}) {
  const [list, setList] = useState<WeekSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.listWeeks(52).then(setList, (e) => setError(String(e)));
  }, [refreshKey]);
  const current = list?.find((w) => w.week === week) ?? list?.[0] ?? null;

  return (
    <div className="page sessions">
      <div className="sessions-grid">
        <section className="panel session-list">
          <h2>Weeks</h2>
          <p className="muted small-text">Monday to Sunday, your local time.</p>
          {error && <div className="banner error">{error}</div>}
          {list?.length === 0 && <p className="muted">No games yet.</p>}
          {list?.map((w) => (
            <button
              key={w.week}
              className={`session-row ${current?.week === w.week ? "on" : ""}`}
              onClick={() => onWeek(w.week)}
            >
              <span className="session-when">
                {weekLabel(w.week)}
                {!w.complete && <span className="tag">this week</span>}
              </span>
              <span className="muted small-text">{w.games} games</span>
              <span className="session-score">
                <span className="good">{w.wins}W</span> <span className="bad">{w.losses}L</span>
                {w.has_report && <span className="spark"> ✦</span>}
              </span>
            </button>
          ))}
        </section>
        {current && (
          <WeekView
            key={current.week}
            week={current.week}
            refreshKey={refreshKey}
            onOpenSettings={onOpenSettings}
            onTrackFocus={onTrackFocus}
          />
        )}
      </div>
    </div>
  );
}

function WeekView({
  week,
  refreshKey,
  onOpenSettings,
  onTrackFocus,
}: {
  week: string;
  refreshKey: number;
  onOpenSettings: () => void;
  onTrackFocus: (draft: GoalInput) => void;
}) {
  const [d, setD] = useState<WeekDetail | null>(null);
  const [defs, setDefs] = useState<MetricDef[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [facts, setFacts] = useState<string | null>(null);

  useEffect(() => {
    api.weekDetail(week).then(
      (x) => (setD(x), setError(null)),
      (e) => setError(String(e)),
    );
  }, [week, refreshKey]);
  useEffect(() => {
    api.metricDefinitions().then(setDefs, () => {});
  }, []);

  const labels = useMemo(() => {
    const m = new Map<string, string>([["week", "this week"]]);
    d?.aggregates.forEach((a) => m.set(a.metric, a.label));
    d?.patterns.forEach((p) => m.set(`pattern:${p.kind}`, p.title));
    d?.champions.forEach((c) => m.set(`champ:${c.champion}`, c.champion));
    d?.trend.forEach((t) => m.set(`week:${t.week}`, weekLabel(t.week).toLowerCase()));
    d?.benchmark?.gaps.forEach((g) => m.set(`bench:${g.metric}`, `${g.label} vs benchmark`));
    return m;
  }, [d]);

  if (error && !d) return <div className="banner error">{error}</div>;
  if (!d) return <div className="muted">Loading week…</div>;
  const s = d.summary;

  const run = async (force: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const report = await api.generateWeeklyReport(week, force);
      setD({ ...d, report });
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const showFacts = async () => {
    if (facts) return setFacts(null);
    try {
      setFacts(JSON.stringify(await api.weeklyFacts(week), null, 2));
    } catch (e) {
      setError(String(e));
    }
  };

  const defOf = (k: string) => defs.find((x) => x.key === k);
  const trendKeys = Object.keys(d.trend.at(-1)?.metrics ?? {});

  return (
    <div className="stack">
      <section className="panel">
        <div className="panel-head">
          <div>
            <h2>{weekLabel(s.week)}</h2>
            <div className="muted">
              {s.games} games · {s.wins}W {s.losses}L{!s.complete && " · still in progress"}
            </div>
          </div>
          <div className="spacer" />
          <div className="champ-strip">
            {d.champions.map((c) => (
              <span
                key={c.champion}
                className="champ-chip"
                title={`${c.champion}: ${c.wins}W ${c.games - c.wins}L`}
              >
                <ChampionIcon id={c.champion_id} patch="latest" size={24} />
                <span className="small-text">
                  {c.games} · {Math.round((c.wins / c.games) * 100)}%
                </span>
              </span>
            ))}
          </div>
        </div>
      </section>

      <section className="panel review">
        <div className="panel-head">
          <h2>
            <span className="spark">✦</span> AI weekly report
          </h2>
          <div className="spacer" />
          {d.report && (
            <span className="muted small-text">
              {d.report.model} · {dateTime(d.report.created_at)}
            </span>
          )}
          <button className="secondary small" onClick={showFacts}>
            {facts ? "Hide facts" : "Facts sent"}
          </button>
          <button disabled={busy || s.rift_games === 0} onClick={() => run(!!d.report)}>
            {busy ? "Thinking…" : d.report ? "Regenerate" : "Write the weekly report"}
          </button>
        </div>
        {error && (
          <div className="banner error">
            {error}{" "}
            {/key|model|provider|cap/i.test(error) && (
              <button className="link" onClick={onOpenSettings}>
                Open AI settings
              </button>
            )}
          </div>
        )}
        {facts && <pre className="facts">{facts}</pre>}
        {!d.report && !busy && (
          <p className="muted">
            A look at the whole week: what improved or slipped against the four weeks before, your recurring
            habits, the biggest gaps to higher-ranked players, and one focus for next week.
            {!s.complete && " The week isn't over yet; a report now covers the games so far."}
          </p>
        )}
        {d.report && (
          <ReviewBody
            report={d.report}
            labelOf={(id) => labels.get(id) ?? id}
            onTrackFocus={onTrackFocus}
            focusLabel="Next week's focus"
          />
        )}
      </section>

      {trendKeys.length > 0 && (
        <section className="panel">
          <h2>Last {d.trend.length} weeks</h2>
          <div className="trend-grid">
            {trendKeys.map((k, i) => {
              const def = defOf(k);
              const points = d.trend.filter((t) => t.metrics[k] !== undefined);
              return (
                <div key={k}>
                  <div className="small-text">
                    <b>{def?.label ?? k}</b>
                    {def?.higher_is_better === false && <span className="muted"> (lower is better)</span>}
                  </div>
                  <LineChart
                    height={120}
                    x={points.map((_, j) => j)}
                    series={[
                      {
                        label: def?.label ?? k,
                        color: TREND_COLORS[i % TREND_COLORS.length] ?? "#d6ac52",
                        values: points.map((t) => t.metrics[k] ?? 0),
                      },
                    ]}
                    formatX={(j) => points[Math.round(j)]?.week.slice(5) ?? ""}
                    formatY={(v) =>
                      def ? formatMetric(v, def.unit, Math.max(def.decimals, 1)) : v.toFixed(1)
                    }
                  />
                </div>
              );
            })}
          </div>
        </section>
      )}

      <div className="grid-2 even">
        <section className="panel">
          <h2>This week vs the 4 before</h2>
          <table>
            <thead>
              <tr>
                <th>Metric</th>
                <th className="num">This week</th>
                <th className="num">Before</th>
              </tr>
            </thead>
            <tbody>
              {d.aggregates.map((a) => (
                <tr key={a.metric}>
                  <td>{a.label}</td>
                  <td className={`num ${a.better === null ? "" : a.better ? "good" : "bad"}`}>
                    {formatMetric(a.session, a.unit, Math.max(a.decimals, 1))}
                  </td>
                  <td className="num muted">
                    {a.baseline === null ? "-" : formatMetric(a.baseline, a.unit, Math.max(a.decimals, 1))}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="muted small-text">
            "Before" is your average over {d.baseline_games} games in the 4 weeks before.
          </p>
        </section>
        <section className="panel">
          <h2>Patterns this week</h2>
          {d.patterns.length === 0 && <p className="muted">None of the tracked patterns showed up.</p>}
          {d.patterns.map((p) => (
            <div key={p.kind} className={`signal ${p.polarity === "weakness" ? "weak" : "strong"}`}>
              <div className="signal-head">
                <b>{p.title}</b>
                <span className="spacer" />
                <span className="tag">
                  {p.session_games_affected}/{p.session_games} games
                </span>
                {p.baseline_rate_pct !== null && <span className="tag">usually {p.baseline_rate_pct}%</span>}
              </div>
              <p className="muted small-text">{p.what}.</p>
            </div>
          ))}
        </section>
      </div>

      {d.benchmark && (
        <section className="panel">
          <h2>This week vs higher ranks</h2>
          <GapsTable view={d.benchmark} limit={10} />
        </section>
      )}
    </div>
  );
}

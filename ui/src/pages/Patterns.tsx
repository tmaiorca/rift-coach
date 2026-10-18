import { useEffect, useMemo, useState } from "react";
import { api } from "../api";
import { DeathHeatmap } from "../components/DeathHeatmap";
import { SignalCard } from "../components/SignalCard";
import { formatMetric, roleLabel } from "../lib/format";
import type { GoalInput, MetricDef, PatternsView, WinLink } from "../types";

const WINDOWS = [20, 30, 50, 100, 200];

export function Patterns({
  refreshKey,
  baselineGames,
  onOpenMatch,
  onMakeGoal,
}: {
  refreshKey: number;
  baselineGames: number;
  onOpenMatch: (id: string) => void;
  onMakeGoal: (draft: GoalInput) => void;
}) {
  const [view, setView] = useState<PatternsView | null>(null);
  const [defs, setDefs] = useState<MetricDef[]>([]);
  const [span, setSpan] = useState(baselineGames);
  const [role, setRole] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.patterns(span, role).then(
      (v) => (setView(v), setError(null)),
      (e) => setError(String(e)),
    );
  }, [span, role, refreshKey]);
  useEffect(() => {
    api.metricDefinitions().then(setDefs, () => {});
  }, []);

  const def = useMemo(() => new Map(defs.map((d) => [d.key, d])), [defs]);
  const weak = view?.signals.filter((s) => s.polarity === "weakness") ?? [];
  const strong = view?.signals.filter((s) => s.polarity === "strength") ?? [];
  const sel = view?.signals.find((s) => s.id === selected) ?? null;
  const windows = [...new Set([...WINDOWS, baselineGames])].sort((a, b) => a - b);

  return (
    <div className="page patterns">
      <section className="panel">
        <div className="panel-head">
          <h2>Patterns</h2>
          <div className="spacer" />
          <select value={role ?? ""} onChange={(e) => setRole(e.target.value || null)} aria-label="Role">
            <option value="">All roles</option>
            {(view?.roles ?? []).map(([r, n]) => (
              <option key={r} value={r}>
                {roleLabel(r)} ({n})
              </option>
            ))}
          </select>
          <select value={span} onChange={(e) => setSpan(Number(e.target.value))} aria-label="Games">
            {windows.map((w) => (
              <option key={w} value={w}>
                Last {w} games
              </option>
            ))}
          </select>
        </div>
        <p className="muted">
          Habits that repeat across your Summoner's Rift games, found by fixed rules and compared with players
          in your role in your own games.
          {view && ` ${view.games} games, ${view.wins} wins.`} Recurring and costly patterns are listed first.
        </p>
        {error && <div className="banner error">{error}</div>}
        {view && view.games < 5 && <p className="muted">Sync more games to find patterns (at least 5).</p>}
      </section>

      <div className="grid-2">
        <section className="panel">
          <h2 className="bad">Fix these first</h2>
          {view && weak.length === 0 && <p className="muted">No recurring weaknesses in this window.</p>}
          {weak.map((s) => (
            <SignalCard
              key={s.id}
              s={s}
              selected={s.id === selected}
              onSelect={() => setSelected(s.id === selected ? null : s.id)}
              onOpenMatch={onOpenMatch}
              onMakeGoal={onMakeGoal}
            />
          ))}
        </section>
        <div className="stack">
          <section className="panel">
            <h2>Where you die</h2>
            {view && (
              <DeathHeatmap deaths={view.deaths} highlight={sel?.evidence ?? []} onOpenMatch={onOpenMatch} />
            )}
          </section>
          <section className="panel">
            <h2 className="good">Strengths</h2>
            {view && strong.length === 0 && <p className="muted">Nothing stands out yet.</p>}
            {strong.map((s) => (
              <SignalCard key={s.id} s={s} onOpenMatch={onOpenMatch} />
            ))}
          </section>
        </div>
      </div>

      {view && view.win_links.length > 0 && (
        <section className="panel">
          <h2>What wins your games</h2>
          <p className="muted">
            How your numbers differ between your wins and losses. This is correlation, not cause: late-game
            numbers mostly reflect who was already winning, so the early-game table (things you control before
            about 15:00) is the one to act on.
          </p>
          <div className="grid-2 even">
            <WinTable
              title="Early game (you control)"
              links={view.win_links.filter((l) => l.early)}
              def={def}
            />
            <WinTable
              title="Whole game (mostly reflects the result)"
              links={view.win_links.filter((l) => !l.early)}
              def={def}
            />
          </div>
        </section>
      )}
    </div>
  );
}

const STRENGTH: Record<WinLink["strength"], string> = {
  strong: "Strong",
  moderate: "Moderate",
  weak: "Weak",
  none: "None",
};

function WinTable({ title, links, def }: { title: string; links: WinLink[]; def: Map<string, MetricDef> }) {
  const shown = links.filter((l) => l.strength !== "none").slice(0, 10);
  return (
    <div>
      <h3>{title}</h3>
      {shown.length === 0 ? (
        <p className="muted">No clear links yet.</p>
      ) : (
        <table>
          <thead>
            <tr>
              <th>Metric</th>
              <th className="num">Wins</th>
              <th className="num">Losses</th>
              <th className="num" title="Your win rate when above / below your own median">
                WR above / below
              </th>
              <th>Link</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((l) => {
              const d = def.get(l.metric);
              const f = (v: number) => (d ? formatMetric(v, d.unit, d.decimals) : v.toFixed(1));
              return (
                <tr key={l.metric} title={l.summary}>
                  <td>{l.label}</td>
                  <td className="num good">{f(l.win_median)}</td>
                  <td className="num bad">{f(l.loss_median)}</td>
                  <td className="num">
                    {l.win_rate_above ?? "-"}% / {l.win_rate_below ?? "-"}%
                  </td>
                  <td>
                    <span className={`tag link-${l.strength}`}>{STRENGTH[l.strength]}</span>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
      <p className="muted small-text">
        Medians in your wins and losses. "Strong" means unlikely to be chance.
      </p>
    </div>
  );
}

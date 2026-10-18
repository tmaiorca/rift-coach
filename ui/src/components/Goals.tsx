// Focus goal progress card and the form to create one.
import { useMemo, useState } from "react";
import { ChampionIcon } from "../lib/assets";
import { formatMetric, ROLE_LABEL } from "../lib/format";
import type { Comparator, Goal, GoalInput, MetricDef } from "../types";

export function GoalCard({
  goal: g,
  compact = false,
  onOpenMatch,
  onStatus,
  onDelete,
}: {
  goal: Goal;
  compact?: boolean;
  onOpenMatch?: (id: string) => void;
  onStatus?: (status: Goal["status"]) => void;
  onDelete?: () => void;
}) {
  const p = g.progress;
  const fmt = (v: number | null) => (v === null ? "-" : formatMetric(v, g.unit, Math.max(g.decimals, 1)));
  const state =
    g.status === "completed"
      ? g.achieved
        ? { cls: "ok", text: "Achieved" }
        : { cls: "warn", text: "Missed" }
      : g.status === "abandoned"
        ? { cls: "", text: "Dropped" }
        : p.games_played === 0
          ? { cls: "", text: "Starts next game" }
          : p.on_track
            ? { cls: "ok", text: "On track" }
            : { cls: "warn", text: "Off track" };
  const better = p.improvement === null ? null : p.improvement > 0;

  return (
    <div className={`goal-card ${compact ? "compact" : ""}`}>
      <div className="goal-head">
        <b>{g.title}</b>
        <span className={`tag ${state.cls}`}>{state.text}</span>
        {g.source !== "user" && (
          <span className="tag">{g.source === "ai" ? "from AI coach" : "from a pattern"}</span>
        )}
        {g.role && <span className="tag">{ROLE_LABEL[g.role] ?? g.role} only</span>}
      </div>
      <div className="goal-progress">
        <div className="goal-bar" title={`${p.games_played} of ${p.games_total} games`}>
          {Array.from({ length: p.games_total }, (_, i) => {
            const game = p.games[i];
            return (
              <button
                key={i}
                className={`goal-pip ${game ? (game.met ? "met" : "miss") : "todo"}`}
                disabled={!game || !onOpenMatch}
                title={
                  game
                    ? `${fmt(game.value)} on ${game.champion_name}${game.win ? " (win)" : " (loss)"}`
                    : "to play"
                }
                onClick={() => game && onOpenMatch?.(game.match_id)}
              />
            );
          })}
        </div>
        <span className="muted small-text">
          {p.games_played}/{p.games_total} games · target met in {p.games_met}
        </span>
      </div>
      <div className="goal-stats">
        <span>
          <span className="muted">Average so far</span> <b>{fmt(p.average)}</b>
        </span>
        <span>
          <span className="muted">Before the goal</span> <b>{fmt(g.baseline)}</b>
        </span>
        {better !== null && (
          <span className={better ? "good" : "bad"}>{better ? "Improving" : "Not improving yet"}</span>
        )}
      </div>
      {!compact && g.note && <p className="muted small-text">{g.note}</p>}
      {!compact && p.games.length > 0 && (
        <div className="goal-games">
          {p.games.map((x) => (
            <button key={x.match_id} className="recent" onClick={() => onOpenMatch?.(x.match_id)}>
              <ChampionIcon id={x.champion_id} patch="latest" size={22} />
              <span className={`result ${x.win ? "win" : "loss"}`}>{x.win ? "W" : "L"}</span>
              <span className={x.met ? "good" : "bad"}>{fmt(x.value)}</span>
              <span className="muted small-text">{new Date(x.game_start_ms).toLocaleDateString()}</span>
            </button>
          ))}
        </div>
      )}
      {!compact && (onStatus || onDelete) && (
        <div className="form-row">
          {g.status === "active" && onStatus && (
            <>
              <button className="secondary small" onClick={() => onStatus("completed")}>
                Finish now
              </button>
              <button className="secondary small" onClick={() => onStatus("abandoned")}>
                Drop
              </button>
            </>
          )}
          {g.status !== "active" && onStatus && (
            <button className="secondary small" onClick={() => onStatus("active")}>
              Reopen
            </button>
          )}
          {onDelete && (
            <button className="secondary small danger" onClick={onDelete}>
              Delete
            </button>
          )}
        </div>
      )}
    </div>
  );
}

/** Remount (change `key`) to load a different draft. */
export function GoalForm({
  defs,
  draft,
  onCreate,
  onCancel,
}: {
  defs: MetricDef[];
  draft: GoalInput | null;
  onCreate: (input: GoalInput) => Promise<void>;
  onCancel?: () => void;
}) {
  const usable = useMemo(() => defs.filter((d) => d.higher_is_better !== null), [defs]);
  const [metric, setMetric] = useState(draft?.metric ?? "deaths_pre14");
  const def = usable.find((d) => d.key === metric);
  const [comparator, setComparator] = useState<Comparator>(draft?.comparator ?? "at_most");
  const [target, setTarget] = useState(String(draft?.target ?? 1));
  const [games, setGames] = useState(draft?.games ?? 10);
  const [role, setRole] = useState<string>(draft?.role ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const pickMetric = (key: string) => {
    setMetric(key);
    const d = usable.find((x) => x.key === key);
    if (d) setComparator(d.higher_is_better ? "at_least" : "at_most");
  };

  const submit = async () => {
    const t = Number(target);
    if (!Number.isFinite(t)) return setError("The target must be a number.");
    setBusy(true);
    setError(null);
    try {
      await onCreate({
        metric,
        comparator,
        target: t,
        games,
        role: role || null,
        champion_name: draft?.champion_name ?? null,
        source: draft?.source ?? "user",
        note: draft?.note ?? null,
      });
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="goal-form">
      <div className="form-row">
        <select value={metric} onChange={(e) => pickMetric(e.target.value)} aria-label="Metric">
          {usable.map((d) => (
            <option key={d.key} value={d.key}>
              {d.label}
            </option>
          ))}
        </select>
        <select
          value={comparator}
          onChange={(e) => setComparator(e.target.value as Comparator)}
          aria-label="Comparator"
        >
          <option value="at_most">at most</option>
          <option value="at_least">at least</option>
        </select>
        <input
          className="num-input"
          value={target}
          onChange={(e) => setTarget(e.target.value)}
          aria-label="Target"
          inputMode="decimal"
        />
        {def?.unit === "minutes" && <span className="muted small-text">minutes (e.g. 11.5 = 11:30)</span>}
        {def?.unit === "percent" && <span className="muted small-text">%</span>}
        <span className="muted">on average over</span>
        <input
          className="num-input"
          type="number"
          min={1}
          max={100}
          value={games}
          onChange={(e) => setGames(Number(e.target.value))}
          aria-label="Games"
        />
        <span className="muted">games</span>
        <select value={role} onChange={(e) => setRole(e.target.value)} aria-label="Role">
          <option value="">any role</option>
          {Object.entries(ROLE_LABEL).map(([k, v]) => (
            <option key={k} value={k}>
              {v} only
            </option>
          ))}
        </select>
      </div>
      {def && <p className="muted small-text">{def.description}</p>}
      {error && <div className="banner error">{error}</div>}
      <div className="form-row">
        <button disabled={busy} onClick={submit}>
          {busy ? "Saving…" : "Start goal"}
        </button>
        {onCancel && (
          <button className="secondary" onClick={onCancel}>
            Cancel
          </button>
        )}
      </div>
    </div>
  );
}

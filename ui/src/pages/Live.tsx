import { useContext, useEffect, useMemo, useState } from "react";
import { api } from "../api";
import { LiveBoard, lastGameText, phaseText, useLiveState } from "../components/LiveBoard";
import { ChampionsContext, ItemIcon } from "../lib/assets";
import { ago, clock, dateTime } from "../lib/format";
import type { BuildPlanView, LiveGameView, PlannerItem, StatusView } from "../types";

interface Props {
  status: StatusView | null;
  refreshKey: number;
  /** Data Dragon id of your most-played champion (planner default). */
  defaultChampion: string | null;
  onOpenMatch: (id: string) => void;
  onOpenSettings: () => void;
}

export function Live({ status, refreshKey, defaultChampion, onOpenMatch, onOpenSettings }: Props) {
  const live = useLiveState();
  const [games, setGames] = useState<LiveGameView[]>([]);
  const phase = live?.phase;
  useEffect(() => {
    api.liveGames(20).then(setGames, () => {});
  }, [refreshKey, phase]);

  const showBoard = live?.snapshot && (live.phase === "in_game" || live.phase === "ended");
  const last = games[0];
  const liveChampion = live?.context?.champion_ddragon_id ?? null;

  return (
    <div className="page live-page">
      <section className="panel">
        <div className="panel-head">
          <h2>Live</h2>
          <span className={`pill live-${phase ?? "idle"}`}>{phaseText(live)}</span>
          {status?.engine.in_game && <span className="tag">background work paused</span>}
          <div className="spacer" />
          <button className="secondary" onClick={() => void api.openLiveWindow()}>
            Pop out window
          </button>
        </div>
        {live && !live.enabled && (
          <p>
            Turn on live game detection in <a onClick={onOpenSettings}>Settings → Live</a>.
          </p>
        )}
        {showBoard && live ? (
          <>
            {!live.tracked && (
              <div className="banner warn">
                This Riot ID isn't tracked, so the game won't be analyzed afterwards. Add it on the Players
                page.
              </div>
            )}
            <LiveBoard state={live} />
          </>
        ) : (
          live?.enabled && (
            <p className="muted">
              Rift Coach checks for a running game every few seconds. When one starts it pauses match
              fetching, analysis, and AI work until the game ends, and shows your CS pacing, gold toward your
              next planned item, KDA, and focus goals here. Put this window (or the pop-out) on a second
              monitor. It never draws over the game.
            </p>
          )
        )}
      </section>

      {last && last.ended_at && (
        <section className="panel">
          <div className="panel-head">
            <h2>Last game</h2>
            <span className="muted small-text">{ago(last.ended_at)}</span>
          </div>
          <p>
            <b>{last.champion_name}</b> · {last.game_mode}
            {last.result && (
              <span className={`tag ${last.result === "Win" ? "ok" : "bad"}`}>
                {last.result === "Win" ? "Victory" : "Defeat"}
              </span>
            )}
            {last.duration_s != null && <span className="muted"> · {clock(last.duration_s)}</span>}
          </p>
          <p className="muted">{lastGameText(last)}</p>
          {last.match_id && (last.status === "analyzed" || last.status === "reviewed") && (
            <button onClick={() => onOpenMatch(last.match_id ?? "")}>
              {last.status === "reviewed" ? "Open game and AI review" : "Open game"}
            </button>
          )}
        </section>
      )}

      <BuildPlanner
        key={liveChampion ?? defaultChampion ?? "none"}
        initial={liveChampion ?? defaultChampion}
      />

      <Footprints games={games} />
    </div>
  );
}

function BuildPlanner({ initial }: { initial: string | null }) {
  const champs = useContext(ChampionsContext);
  const list = useMemo(() => [...champs.values()].sort((a, b) => a.name.localeCompare(b.name)), [champs]);
  const [champion, setChampion] = useState<string | null>(initial);
  const [plan, setPlan] = useState<BuildPlanView | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!champion) return;
    api.buildPlan(champion).then(setPlan, (e) => setError(String(e)));
  }, [champion]);

  const save = async (items: number[]) => {
    if (!champion) return;
    setError(null);
    try {
      setPlan(await api.setBuildPlan(champion, items));
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <section className="panel">
      <div className="panel-head">
        <h2>Planned build</h2>
        <div className="spacer" />
        <select
          value={champion ?? ""}
          onChange={(e) => {
            setPlan(null);
            setChampion(e.target.value || null);
          }}
          aria-label="Champion"
        >
          <option value="">Choose a champion</option>
          {list.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name}
            </option>
          ))}
        </select>
      </div>
      <p className="muted">
        The live view tracks gold toward the next item here, counting components you already own. With no
        saved plan it uses your most common items on the champion.
      </p>
      {error && <div className="banner error">{error}</div>}
      {plan && (
        <PlanEditor
          key={`${plan.champion}:${plan.source}:${plan.items.map((i) => i.item_id).join(",")}`}
          plan={plan}
          onSave={save}
        />
      )}
    </section>
  );
}

function PlanEditor({ plan, onSave }: { plan: BuildPlanView; onSave: (items: number[]) => Promise<void> }) {
  const [draft, setDraft] = useState(plan.items);
  const [catalog, setCatalog] = useState<PlannerItem[]>([]);
  const [query, setQuery] = useState("");
  useEffect(() => {
    api.plannerItems().then(
      (r) => setCatalog(r.items),
      () => {},
    );
  }, []);

  const changed = draft.map((i) => i.item_id).join(",") !== plan.items.map((i) => i.item_id).join(",");
  const q = query.trim().toLowerCase();
  const matches = q
    ? catalog
        .filter((i) => i.name.toLowerCase().includes(q) && !draft.some((d) => d.item_id === i.item_id))
        .slice(0, 12)
    : [];
  const move = (k: number, d: number) =>
    setDraft((list) => {
      const next = [...list];
      const [x] = next.splice(k, 1);
      if (x) next.splice(Math.max(0, Math.min(next.length, k + d)), 0, x);
      return next;
    });

  return (
    <div className="plan-editor">
      <p className="small-text">
        {plan.source === "plan"
          ? "Your saved plan:"
          : plan.source === "usual"
            ? "No saved plan; your usual items (by how often you finish them, not purchase order):"
            : "No saved plan and no games on this champion yet. Add items below."}
      </p>
      <ol className="plan-list">
        {draft.map((i, k) => (
          <li key={i.item_id}>
            <ItemIcon id={i.item_id} patch={plan.patch} size={28} />
            <span className="grow">{i.name}</span>
            <span className="muted small-text mono">{i.total_gold.toLocaleString()}</span>
            <button
              className="secondary small"
              disabled={k === 0}
              onClick={() => move(k, -1)}
              aria-label="Earlier"
            >
              ↑
            </button>
            <button
              className="secondary small"
              disabled={k === draft.length - 1}
              onClick={() => move(k, 1)}
              aria-label="Later"
            >
              ↓
            </button>
            <button
              className="secondary small"
              onClick={() => setDraft((d) => d.filter((x) => x.item_id !== i.item_id))}
              aria-label="Remove"
            >
              ✕
            </button>
          </li>
        ))}
      </ol>
      {draft.length < 8 && (
        <div className="plan-add">
          <input
            placeholder="Add an item…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            aria-label="Search items"
          />
          {matches.length > 0 && (
            <div className="plan-results">
              {matches.map((i) => (
                <button
                  key={i.item_id}
                  className="secondary small"
                  onClick={() => {
                    setDraft((d) => [...d, { item_id: i.item_id, name: i.name, total_gold: i.total_gold }]);
                    setQuery("");
                  }}
                >
                  <ItemIcon id={i.item_id} patch={plan.patch} size={20} /> {i.name}
                </button>
              ))}
            </div>
          )}
        </div>
      )}
      <div className="form-row">
        <button
          disabled={!changed && plan.source === "plan"}
          onClick={() => void onSave(draft.map((i) => i.item_id))}
        >
          Save plan
        </button>
        {changed && (
          <button className="secondary" onClick={() => setDraft(plan.items)}>
            Undo changes
          </button>
        )}
        {plan.source === "plan" && (
          <button className="secondary" onClick={() => void onSave([])}>
            Use my usual items instead
          </button>
        )}
      </div>
    </div>
  );
}

function Footprints({ games }: { games: LiveGameView[] }) {
  const measured = games.filter((g) => g.footprint);
  const first = measured[0];
  return (
    <section className="panel">
      <h2>App footprint during games</h2>
      <p className="muted">
        Measured every 5 seconds while a game runs: Rift Coach's backend plus its windows (WebView2), as
        private memory. Budget:{" "}
        {first
          ? `${first.budget_cpu_avg_pct}% CPU on average and ${first.budget_mem_mb} MB memory`
          : "1% CPU on average and 400 MB memory"}
        . CPU is a share of the whole processor (all cores).
      </p>
      {measured.length === 0 ? (
        <p className="muted">Nothing measured yet. Play a game with Rift Coach open.</p>
      ) : (
        <table>
          <thead>
            <tr>
              <th>Game</th>
              <th>CPU avg</th>
              <th>CPU peak</th>
              <th>Memory peak</th>
              <th>Backend only</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {measured.map((g) => {
              const f = g.footprint;
              if (!f) return null;
              return (
                <tr key={g.id}>
                  <td>
                    {g.champion_name} <span className="muted small-text">{dateTime(g.started_at)}</span>
                  </td>
                  <td className="mono">{f.cpu_avg_pct.toFixed(2)}%</td>
                  <td className="mono">{f.cpu_max_pct.toFixed(1)}%</td>
                  <td className="mono">{Math.round(f.mem_max_mb)} MB</td>
                  <td className="mono muted">
                    {f.backend_cpu_avg_pct.toFixed(2)}% · {Math.round(f.backend_mem_max_mb)} MB
                  </td>
                  <td>
                    <span className={`tag ${g.within_budget ? "ok" : "bad"}`}>
                      {g.within_budget ? "within budget" : "over budget"}
                    </span>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </section>
  );
}

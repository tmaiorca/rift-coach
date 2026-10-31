// The in-game board: your own numbers and pre-set targets only (no advice,
// no alerts, nothing about other players). Shared by the Live page and the
// compact pop-out window.
import { useEffect, useState } from "react";
import { api, onLiveUpdate } from "../api";
import { ChampionIcon, ItemIcon } from "../lib/assets";
import { clock, signed } from "../lib/format";
import type { GoalLive, LiveGameView, LiveState } from "../types";

/** The newest live state, kept current by `live-update` events. */
export function useLiveState(): LiveState | null {
  const [state, setState] = useState<LiveState | null>(null);
  useEffect(() => {
    api.liveState().then(setState, () => {});
    const un = onLiveUpdate(setState);
    return () => {
      void un.then((f) => f());
    };
  }, []);
  return state;
}

/** Game clock that ticks between polls. */
function useGameClock(state: LiveState): number {
  const [now, setNow] = useState(() => Date.now());
  const running = state.phase === "in_game";
  useEffect(() => {
    if (!running) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [running]);
  const t = state.snapshot?.game_time_s ?? 0;
  return running ? t + Math.max(0, Math.min(5, (now - state.updated_ms) / 1000)) : t;
}

const TARGET_SOURCE = { goal: "your goal", setting: "your target", usual: "your usual" } as const;

const STATUS_TEXT: Record<LiveGameView["status"], string> = {
  in_progress: "In progress",
  not_tracked: "Not one of your tracked Riot IDs, so it isn't analyzed.",
  not_published: "Practice tool games never reach Riot's match history.",
  waiting: "Waiting for Riot to publish the match (usually 1–5 minutes)…",
  analyzing: "Found it. Analyzing…",
  analyzed: "Analyzed.",
  reviewed: "Analyzed, with an AI review.",
};

export function lastGameText(g: LiveGameView): string {
  return STATUS_TEXT[g.status];
}

const fmtGold = (n: number) => Math.round(n).toLocaleString();

function goalValue(g: GoalLive): string {
  if (g.value == null) return g.pending ?? "-";
  const decimals = ["cspm", "kda", "vision_pm"].includes(g.metric) ? 1 : 0;
  return g.value.toFixed(decimals);
}

export function LiveBoard({ state, compact = false }: { state: LiveState; compact?: boolean }) {
  const t = useGameClock(state);
  const s = state.snapshot;
  const ctx = state.context;
  if (!s) return null;
  const patch = ctx?.patch ?? "";
  const p = state.pacing;
  const b = state.build;
  const sc = s.scores;
  const ended = state.phase === "ended";

  return (
    <div className={`live-board ${compact ? "compact" : ""}`}>
      <div className="live-top">
        {ctx?.champion_id != null && patch ? (
          <ChampionIcon id={ctx.champion_id} patch={patch} size={compact ? 40 : 52} />
        ) : (
          <span className="icon placeholder" style={{ width: 40, height: 40 }} />
        )}
        <div className="live-who">
          <b>{s.champion_name}</b>
          <span className="muted small-text">
            Level {s.level}
            {s.is_dead ? " · dead" : ""}
          </span>
        </div>
        <div className="live-clock mono">{ended ? (s.game_end ?? "Ended") : clock(t)}</div>
      </div>

      <div className="live-stats">
        <div className="live-stat">
          <span className="live-label">KDA</span>
          <span className="live-big mono">
            {sc.kills}/{sc.deaths}/{sc.assists}
          </span>
        </div>
        <div className="live-stat">
          <span className="live-label">CS</span>
          <span className="live-big mono">{sc.creepScore}</span>
          <span className="muted small-text">{p ? `${p.cs_per_min.toFixed(1)}/min` : ""}</span>
        </div>
        <div className="live-stat">
          <span className="live-label">Gold</span>
          <span className="live-big mono">{fmtGold(s.current_gold)}</span>
        </div>
      </div>

      {p && (
        <div className="live-card">
          <div className="live-card-head">
            <span>CS pacing</span>
            {p.target && (
              <span className="muted small-text">
                {p.target.per_min.toFixed(1)}/min ({TARGET_SOURCE[p.target.source]})
              </span>
            )}
          </div>
          {p.target == null ? (
            <p className="muted small-text">
              No target yet: set one in Settings → Live, or play a few games in this role.
            </p>
          ) : p.expected_cs == null ? (
            <p className="muted small-text">Pacing starts at 5:00.</p>
          ) : (
            <>
              <PaceBar cs={p.cs} expected={p.expected_cs} />
              <p className="small-text">
                <span className="mono">{p.cs}</span> vs{" "}
                <span className="mono">{Math.round(p.expected_cs)}</span> at this point{" "}
                <span className={`mono ${p.diff != null && p.diff >= 0 ? "good" : "muted"}`}>
                  ({signed(Math.round(p.diff ?? 0))})
                </span>
              </p>
            </>
          )}
          {p.benchmark && (
            <p className="muted small-text">
              Benchmark: {p.benchmark.per_min.toFixed(1)}/min, {p.benchmark.label}
            </p>
          )}
        </div>
      )}

      <div className="live-card">
        <div className="live-card-head">
          <span>Build</span>
          {ctx?.build_source && (
            <span className="muted small-text">
              {ctx.build_source === "plan" ? "your plan" : "your usual items"}
            </span>
          )}
        </div>
        {!b ? (
          <p className="muted small-text">No planned build for this champion. Plan one on the Live page.</p>
        ) : (
          <>
            {b.next ? (
              <div className="live-next">
                <ItemIcon id={b.next.item_id} patch={patch} size={compact ? 32 : 38} />
                <div className="grow">
                  <div className="small-text">
                    Next: <b>{b.next.name}</b>
                  </div>
                  <div className="bar">
                    <div className="bar-fill" style={{ width: `${b.next.progress_pct}%` }} />
                  </div>
                  <div className="muted small-text mono">
                    {fmtGold(b.next.total_gold - b.next.remaining_gold)} / {fmtGold(b.next.total_gold)}
                    {b.next.components_gold > 0 && ` (${fmtGold(b.next.components_gold)} in parts)`}
                    {b.next.remaining_gold > 0 ? ` · ${fmtGold(b.next.remaining_gold)} to go` : " · covered"}
                  </div>
                </div>
              </div>
            ) : (
              <p className="small-text">Planned build complete.</p>
            )}
            <div className="live-plan">
              {b.plan.map((i, k) => (
                <span key={`${i.item_id}-${k}`} className={i.owned ? "owned" : ""} title={i.name}>
                  <ItemIcon id={i.item_id} patch={patch} size={compact ? 24 : 28} />
                </span>
              ))}
            </div>
          </>
        )}
      </div>

      {state.goals.length > 0 && (
        <div className="live-card">
          <div className="live-card-head">
            <span>Focus goals</span>
          </div>
          {state.goals.map((g) => (
            <div key={g.goal_id} className="live-goal">
              <span className="small-text grow">{g.title}</span>
              <span className={`mono ${g.met === true ? "good" : g.met === false ? "bad" : ""}`}>
                {goalValue(g)}
              </span>
            </div>
          ))}
        </div>
      )}

      {!compact && state.vision && (
        <p className="muted small-text">
          {state.vision.running
            ? `Screenshots for your after-game vision review: ${state.vision.frames} taken, one every ${state.vision.interval_s.toFixed(0)} s (${state.vision.avg_ms.toFixed(0)} ms each).`
            : "Vision review is on: waiting for the game window to take screenshots."}
          {state.vision.error ? ` ${state.vision.error}` : ""}
        </p>
      )}
      {!compact && (
        <p className="muted small-text">
          Your own numbers from the game client, with targets you set before the game. Rift Coach doesn't
          advise, alert, or show anything about other players during a game.
        </p>
      )}
    </div>
  );
}

function PaceBar({ cs, expected }: { cs: number; expected: number }) {
  // Scale so the target sits at 80% of the bar.
  const max = Math.max(expected / 0.8, cs, 1);
  return (
    <div className="pace-bar">
      <div className="bar-fill" style={{ width: `${Math.min(100, (cs / max) * 100)}%` }} />
      <div className="pace-mark" style={{ left: `${(expected / max) * 100}%` }} />
    </div>
  );
}

/** A one-line description of the live phase. */
export function phaseText(state: LiveState | null): string {
  if (!state) return "…";
  if (!state.enabled) return "Live game detection is off (Settings → Live).";
  switch (state.phase) {
    case "idle":
      return "No game running.";
    case "loading":
      return "Game loading.";
    case "in_game":
      return "In game.";
    case "ended":
      return "Game over.";
  }
}

const RECORD_LABEL: Record<string, string> = {
  cs_10: "CS at 10",
  cs_diff_10: "CS diff at 10",
  gold_diff_15: "gold diff at 15",
  deaths_pre14: "deaths before 14:00",
  solo_deaths: "solo deaths",
};

function recordLine(r: { games: number; wins: number; averages: Record<string, number> }): string {
  const avg = Object.entries(r.averages)
    .map(([k, v]) => `${RECORD_LABEL[k] ?? k} ${k.includes("diff") && v > 0 ? "+" : ""}${v.toFixed(1)}`)
    .join(", ");
  return `${r.games} games, ${r.wins}W ${r.games - r.wins}L${avg ? ` · avg ${avg}` : ""}`;
}

/** The pre-game brief: static content from the loading screen (champions
 * only), plus the optional AI game plan. */
export function BriefCard({ state, compact = false }: { state: LiveState; compact?: boolean }) {
  const [asking, setAsking] = useState(false);
  const [askError, setAskError] = useState<string | null>(null);
  const b = state.brief;
  if (!b) return null;
  const brief = b.brief;
  const patch = state.context?.patch ?? "latest";
  const loading = state.phase === "loading";

  const ask = async () => {
    setAsking(true);
    setAskError(null);
    try {
      await api.generateGamePlan();
    } catch (e) {
      setAskError(String(e));
    } finally {
      setAsking(false);
    }
  };

  return (
    <div className={`live-card brief ${compact ? "compact" : ""}`}>
      <div className="live-card-head">
        <span>Game plan</span>
        {brief && (
          <span className="muted small-text">
            {brief.champion}
            {brief.role ? ` · ${brief.role.toLowerCase()}` : ""}
          </span>
        )}
      </div>
      {b.loading && <p className="muted small-text">Reading the champions in your game…</p>}
      {!b.loading && !brief && <p className="muted small-text">{b.note ?? "No brief for this game."}</p>}
      {brief && (
        <>
          {b.plan_status === "ready" && b.plan && (
            <div className="plan">
              <p className="small-text">
                <b>{b.plan.summary}</b>
              </p>
              <ul className="small-text">
                {b.plan.plan.map((p, i) => (
                  <li key={i}>
                    <b>{p.title}.</b> {p.detail}
                  </li>
                ))}
                {b.plan.watch_out.map((p, i) => (
                  <li key={`w${i}`} className="watch">
                    <b>{p.title}.</b> {p.detail}
                  </li>
                ))}
              </ul>
              <p className="small-text">
                Focus: <b>{b.plan.focus}</b>
              </p>
            </div>
          )}
          {b.plan_status === "generating" && <p className="muted small-text">✦ Writing your game plan…</p>}
          {b.plan_status === "failed" && (
            <p className="bad small-text">AI game plan failed: {b.plan_error}</p>
          )}
          {(b.plan_status === "off" || b.plan_status === "failed") && loading && (
            <button className="small" disabled={asking} onClick={() => void ask()}>
              ✦ Write an AI game plan
            </button>
          )}
          {askError && <p className="bad small-text">{askError}</p>}

          {brief.likely_opponent && (
            <p className="small-text">
              Likely lane opponent: <b>{brief.likely_opponent.champion}</b>{" "}
              <span className="muted">
                (plays {brief.role?.toLowerCase() ?? "your role"} in {brief.likely_opponent.role_share_pct}%
                of stored games)
              </span>
              {brief.matchup ? (
                <>
                  <br />
                  You vs them in lane: {recordLine(brief.matchup)}
                </>
              ) : (
                <span className="muted"> · no games against them in lane yet</span>
              )}
            </p>
          )}
          {!compact && brief.record.games > 0 && (
            <p className="small-text">
              You on {brief.champion}: {recordLine(brief.record)}
            </p>
          )}
          <div className="brief-enemies">
            {brief.enemies.map((e) => (
              <span key={e.champion_id} className="champ-chip" title={`${e.champion}: your games against`}>
                <ChampionIcon id={e.champion_id} patch={patch} size={compact ? 22 : 26} />
                <span className="small-text mono">{e.games ? `${e.wins}-${e.games - e.wins}` : "new"}</span>
              </span>
            ))}
          </div>
          {brief.reminders.length > 0 && (
            <div className="small-text">
              {brief.reminders.map((r) => (
                <p key={r.id}>
                  <b>Watch: {r.title}.</b> <span className="muted">{r.tip}</span>
                </p>
              ))}
            </div>
          )}
          {!compact && brief.targets.length > 0 && (
            <p className="small-text muted">
              {brief.benchmark_label}:{" "}
              {brief.targets
                .map(
                  (t) =>
                    `${t.label} ${t.benchmark_median.toFixed(t.decimals)}${t.your_average != null ? ` (you ${t.your_average.toFixed(t.decimals)})` : ""}`,
                )
                .join(" · ")}
            </p>
          )}
        </>
      )}
    </div>
  );
}

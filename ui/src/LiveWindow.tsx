// The compact "live" window for a second monitor: the live board and the
// last game's post-game status, nothing else.
import { useEffect, useState } from "react";
import { api } from "./api";
import { BriefCard, LiveBoard, lastGameText, phaseText, useLiveState } from "./components/LiveBoard";
import { ChampionsContext } from "./lib/assets";
import type { ChampionEntry, LiveGameView } from "./types";

export function LiveWindow() {
  const live = useLiveState();
  const [last, setLast] = useState<LiveGameView | null>(null);
  const [champions, setChampions] = useState<Map<number, ChampionEntry>>(new Map());
  const phase = live?.phase;
  useEffect(() => {
    api.champions().then(
      (list) => setChampions(new Map(list.map((c) => [c.key, c]))),
      () => {},
    );
  }, []);
  // Post-game status updates come with engine events; a slow poll is enough here.
  useEffect(() => {
    const load = () =>
      api.liveGames(1).then(
        (g) => setLast(g[0] ?? null),
        () => {},
      );
    void load();
    const id = window.setInterval(load, 10_000);
    return () => window.clearInterval(id);
  }, [phase]);

  const board = live?.snapshot && (live.phase === "in_game" || live.phase === "ended");
  return (
    <ChampionsContext.Provider value={champions}>
      <div className="live-window">
        <div className="live-window-head">
          <span className="brand-mark">⬡</span>
          <span className={`pill live-${phase ?? "idle"}`}>{phaseText(live)}</span>
        </div>
        {live?.brief && (live.phase === "loading" || live.phase === "in_game") && (
          <BriefCard state={live} compact />
        )}
        {board && live ? <LiveBoard state={live} compact /> : <p className="muted">Waiting for a game…</p>}
        {last?.ended_at && live?.phase !== "in_game" && (
          <div className="live-card">
            <div className="live-card-head">
              <span>Last game: {last.champion_name}</span>
              {last.result && (
                <span className={`tag ${last.result === "Win" ? "ok" : "bad"}`}>{last.result}</span>
              )}
            </div>
            <p className="muted small-text">{lastGameText(last)}</p>
          </div>
        )}
      </div>
    </ChampionsContext.Provider>
  );
}

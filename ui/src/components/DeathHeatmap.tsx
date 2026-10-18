// Every death in a window of games on one minimap. Red-side games can be
// mirrored so "your base" is always bottom-left and spots line up.
import { useMemo, useState } from "react";
import { useMinimap } from "../lib/assets";
import { clock } from "../lib/format";
import type { Evidence, MapDeath } from "../types";

const MIN = -120;
const MAX_X = 14870;
const MAX_Y = 14980;
const px = (x: number) => ((x - MIN) / (MAX_X - MIN)) * 100;
const py = (y: number) => (1 - (y - MIN) / (MAX_Y - MIN)) * 100;

export function DeathHeatmap({
  deaths,
  highlight,
  onOpenMatch,
}: {
  deaths: MapDeath[];
  /** Evidence of the selected pattern, drawn on top. */
  highlight: Evidence[];
  onOpenMatch: (id: string) => void;
}) {
  const map = useMinimap("latest");
  const [mirror, setMirror] = useState(true);
  const [only, setOnly] = useState<"all" | "wins" | "losses">("all");
  const [picked, setPicked] = useState<MapDeath | null>(null);

  const side = useMemo(() => new Map(deaths.map((d) => [d.match_id, d.blue_side])), [deaths]);
  const place = (x: number, y: number, blue: boolean) =>
    mirror && !blue ? { x: MIN + MAX_X - x, y: MIN + MAX_Y - y } : { x, y };
  const shown = deaths.filter((d) => only === "all" || (only === "wins") === d.win);
  const marks = highlight.filter((e) => e.x !== null && e.y !== null);

  return (
    <div className="deathmap">
      <div className="map-frame">
        {map ? <img src={map} alt="Summoner's Rift" /> : <div className="map-placeholder">Loading map…</div>}
        <svg viewBox="0 0 100 100" className="map-overlay">
          <defs>
            <radialGradient id="heat">
              <stop offset="0%" stopColor="rgba(255,90,70,0.28)" />
              <stop offset="100%" stopColor="rgba(255,90,70,0)" />
            </radialGradient>
          </defs>
          {shown.map((d, i) => {
            const p = place(d.x, d.y, d.blue_side);
            return <circle key={`h${i}`} cx={px(p.x)} cy={py(p.y)} r={3.5} fill="url(#heat)" />;
          })}
          {shown.map((d, i) => {
            const p = place(d.x, d.y, d.blue_side);
            return (
              <circle
                key={`d${i}`}
                className={`heat-dot ${d.win ? "win" : "loss"} ${picked === d ? "sel" : ""}`}
                cx={px(p.x)}
                cy={py(p.y)}
                r={0.9}
                onClick={() => setPicked(d)}
              />
            );
          })}
          {marks.map((e, i) => {
            const p = place(e.x!, e.y!, side.get(e.match_id) ?? true);
            return <circle key={`e${i}`} className="heat-mark" cx={px(p.x)} cy={py(p.y)} r={1.8} />;
          })}
        </svg>
      </div>
      <div className="form-row">
        <label className="toggle inline">
          <input type="checkbox" checked={mirror} onChange={(e) => setMirror(e.target.checked)} />
          <span className="switch" />
          <span>Show every game from the blue side</span>
        </label>
        <div className="tabs">
          {(["all", "wins", "losses"] as const).map((k) => (
            <button key={k} className={`tab ${only === k ? "on" : ""}`} onClick={() => setOnly(k)}>
              {k === "all" ? "All" : k === "wins" ? "Wins" : "Losses"}
            </button>
          ))}
        </div>
      </div>
      <p className="map-info">
        {picked ? (
          <>
            <b>{clock(picked.t_s)}</b> in {picked.match_id} ({picked.win ? "win" : "loss"}){" "}
            <button className="link" onClick={() => onOpenMatch(picked.match_id)}>
              Open match
            </button>
          </>
        ) : (
          <span className="muted">
            {shown.length} deaths. Positions are exact for kills; gold rings mark the selected pattern's
            examples.
          </span>
        )}
      </p>
    </div>
  );
}

// Summoner's Rift minimap with kills, deaths, objectives and towers, and a
// time scrubber. Riot's timeline samples positions about once per minute,
// but kill/objective events carry exact coordinates.
import { useEffect, useState } from "react";
import { useMinimap } from "../lib/assets";
import { clock } from "../lib/format";
import type { MapEvent, ScoreRow } from "../types";

const MIN = -120;
const MAX_X = 14870;
const MAX_Y = 14980;
const px = (x: number) => ((x - MIN) / (MAX_X - MIN)) * 100;
const py = (y: number) => (1 - (y - MIN) / (MAX_Y - MIN)) * 100;

type Layer = "deaths" | "kills" | "assists" | "others" | "objectives" | "buildings";
const LAYERS: { id: Layer; label: string }[] = [
  { id: "deaths", label: "My deaths" },
  { id: "kills", label: "My kills" },
  { id: "assists", label: "My assists" },
  { id: "others", label: "Other kills" },
  { id: "objectives", label: "Objectives" },
  { id: "buildings", label: "Towers" },
];

interface Props {
  patch: string;
  events: MapEvent[];
  me: number;
  myTeam: number;
  durationS: number;
  players: ScoreRow[];
}

export function DeathMap({ patch, events, me, myTeam, durationS, players }: Props) {
  const map = useMinimap(patch);
  const [t, setT] = useState(durationS);
  const [playing, setPlaying] = useState(false);
  const [on, setOn] = useState<Set<Layer>>(new Set(["deaths", "kills", "assists", "objectives"]));
  const [selected, setSelected] = useState<MapEvent | null>(null);
  const champ = (pid: number | null) =>
    players.find((p) => p.participant_id === pid)?.champion_name ?? "minions/tower";

  useEffect(() => {
    if (!playing) return;
    const id = window.setInterval(() => {
      setT((v) => {
        const next = v + 15;
        if (next >= durationS) {
          setPlaying(false);
          return durationS;
        }
        return next;
      });
    }, 60);
    return () => window.clearInterval(id);
  }, [playing, durationS]);

  const layerOf = (e: MapEvent): Layer => {
    if (e.kind === "objective") return "objectives";
    if (e.kind === "building") return "buildings";
    if (e.victim_id === me) return "deaths";
    if (e.killer_id === me) return "kills";
    if (e.assist_ids.includes(me)) return "assists";
    return "others";
  };
  const visible = events.filter((e) => e.t_s <= t && on.has(layerOf(e)));

  const toggle = (l: Layer) =>
    setOn((s) => {
      const n = new Set(s);
      if (n.has(l)) n.delete(l);
      else n.add(l);
      return n;
    });

  return (
    <div className="deathmap">
      <div className="map-frame">
        {map ? <img src={map} alt="Summoner's Rift" /> : <div className="map-placeholder">Loading map…</div>}
        <svg viewBox="0 0 100 100" className="map-overlay">
          {visible.map((e, i) => (
            <Marker
              key={i}
              e={e}
              layer={layerOf(e)}
              ally={e.team_id === myTeam}
              selected={selected === e}
              onClick={() => setSelected(e)}
            />
          ))}
        </svg>
      </div>
      <div className="map-controls">
        <button
          className="secondary small"
          onClick={() => (t >= durationS ? (setT(0), setPlaying(true)) : setPlaying(!playing))}
        >
          {playing ? "Pause" : "Play"}
        </button>
        <input
          type="range"
          min={0}
          max={Math.ceil(durationS)}
          value={t}
          onChange={(e) => {
            setPlaying(false);
            setT(Number(e.target.value));
          }}
          aria-label="Game time"
        />
        <span className="mono">{clock(t)}</span>
      </div>
      <div className="chips">
        {LAYERS.map((l) => (
          <button
            key={l.id}
            className={`chip layer-${l.id} ${on.has(l.id) ? "on" : ""}`}
            onClick={() => toggle(l.id)}
          >
            <i /> {l.label}
          </button>
        ))}
      </div>
      <p className="map-info">
        {selected ? (
          <>
            <b>{clock(selected.t_s)}</b>{" "}
            {selected.kind === "kill"
              ? `${champ(selected.killer_id)} killed ${champ(selected.victim_id)}${
                  selected.assist_ids.length ? ` (assists: ${selected.assist_ids.map(champ).join(", ")})` : ""
                }${selected.shutdown_bounty ? `, ${selected.shutdown_bounty}g shutdown` : ""}`
              : `${selected.label ?? selected.kind} by ${selected.team_id === myTeam ? "your team" : "enemy team"}`}
          </>
        ) : (
          <span className="muted">
            Click a marker for details. Ward positions aren't in Riot's timeline data.
          </span>
        )}
      </p>
    </div>
  );
}

function Marker({
  e,
  layer,
  ally,
  selected,
  onClick,
}: {
  e: MapEvent;
  layer: Layer;
  ally: boolean;
  selected: boolean;
  onClick: () => void;
}) {
  const x = px(e.x);
  const y = py(e.y);
  const cls = `mk mk-${layer} ${ally ? "ally" : "enemy"} ${selected ? "sel" : ""}`;
  const common = { className: cls, onClick, style: { cursor: "pointer" } };
  switch (layer) {
    case "deaths":
      return (
        <g {...common}>
          <circle cx={x} cy={y} r={2.2} />
          <path
            d={`M${x - 1.1},${y - 1.1}L${x + 1.1},${y + 1.1}M${x + 1.1},${y - 1.1}L${x - 1.1},${y + 1.1}`}
          />
        </g>
      );
    case "objectives":
      return <path {...common} d={`M${x},${y - 2}L${x + 2},${y}L${x},${y + 2}L${x - 2},${y}Z`} />;
    case "buildings":
      return <rect {...common} x={x - 1.3} y={y - 1.3} width={2.6} height={2.6} />;
    default:
      return <circle {...common} cx={x} cy={y} r={layer === "others" ? 0.9 : 1.6} />;
  }
}

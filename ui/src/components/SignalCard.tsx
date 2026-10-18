// One recurring pattern with its numbers, evidence and a goal shortcut.
import { useState } from "react";
import { clock } from "../lib/format";
import type { GoalInput, Signal } from "../types";

export function SignalCard({
  s,
  selected,
  onSelect,
  onOpenMatch,
  onMakeGoal,
}: {
  s: Signal;
  selected?: boolean;
  onSelect?: () => void;
  onOpenMatch?: (id: string) => void;
  onMakeGoal?: (draft: GoalInput) => void;
}) {
  const [open, setOpen] = useState(false);
  const weak = s.polarity === "weakness";
  return (
    <div className={`signal ${weak ? "weak" : "strong"} sev-${s.severity} ${selected ? "on" : ""}`}>
      <div className="signal-head" onClick={onSelect}>
        <span className={`sev-dot`} title={`${s.severity} ${weak ? "priority" : "strength"}`} />
        <b>{s.title}</b>
        <span className="spacer" />
        <span className="tag">
          {s.games_affected}/{s.games_considered} games
        </span>
        {s.win_rate_affected !== null && s.win_rate_other !== null && (
          <span
            className={`tag ${weak ? (s.win_rate_affected < s.win_rate_other ? "warn" : "") : "ok"}`}
            title="Your win rate when this happens vs otherwise"
          >
            WR {s.win_rate_affected}% vs {s.win_rate_other}%
          </span>
        )}
      </div>
      <p>{s.detail}</p>
      <p className="muted small-text tip">{s.tip}</p>
      <div className="form-row">
        {s.evidence.length > 0 && (
          <button className="secondary small" onClick={() => setOpen(!open)}>
            {open ? "Hide examples" : `Examples (${s.evidence.length})`}
          </button>
        )}
        {s.goal && onMakeGoal && (
          <button
            className="secondary small"
            onClick={() =>
              onMakeGoal({
                metric: s.goal!.metric,
                comparator: s.goal!.comparator,
                target: s.goal!.target,
                games: 10,
                role: s.id.split(":")[2] ?? null,
                champion_name: null,
                source: "pattern",
                note: s.title,
              })
            }
          >
            Make this a goal
          </button>
        )}
      </div>
      {open && (
        <ul className="evidence-list">
          {s.evidence.map((e, i) => (
            <li key={i}>
              <button className="link" onClick={() => onOpenMatch?.(e.match_id)}>
                {e.match_id}
              </button>
              {e.t_s !== null && <span className="mono"> {clock(e.t_s)}</span>} <span>{e.note}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

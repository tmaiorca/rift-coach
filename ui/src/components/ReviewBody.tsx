// Renders a validated AI report (post-game or session). Evidence ids are
// shown as tags via `labelOf`.
import type { GoalInput, Report } from "../types";

interface Props {
  report: Report;
  labelOf: (id: string) => string;
  /** Next-game focus that maps to a metric can become a tracked goal. */
  onTrackFocus?: (draft: GoalInput) => void;
  focusLabel?: string;
}

export function ReviewBody({ report: r, labelOf, onTrackFocus, focusLabel = "Next-game focus" }: Props) {
  const o = r.output;
  const evidence = (ids: string[]) =>
    ids.length > 0 && (
      <div className="evidence">
        {ids.map((id) => (
          <span key={id} className="tag">
            {labelOf(id)}
          </span>
        ))}
      </div>
    );
  const f = o.next_game_focus;
  const trackable = f.metric && f.comparator && typeof f.target === "number";

  return (
    <div className="review-body">
      {r.validation !== "ok" && (
        <div className="banner warn">
          Some numbers couldn't be traced to the game data. Treat them with care. {r.notes}
        </div>
      )}
      <p className="summary">{o.summary}</p>
      <div className="review-cols">
        <div>
          <h3 className="good">What went well</h3>
          {o.went_well.map((it, i) => (
            <div key={i} className="review-item">
              <b>{it.title}</b>
              <p>{it.detail}</p>
              {evidence(it.evidence)}
            </div>
          ))}
        </div>
        <div>
          <h3 className="bad">What to improve</h3>
          {o.improve.map((it, i) => (
            <div key={i} className="review-item">
              <b>{it.title}</b>
              <p>{it.detail}</p>
              {evidence(it.evidence)}
            </div>
          ))}
        </div>
      </div>
      <h3>Priority fixes</h3>
      <ol className="fixes">
        {o.priority_fixes.map((fix, i) => (
          <li key={i}>
            <b>{fix.title}</b>
            <p>{fix.why}</p>
            <p className="how">
              <span className="muted">Try:</span> {fix.how}
            </p>
            {evidence(fix.evidence)}
          </li>
        ))}
      </ol>
      {o.goal_feedback && (
        <div className="goal-feedback">
          <span className="muted">Your goals</span>
          <p>{o.goal_feedback}</p>
        </div>
      )}
      <div className="focus">
        <span className="muted">{focusLabel}</span>
        <b>{f.goal}</b>
        {trackable && onTrackFocus && (
          <button
            className="secondary small"
            onClick={() =>
              onTrackFocus({
                metric: f.metric!,
                comparator: f.comparator!,
                target: f.target!,
                games: 10,
                role: null,
                champion_name: null,
                source: "ai",
                note: f.goal,
              })
            }
          >
            Track as goal
          </button>
        )}
      </div>
      <p className="muted small-text">
        Confidence: {o.confidence}
        {o.confidence_reason ? ` (${o.confidence_reason})` : ""}
      </p>
    </div>
  );
}

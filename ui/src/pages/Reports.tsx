import { useEffect, useState } from "react";
import { api } from "../api";
import { dateTime } from "../lib/format";
import type { Report } from "../types";

const KIND: Record<string, string> = { post_game: "game", session: "session" };

export function Reports({
  refreshKey,
  onOpenMatch,
  onOpenSession,
}: {
  refreshKey: number;
  onOpenMatch: (id: string) => void;
  onOpenSession: (scope: string) => void;
}) {
  const [reports, setReports] = useState<Report[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.listReports(500).then(setReports, (e) => setError(String(e)));
  }, [refreshKey]);

  const tokens = (reports ?? []).reduce((s, r) => s + (r.input_tokens ?? 0) + (r.output_tokens ?? 0), 0);
  return (
    <div className="page">
      <section className="panel">
        <div className="panel-head">
          <h2>AI report history</h2>
          <div className="spacer" />
          {reports && reports.length > 0 && (
            <span className="muted">
              {reports.length} reports · {tokens.toLocaleString()} tokens total
            </span>
          )}
        </div>
        <p className="muted">
          Every review is kept, including regenerated ones, so you can compare them over time.
        </p>
        {error && <div className="banner error">{error}</div>}
        {reports?.length === 0 && (
          <p className="muted">
            No reviews yet. Open a match and click "Review this game", or review a session on the Sessions
            page.
          </p>
        )}
        {reports?.map((r) => (
          <details key={r.id} className="report-row">
            <summary>
              <span className="mono">{dateTime(r.created_at)}</span>
              <span className="tag">{KIND[r.kind] ?? r.kind}</span>
              {r.validation !== "ok" && <span className="tag warn">unverified numbers</span>}
              <span className="report-summary">{r.output?.summary}</span>
              <span className="muted small-text">{r.model}</span>
            </summary>
            <div className="report-body">
              {r.output?.priority_fixes?.length > 0 && (
                <>
                  <h3>Priority fixes</h3>
                  <ol>
                    {r.output.priority_fixes.map((f, i) => (
                      <li key={i}>
                        <b>{f.title}</b>: {f.how}
                      </li>
                    ))}
                  </ol>
                </>
              )}
              {r.output?.next_game_focus && (
                <p>
                  <span className="muted">Focus:</span> {r.output.next_game_focus.goal}
                </p>
              )}
              {r.match_id && (
                <button className="secondary small" onClick={() => onOpenMatch(r.match_id!)}>
                  Open match
                </button>
              )}
              {r.scope && r.kind === "session" && (
                <button className="secondary small" onClick={() => onOpenSession(r.scope!)}>
                  Open session
                </button>
              )}
            </div>
          </details>
        ))}
      </section>
    </div>
  );
}

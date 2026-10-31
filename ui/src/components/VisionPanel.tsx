// Vision review of a match: the screenshots taken during the game (if any)
// and the vision model's review of them, written after the game.
import { useEffect, useState } from "react";
import { api } from "../api";
import { dateTime } from "../lib/format";
import type { Report, VisionCategory, VisionReviewOutput, VisionView } from "../types";

const CATEGORY: Record<VisionCategory, string> = {
  positioning: "positioning",
  map_awareness: "map awareness",
  vision: "vision",
  wave_management: "waves",
  trading: "trading",
  objective_setup: "objective setup",
  recall_timing: "recall timing",
  teamfight: "teamfight",
  macro: "macro",
  other: "other",
};

export function momentLabel(id: string): string {
  if (id.startsWith("death_")) return `Death ${id.slice(6)}`;
  if (id.startsWith("objective_")) return "Objective fight";
  if (id.startsWith("checkpoint_")) return `${id.slice(11)}:00 snapshot`;
  return id;
}

export function VisionReport({ report }: { report: Report }) {
  const o = report.output as unknown as VisionReviewOutput;
  return (
    <div className="review-body">
      <p className="summary">{o.summary}</p>
      {o.themes && o.themes.length > 0 && (
        <>
          <h3>Patterns in the screenshots</h3>
          {o.themes.map((t, i) => (
            <div key={i} className="review-item">
              <b>{t.title}</b>
              {t.detail && <p>{t.detail}</p>}
              {t.moments && t.moments.length > 0 && (
                <div className="evidence">
                  {t.moments.map((m) => (
                    <span key={m} className="tag">
                      {momentLabel(m)}
                    </span>
                  ))}
                </div>
              )}
            </div>
          ))}
        </>
      )}
      <h3>Moments</h3>
      <div className="vision-moments">
        {o.moments.map((m) => (
          <div key={m.id} className={`vision-moment sev-${m.severity}`}>
            <div className="vision-moment-head">
              {m.time && <span className="mono">{m.time}</span>}
              <b className="grow">{m.title}</b>
              <span className="tag">{CATEGORY[m.category] ?? m.category}</span>
            </div>
            <div className="muted small-text">
              {momentLabel(m.id)}
              {!m.clear && " · screenshots unclear"}
            </div>
            <p>
              <span className="muted">Seen: </span>
              {m.seen}
            </p>
            <p>
              <span className="muted">Next time: </span>
              {m.lesson}
            </p>
          </div>
        ))}
      </div>
      {o.next_game_focus && (
        <p>
          <span className="muted">Focus for your next game:</span> <b>{o.next_game_focus.goal}</b>
          {o.next_game_focus.how && ` ${o.next_game_focus.how}`}
        </p>
      )}
      <p className="muted small-text">
        Confidence: {o.confidence}
        {o.confidence_reason ? `. ${o.confidence_reason}` : ""}
      </p>
    </div>
  );
}

export function VisionPanel({
  matchId,
  isRift,
  refreshKey,
  onOpenSettings,
}: {
  matchId: string;
  isRift: boolean;
  refreshKey: number;
  onOpenSettings: () => void;
}) {
  const [view, setView] = useState<VisionView | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    api.visionStatus(matchId).then(
      (v) => alive && setView(v),
      () => {},
    );
    return () => {
      alive = false;
    };
  }, [matchId, refreshKey]);

  if (!view) return null;
  const r = view.report;
  // Games not recorded live never had screenshots; say nothing unless reviewed.
  if (!r && view.live_game_id == null) return null;
  if (!r && !isRift) return null;

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const report = await api.generateVisionReview(matchId);
      setView({ ...view, report, frames_on_disk: 0, state: "reviewed" });
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const s = view.stats;

  return (
    <section className="panel review">
      <div className="panel-head">
        <h2>
          <span className="spark">◉</span> Vision review
        </h2>
        <div className="spacer" />
        {r && (
          <span className="muted small-text">
            {r.model} · {dateTime(r.created_at)}
          </span>
        )}
        {!r && view.frames_on_disk > 0 && (
          <button disabled={busy || view.queued} onClick={run}>
            {busy ? "Looking at the screenshots…" : view.queued ? "Review queued…" : "Review screenshots"}
          </button>
        )}
      </div>
      {error && (
        <div className="banner error">
          {error}{" "}
          {/key|model|provider/i.test(error) && (
            <button className="link" onClick={onOpenSettings}>
              Open vision settings
            </button>
          )}
        </div>
      )}
      {!r && view.frames_on_disk > 0 && !busy && (
        <p className="muted">
          {view.frames_on_disk} screenshots from this game are waiting
          {s ? ` (one every ${s.interval_s.toFixed(0)} s, ${s.width}×${s.height})` : ""}. The review sends the
          ones around your deaths, objective fights and laning checkpoints to your vision model, then deletes
          all of them.
          {view.state === "failed" &&
            " The automatic review failed; check the vision settings and try again."}
        </p>
      )}
      {!r && view.frames_on_disk === 0 && (
        <p className="muted">
          {view.state === "deleted"
            ? "This game's screenshots were deleted before a review."
            : view.enabled
              ? "No screenshots were taken in this game."
              : "Turn on Vision review in Settings to get a screenshot review of your next games."}
          {s?.error && ` (${s.error})`}{" "}
          {!view.enabled && (
            <button className="link" onClick={onOpenSettings}>
              Open settings
            </button>
          )}
        </p>
      )}
      {r && <VisionReport report={r} />}
    </section>
  );
}

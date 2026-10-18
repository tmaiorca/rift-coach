import { useEffect, useMemo, useState } from "react";
import { api } from "../api";
import { ReviewBody } from "../components/ReviewBody";
import { ChampionIcon, useChampionName } from "../lib/assets";
import { clock, dateTime, formatMetric, roleLabel } from "../lib/format";
import type { GoalInput, SessionDetail, SessionSummary } from "../types";

export function Sessions({
  refreshKey,
  scope,
  onScope,
  onOpenMatch,
  onOpenSettings,
  onTrackFocus,
}: {
  refreshKey: number;
  scope: string | null;
  onScope: (scope: string | null) => void;
  onOpenMatch: (id: string) => void;
  onOpenSettings: () => void;
  onTrackFocus: (draft: GoalInput) => void;
}) {
  const [list, setList] = useState<SessionSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.listSessions(100).then(setList, (e) => setError(String(e)));
  }, [refreshKey]);

  // Open the newest session by default; drop a scope that no longer exists
  // (a session grows when another game is added to it).
  const current = list?.find((s) => s.scope === scope) ?? list?.[0] ?? null;

  return (
    <div className="page sessions">
      <div className="sessions-grid">
        <section className="panel session-list">
          <h2>Sessions</h2>
          <p className="muted small-text">
            Games less than the session gap apart (Settings) count as one session.
          </p>
          {error && <div className="banner error">{error}</div>}
          {list?.length === 0 && <p className="muted">No games yet.</p>}
          {list?.map((s) => (
            <button
              key={s.scope}
              className={`session-row ${current?.scope === s.scope ? "on" : ""}`}
              onClick={() => onScope(s.scope)}
            >
              <span className="session-when">{dateTime(s.start_ms)}</span>
              <span className="session-icons">
                {s.champion_ids.slice(0, 6).map((c, i) => (
                  <ChampionIcon key={i} id={c} patch="latest" size={20} />
                ))}
              </span>
              <span className="session-score">
                <span className="good">{s.wins}W</span> <span className="bad">{s.losses}L</span>
                {s.has_report && <span className="spark"> ✦</span>}
              </span>
            </button>
          ))}
        </section>
        {current && (
          <SessionView
            key={current.scope}
            scope={current.scope}
            refreshKey={refreshKey}
            onOpenMatch={onOpenMatch}
            onOpenSettings={onOpenSettings}
            onTrackFocus={onTrackFocus}
          />
        )}
      </div>
    </div>
  );
}

function SessionView({
  scope,
  refreshKey,
  onOpenMatch,
  onOpenSettings,
  onTrackFocus,
}: {
  scope: string;
  refreshKey: number;
  onOpenMatch: (id: string) => void;
  onOpenSettings: () => void;
  onTrackFocus: (draft: GoalInput) => void;
}) {
  const [d, setD] = useState<SessionDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [facts, setFacts] = useState<string | null>(null);
  const name = useChampionName();

  useEffect(() => {
    api.sessionDetail(scope).then(
      (x) => (setD(x), setError(null)),
      (e) => setError(String(e)),
    );
  }, [scope, refreshKey]);

  const labels = useMemo(() => {
    const m = new Map<string, string>([["session", "session"]]);
    d?.aggregates.forEach((a) => m.set(a.metric, a.label));
    d?.patterns.forEach((p) => m.set(`pattern:${p.kind}`, p.title));
    d?.games
      .filter((g) => g.is_standard_rift)
      .forEach((g, i) =>
        m.set(`game_${i + 1}`, `game ${i + 1} (${name(g.summary.champion_id, g.summary.champion_name)})`),
      );
    return m;
  }, [d, name]);

  if (error) return <div className="banner error">{error}</div>;
  if (!d) return <div className="muted">Loading session…</div>;
  const s = d.summary;
  const minutes = Math.round(d.games.reduce((a, g) => a + g.summary.duration_s, 0) / 60);

  const run = async (force: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const report = await api.generateSessionReview(scope, force);
      setD({ ...d, report });
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const showFacts = async () => {
    if (facts) return setFacts(null);
    try {
      setFacts(JSON.stringify(await api.sessionFacts(scope), null, 2));
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="stack">
      <section className="panel">
        <div className="panel-head">
          <div>
            <h2>
              {dateTime(s.start_ms)} · {s.games} games
            </h2>
            <div className="muted">
              {s.wins}W {s.losses}L · {minutes} minutes played
            </div>
          </div>
          <div className="spacer" />
          <div className="streak">
            {d.games
              .filter((g) => !g.summary.is_remake)
              .map((g, i) => (
                <span key={i} className={`result ${g.summary.win ? "win" : "loss"}`}>
                  {g.summary.win ? "W" : "L"}
                </span>
              ))}
          </div>
        </div>
        {d.games.map((g, i) => (
          <button key={g.summary.match_id} className="recent" onClick={() => onOpenMatch(g.summary.match_id)}>
            <span className="muted mono">#{i + 1}</span>
            <ChampionIcon id={g.summary.champion_id} patch={g.summary.patch} size={28} />
            <span className={`result ${g.summary.win ? "win" : "loss"}`}>
              {g.summary.is_remake ? "Remake" : g.summary.win ? "Win" : "Loss"}
            </span>
            <span>{name(g.summary.champion_id, g.summary.champion_name)}</span>
            <span className="muted">
              {g.is_standard_rift ? roleLabel(g.summary.team_position) : g.queue_name}
            </span>
            <span className="mono">
              {g.summary.kills}/{g.summary.deaths}/{g.summary.assists}
            </span>
            <span className="muted">{clock(g.summary.duration_s)}</span>
            <span className="flags">
              {g.flags.map((f) => (
                <span key={f.kind} className={`tag ${f.polarity === "weakness" ? "warn" : "ok"}`}>
                  {f.title}
                </span>
              ))}
            </span>
          </button>
        ))}
      </section>

      <section className="panel review">
        <div className="panel-head">
          <h2>
            <span className="spark">✦</span> AI session review
          </h2>
          <div className="spacer" />
          {d.report && (
            <span className="muted small-text">
              {d.report.model} · {dateTime(d.report.created_at)}
            </span>
          )}
          <button className="secondary small" onClick={showFacts}>
            {facts ? "Hide facts" : "Facts sent"}
          </button>
          <button disabled={busy || s.rift_games === 0} onClick={() => run(!!d.report)}>
            {busy ? "Thinking…" : d.report ? "Regenerate" : "Review this session"}
          </button>
        </div>
        {error && (
          <div className="banner error">
            {error}{" "}
            {/key|model|provider|cap/i.test(error) && (
              <button className="link" onClick={onOpenSettings}>
                Open AI settings
              </button>
            )}
          </div>
        )}
        {facts && <pre className="facts">{facts}</pre>}
        {!d.report && !busy && (
          <p className="muted">
            A review of the whole session: how it went compared with your usual games, whether results or play
            dropped as it went on, and one focus for next time.
          </p>
        )}
        {d.report && (
          <ReviewBody
            report={d.report}
            labelOf={(id) => labels.get(id) ?? id}
            onTrackFocus={onTrackFocus}
            focusLabel="Next-session focus"
          />
        )}
      </section>

      <div className="grid-2 even">
        <section className="panel">
          <h2>This session vs your usual</h2>
          <table>
            <thead>
              <tr>
                <th>Metric</th>
                <th className="num">Session</th>
                <th className="num">Usual</th>
              </tr>
            </thead>
            <tbody>
              {d.aggregates.map((a) => (
                <tr key={a.metric}>
                  <td>{a.label}</td>
                  <td className={`num ${a.better === null ? "" : a.better ? "good" : "bad"}`}>
                    {formatMetric(a.session, a.unit, Math.max(a.decimals, 1))}
                  </td>
                  <td className="num muted">
                    {a.baseline === null ? "-" : formatMetric(a.baseline, a.unit, Math.max(a.decimals, 1))}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="muted small-text">
            "Usual" is your average over the {d.baseline_games} games before.
          </p>
        </section>
        <section className="panel">
          <h2>Patterns this session</h2>
          {d.patterns.length === 0 && <p className="muted">None of the tracked patterns showed up.</p>}
          {d.patterns.map((p) => (
            <div key={p.kind} className={`signal ${p.polarity === "weakness" ? "weak" : "strong"}`}>
              <div className="signal-head">
                <b>{p.title}</b>
                <span className="spacer" />
                <span className="tag">
                  {p.session_games_affected}/{p.session_games} games
                </span>
                {p.baseline_rate_pct !== null && <span className="tag">usually {p.baseline_rate_pct}%</span>}
              </div>
              <p className="muted small-text">{p.what}.</p>
            </div>
          ))}
        </section>
      </div>
    </div>
  );
}

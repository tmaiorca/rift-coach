import { useEffect, useState } from "react";
import { api } from "../api";
import { GapsTable } from "../components/Gaps";
import { ago, roleLabel } from "../lib/format";
import type { BenchmarkStatus, GapsView } from "../types";

const PHASE: Record<BenchmarkStatus["collector"]["phase"], string> = {
  starting: "Starting",
  disabled: "Off",
  waiting: "Waiting",
  collecting: "Collecting",
  done: "Up to date",
  nothing_to_do: "Nothing to do yet",
};

export function Benchmarks({
  refreshKey,
  onOpenSettings,
}: {
  refreshKey: number;
  onOpenSettings: () => void;
}) {
  const [status, setStatus] = useState<BenchmarkStatus | null>(null);
  const [role, setRole] = useState<string | null>(null);
  const [champion, setChampion] = useState<string | null>(null);
  const [gaps, setGaps] = useState<GapsView | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const load = () => api.benchmarkStatus().then(setStatus, (e) => setError(String(e)));
    void load();
    const id = window.setInterval(load, 5000);
    return () => window.clearInterval(id);
  }, []);
  // Refetch the comparison as games arrive (every 25 collected).
  const bucket = Math.floor((status?.progress.games_by_patch.reduce((a, [, n]) => a + n, 0) ?? 0) / 25);
  useEffect(() => {
    api.benchmarkGaps(role, champion).then(setGaps, (e) => setError(String(e)));
  }, [role, champion, refreshKey, bucket]);

  const p = status?.progress;
  const c = status?.collector;
  return (
    <div className="page benchmarks">
      <section className="panel">
        <div className="panel-head">
          <h2>Higher-rank benchmarks</h2>
          {c && <span className={`pill bench-${c.phase}`}>{PHASE[c.phase]}</span>}
          <div className="spacer" />
          <button className="secondary small" onClick={onOpenSettings}>
            Settings
          </button>
        </div>
        <p className="muted">
          Rift Coach samples recent solo queue games of {p?.tier_label ?? "higher-ranked"} players in the
          background (never during a game, never while your own games are downloading, and at well under half
          your Riot key's limit), runs the same metrics on them, and compares your numbers with theirs, patch
          by patch. Players who main your champions are sampled first. Other players' names are removed before
          anything is stored.
        </p>
        {error && <div className="banner error">{error}</div>}
        {c && (
          <p>
            {c.detail}
            {c.last_error && <span className="bad"> · {c.last_error}</span>}
            {c.updated_ms > 0 && <span className="muted small-text"> · {ago(c.updated_ms)}</span>}
          </p>
        )}
        {p && (
          <>
            <div className="bench-targets">
              {p.targets.map((t) => (
                <div key={t.label} className="bench-target">
                  <div className="small-text">
                    <b>{t.label}</b> on patch {p.patch}
                  </div>
                  <div className="bar">
                    <div
                      className="bar-fill"
                      style={{ width: `${Math.min(100, (t.samples / t.target) * 100)}%` }}
                    />
                  </div>
                  <div className="muted small-text mono">
                    {t.samples} / {t.target} players
                  </div>
                </div>
              ))}
            </div>
            <p className="muted small-text">
              Ladder sample: {p.collector.players} players · {p.collector.players_checked} checked for your
              champions ({p.collector.focus_players} main them) · {p.collector.players_scanned} scanned ·{" "}
              {p.collector.queued} games queued.{" "}
              {p.games_by_patch.length > 0 &&
                `Stored: ${p.games_by_patch.map(([patch, n]) => `${n} games on ${patch}`).join(", ")}.`}
            </p>
          </>
        )}
      </section>

      <section className="panel">
        <div className="panel-head">
          <h2>You vs {p?.tier_label ?? "them"}</h2>
          <div className="spacer" />
          <select value={role ?? ""} onChange={(e) => setRole(e.target.value || null)} aria-label="Role">
            <option value="">Main role{p?.focus.roles[0] ? ` (${roleLabel(p.focus.roles[0])})` : ""}</option>
            {["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "UTILITY"].map((r) => (
              <option key={r} value={r}>
                {roleLabel(r)}
              </option>
            ))}
          </select>
          <select
            value={champion ?? ""}
            onChange={(e) => setChampion(e.target.value || null)}
            aria-label="Champion"
          >
            <option value="">Any champion</option>
            {p?.focus.champions.map((ch) => (
              <option key={ch} value={ch}>
                {ch}
              </option>
            ))}
          </select>
        </div>
        {gaps ? <GapsTable view={gaps} /> : <p className="muted">No Summoner's Rift games yet.</p>}
      </section>
    </div>
  );
}

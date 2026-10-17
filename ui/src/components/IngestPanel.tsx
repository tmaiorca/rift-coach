import { useState } from "react";
import type { BucketSnapshot, StatusView } from "../types";

const PLATFORMS = [
  "na1",
  "euw1",
  "eun1",
  "kr",
  "br1",
  "la1",
  "la2",
  "oc1",
  "jp1",
  "tr1",
  "ru",
  "me1",
  "sg2",
  "tw2",
  "vn2",
];

interface Props {
  status: StatusView | null;
  log: string[];
  onStart: (riotId: string, platform: string, count: number) => Promise<void>;
}

export function IngestPanel({ status, log, onStart }: Props) {
  const account = status?.account;
  const [riotId, setRiotId] = useState("");
  const [platform, setPlatform] = useState("na1");
  const [count, setCount] = useState(100);
  const [error, setError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);

  const effectiveId = riotId || (account ? `${account.game_name}#${account.tag_line}` : "");
  const jobs = status?.engine.jobs;
  const pending = jobs ? jobs.ready + jobs.scheduled + jobs.running : 0;
  const appBuckets = (status?.engine.rate_limits ?? []).filter((b) => b.scope === "app");

  async function start() {
    setError(null);
    setStarting(true);
    try {
      await onStart(effectiveId, account && !riotId ? account.platform : platform, count);
    } catch (e) {
      setError(String(e));
    } finally {
      setStarting(false);
    }
  }

  return (
    <section className="panel ingest">
      <h2>Ingest</h2>
      <div className="form-row">
        <input
          placeholder="Riot ID, e.g. TapRooted#BTC"
          value={riotId}
          onChange={(e) => setRiotId(e.target.value)}
          aria-label="Riot ID"
        />
        <select value={platform} onChange={(e) => setPlatform(e.target.value)} aria-label="Platform">
          {PLATFORMS.map((p) => (
            <option key={p} value={p}>
              {p.toUpperCase()}
            </option>
          ))}
        </select>
        <input
          type="number"
          min={1}
          max={1000}
          value={count}
          onChange={(e) => setCount(Number(e.target.value))}
          aria-label="Number of matches"
          className="count"
        />
        <button disabled={!effectiveId.includes("#") || starting} onClick={start}>
          {account && !riotId ? `Sync ${account.game_name}` : "Ingest"}
        </button>
      </div>
      {error && <div className="banner error">{error}</div>}

      <div className="stats">
        <Stat label="Matches" value={status?.counts?.matches ?? 0} />
        <Stat label="Timelines" value={status?.counts?.with_timeline ?? 0} />
        <Stat label="Jobs pending" value={pending} />
        <Stat label="Failed" value={jobs?.failed ?? 0} tone={jobs?.failed ? "bad" : undefined} />
      </div>

      {appBuckets.length > 0 && (
        <div className="limits">
          <h3>Rate limits (app)</h3>
          {appBuckets.map((b) => (
            <LimitRow key={b.routing} bucket={b} />
          ))}
        </div>
      )}

      <div className="log">
        <h3>Activity</h3>
        {log.length === 0 ? <p className="muted">Nothing yet.</p> : log.map((l, i) => <div key={i}>{l}</div>)}
      </div>

      {(status?.engine.recent_failures.length ?? 0) > 0 && (
        <details className="failures">
          <summary>Recent failures</summary>
          {status?.engine.recent_failures.map((f) => (
            <div key={f.dedupe_key}>
              <code>{f.dedupe_key}</code> {f.last_error}
            </div>
          ))}
        </details>
      )}
      <p className="muted small">Data: {status?.data_dir}</p>
    </section>
  );
}

function Stat({ label, value, tone }: { label: string; value: number; tone?: "bad" }) {
  return (
    <div className={`stat ${tone ?? ""}`}>
      <div className="stat-value">{value}</div>
      <div className="stat-label">{label}</div>
    </div>
  );
}

function LimitRow({ bucket }: { bucket: BucketSnapshot }) {
  return (
    <div className="limit-row">
      <span className="routing">{bucket.routing}</span>
      {bucket.windows.map((w) => {
        const pct = Math.min(100, (w.used / w.limit) * 100);
        return (
          <span key={w.period_s} className="meter" title={`${w.used} of ${w.limit} per ${w.period_s}s`}>
            <span className="meter-fill" style={{ width: `${pct}%` }} />
            <span className="meter-text">
              {w.used}/{w.limit} per {w.period_s}s
            </span>
          </span>
        );
      })}
      {bucket.blocked_for_ms > 0 && (
        <span className="pill key-rejected">paused {Math.ceil(bucket.blocked_for_ms / 1000)}s</span>
      )}
    </div>
  );
}

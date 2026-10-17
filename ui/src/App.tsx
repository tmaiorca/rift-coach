import { useCallback, useEffect, useRef, useState } from "react";
import { api, onEngineEvent } from "./api";
import { IngestPanel } from "./components/IngestPanel";
import { MatchTable } from "./components/MatchTable";
import { StatusBar } from "./components/StatusBar";
import type { EngineEvent, MatchRow, StatusView } from "./types";

const FAST_POLL_MS = 1500;
const SLOW_POLL_MS = 10000;
const LOG_LINES = 8;

export function App() {
  const [status, setStatus] = useState<StatusView | null>(null);
  const [matches, setMatches] = useState<MatchRow[]>([]);
  const [log, setLog] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const refreshTimer = useRef<number | undefined>(undefined);

  const refreshStatus = useCallback(async () => {
    try {
      setStatus(await api.getStatus());
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const refreshMatches = useCallback(async () => {
    try {
      setMatches(await api.listMatches(200));
    } catch (e) {
      setError(String(e));
    }
  }, []);

  // Coalesce bursts of "stored" events into one list refresh.
  const scheduleMatchRefresh = useCallback(() => {
    window.clearTimeout(refreshTimer.current);
    refreshTimer.current = window.setTimeout(refreshMatches, 750);
  }, [refreshMatches]);

  useEffect(() => {
    api.getStatus().then(setStatus, (e) => setError(String(e)));
    api.listMatches(200).then(setMatches, (e) => setError(String(e)));
    const unlisten = onEngineEvent((ev: EngineEvent) => {
      const line = describe(ev);
      if (line) setLog((l) => [line, ...l].slice(0, LOG_LINES));
      if (ev.type === "match_stored" || ev.type === "timeline_stored") scheduleMatchRefresh();
      if (ev.type === "key_status_changed") void refreshStatus();
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, [refreshStatus, refreshMatches, scheduleMatchRefresh]);

  const busy = (status?.engine.jobs.ready ?? 0) + (status?.engine.jobs.running ?? 0) > 0;
  useEffect(() => {
    const id = window.setInterval(refreshStatus, busy ? FAST_POLL_MS : SLOW_POLL_MS);
    return () => window.clearInterval(id);
  }, [busy, refreshStatus]);

  return (
    <div className="app">
      <StatusBar status={status} onReloadKey={async () => (await api.reloadKey(), refreshStatus())} />
      {error && <div className="banner error">{error}</div>}
      <main>
        <IngestPanel
          status={status}
          log={log}
          onStart={async (riotId, platform, count) => {
            await api.startIngest(riotId, platform, count);
            await refreshStatus();
          }}
        />
        <MatchTable matches={matches} />
      </main>
    </div>
  );
}

function describe(ev: EngineEvent): string | null {
  switch (ev.type) {
    case "match_stored":
      return `Stored ${ev.match_id}`;
    case "timeline_stored":
      return `Timeline ${ev.match_id}`;
    case "job_retrying":
      return `Retry in ${ev.retry_in_s}s: ${ev.label} (${ev.error})`;
    case "job_failed":
      return `Failed: ${ev.label} (${ev.error})`;
    case "key_status_changed":
      return `API key: ${ev.status}`;
    case "key_generation_changed":
      return `API key changed; re-resolving accounts`;
    default:
      return null;
  }
}

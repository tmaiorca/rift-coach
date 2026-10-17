import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, inTauri, onEngineEvent } from "./api";
import { AmbientBackground } from "./components/AmbientBackground";
import { StatusBar } from "./components/StatusBar";
import { ChampionsContext } from "./lib/assets";
import { Dashboard } from "./pages/Dashboard";
import { Guide } from "./pages/Guide";
import { MatchDetail } from "./pages/MatchDetail";
import { Matches } from "./pages/Matches";
import { Players } from "./pages/Players";
import { Reports } from "./pages/Reports";
import { Settings } from "./pages/Settings";
import type {
  AccountView,
  ActivityEntry,
  AppSettings,
  ChampionEntry,
  EngineEvent,
  MatchRow,
  StatusView,
} from "./types";

type Page = "dashboard" | "matches" | "match" | "players" | "reports" | "settings" | "guide";

const NAV: { id: Page; label: string; icon: string }[] = [
  { id: "dashboard", label: "Dashboard", icon: "◈" },
  { id: "matches", label: "Matches", icon: "☰" },
  { id: "players", label: "Players", icon: "⚇" },
  { id: "reports", label: "AI reports", icon: "✦" },
  { id: "settings", label: "Settings", icon: "⚙" },
  { id: "guide", label: "Guide", icon: "?" },
];

const FAST_POLL_MS = 1500;
const SLOW_POLL_MS = 10000;

/** What's restored on the next launch (stored in the database). */
interface UiState {
  page: Page;
  matchId: string | null;
}

export function App() {
  if (!inTauri) return <BrowserNotice />;
  return <Shell />;
}

function Shell() {
  const [status, setStatus] = useState<StatusView | null>(null);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [accounts, setAccounts] = useState<AccountView[]>([]);
  const [matches, setMatches] = useState<MatchRow[]>([]);
  const [activity, setActivity] = useState<ActivityEntry[]>([]);
  const [champions, setChampions] = useState<Map<number, ChampionEntry>>(new Map());
  const [ui, setUi] = useState<UiState | null>(null);
  const [lookup, setLookup] = useState<string | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const refreshTimer = useRef<number | undefined>(undefined);
  const mainRef = useRef<HTMLElement>(null);

  const refreshStatus = useCallback(() => api.getStatus().then(setStatus, (e) => setError(String(e))), []);
  const refreshData = useCallback(() => {
    api.listMatches(1000).then(setMatches, (e) => setError(String(e)));
    api.recentActivity(100).then(setActivity, () => {});
    api.listAccounts().then(setAccounts, () => {});
    setRefreshKey((k) => k + 1);
  }, []);

  // Coalesce bursts of engine events into one refresh.
  const scheduleRefresh = useCallback(() => {
    window.clearTimeout(refreshTimer.current);
    refreshTimer.current = window.setTimeout(refreshData, 1000);
  }, [refreshData]);

  useEffect(() => {
    api.getStatus().then(setStatus, (e) => setError(String(e)));
    api.getSettings().then(setSettings, (e) => setError(String(e)));
    api.getUiState().then(
      (s) => setUi({ page: (s?.page as Page) ?? "dashboard", matchId: (s?.matchId as string) ?? null }),
      () => setUi({ page: "dashboard", matchId: null }),
    );
    api.listMatches(1000).then(setMatches, (e) => setError(String(e)));
    api.recentActivity(100).then(setActivity, () => {});
    api.listAccounts().then(setAccounts, () => {});
    api.champions().then(
      (list) => setChampions(new Map(list.map((c) => [c.key, c]))),
      () => {},
    );
    const unlisten = onEngineEvent((ev: EngineEvent) => {
      if (ev.type === "key_status_changed") void refreshStatus();
      if (
        ["match_stored", "timeline_stored", "metrics_computed", "review_ready", "job_failed"].includes(
          ev.type,
        )
      )
        scheduleRefresh();
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, [refreshStatus, scheduleRefresh]);

  // Persist navigation so a restart reopens where you left off; each page
  // starts at the top.
  useEffect(() => {
    if (ui) void api.saveUiState({ ...ui }).catch(() => {});
    mainRef.current?.scrollTo(0, 0);
  }, [ui]);

  const jobs = status?.engine.jobs;
  const busy = (jobs?.ready ?? 0) + (jobs?.running ?? 0) > 0;
  useEffect(() => {
    const id = window.setInterval(refreshStatus, busy ? FAST_POLL_MS : SLOW_POLL_MS);
    return () => window.clearInterval(id);
  }, [busy, refreshStatus]);

  const go = (page: Page, matchId: string | null = ui?.matchId ?? null) => setUi({ page, matchId });
  const openMatch = (id: string) => setUi({ page: "match", matchId: id });

  const saveSettings = async (s: AppSettings) => {
    setSettings(s); // optimistic, so sliders feel instant
    try {
      setSettings(await api.saveSettings(s));
    } catch (e) {
      setSettings(await api.getSettings());
      throw e;
    }
  };

  const switchAccount = async (id: number) => {
    await api.setActiveAccount(id);
    await refreshStatus();
    refreshData();
    setUi((u) => (u?.page === "match" ? { page: "matches", matchId: null } : u));
  };

  const sync = async () => {
    const a = status?.account;
    if (!a || !settings) return;
    await api.startIngest(`${a.game_name}#${a.tag_line}`, a.platform, settings.ingest_count);
    await refreshStatus();
    scheduleRefresh();
  };

  // Background champion: explicit choice → open match → most played.
  const bgChampion = useMemo(() => {
    const chosen = settings?.appearance.background_champion;
    if (chosen) return chosen;
    if (ui?.page === "match" && ui.matchId) {
      const m = matches.find((x) => x.match_id === ui.matchId);
      if (m) return String(m.champion_id);
    }
    const counts = new Map<number, number>();
    for (const m of matches.slice(0, 50))
      if (m.is_standard_rift) counts.set(m.champion_id, (counts.get(m.champion_id) ?? 0) + 1);
    const top = [...counts.entries()].sort((a, b) => b[1] - a[1])[0];
    return top ? String(top[0]) : null;
  }, [settings, ui, matches]);

  if (!ui || !settings) return <div className="boot">Rift Coach</div>;
  const page = ui.page === "match" && !ui.matchId ? "matches" : ui.page;
  const navActive = page === "match" ? "matches" : page;

  return (
    <ChampionsContext.Provider value={champions}>
      <AmbientBackground
        champion={bgChampion}
        enabled={settings.appearance.animated_background}
        particles={settings.appearance.particles}
        intensity={settings.appearance.background_intensity}
      />
      <div className="app">
        <nav className="sidebar">
          <div className="brand">
            <span className="brand-mark">⬡</span> Rift Coach
          </div>
          {NAV.map((n) => (
            <button key={n.id} className={`nav ${navActive === n.id ? "on" : ""}`} onClick={() => go(n.id)}>
              <span className="nav-icon">{n.icon}</span>
              {n.label}
            </button>
          ))}
          <div className="spacer" />
          <div className="muted small-text sidebar-foot">
            {busy
              ? `Syncing… ${(jobs?.ready ?? 0) + (jobs?.running ?? 0) + (jobs?.scheduled ?? 0)} jobs`
              : "Idle"}
          </div>
        </nav>
        <div className="main">
          <StatusBar
            status={status}
            accounts={accounts}
            onSwitch={switchAccount}
            onReloadKey={async () => (await api.reloadKey(), refreshStatus())}
            onOpenSettings={() => go("settings")}
          />
          {error && (
            <div className="banner error" onClick={() => setError(null)}>
              {error}
            </div>
          )}
          <main ref={mainRef}>
            {page === "dashboard" && (
              <Dashboard
                status={status}
                activity={activity}
                refreshKey={refreshKey}
                ingestCount={settings.ingest_count}
                onOpenMatch={openMatch}
                onGoPlayers={() => go("players")}
                onSync={sync}
              />
            )}
            {page === "matches" && <Matches matches={matches} onOpen={openMatch} />}
            {page === "match" && ui.matchId && (
              <MatchDetail
                key={ui.matchId}
                matchId={ui.matchId}
                refreshKey={refreshKey}
                onBack={() => go("matches", null)}
                onOpenSettings={() => go("settings")}
                onLookup={(id) => {
                  setLookup(id);
                  go("players");
                }}
              />
            )}
            {page === "players" && (
              <Players
                defaultCount={settings.ingest_count}
                prefill={lookup}
                refreshKey={refreshKey}
                onChanged={() => {
                  void refreshStatus();
                  refreshData();
                }}
                onView={() => go("dashboard")}
              />
            )}
            {page === "reports" && <Reports refreshKey={refreshKey} onOpenMatch={openMatch} />}
            {page === "settings" && (
              <Settings
                settings={settings}
                status={status}
                onSave={saveSettings}
                onKeyChanged={() => void refreshStatus()}
              />
            )}
            {page === "guide" && <Guide />}
          </main>
        </div>
      </div>
    </ChampionsContext.Provider>
  );
}

function BrowserNotice() {
  return (
    <div className="boot notice">
      <h1>Rift Coach runs as a desktop app</h1>
      <p>
        This address (localhost:1420) is the app's internal dev server, so it can't reach your data from a
        browser tab. Use the Rift Coach window that opened with <code>npm run tauri dev</code> (or the
        installed app). Your data and history live in %LOCALAPPDATA%\RiftCoach and appear there every time you
        open it.
      </p>
    </div>
  );
}

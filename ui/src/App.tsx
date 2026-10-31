import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, inTauri, onEngineEvent, onLiveUpdate } from "./api";
import { AmbientBackground } from "./components/AmbientBackground";
import { Setup } from "./components/Setup";
import { StatusBar } from "./components/StatusBar";
import { ChampionsContext } from "./lib/assets";
import { Dashboard } from "./pages/Dashboard";
import { Goals } from "./pages/Goals";
import { Guide } from "./pages/Guide";
import { Live } from "./pages/Live";
import { MatchDetail } from "./pages/MatchDetail";
import { Matches } from "./pages/Matches";
import { Patterns } from "./pages/Patterns";
import { Benchmarks } from "./pages/Benchmarks";
import { Champions } from "./pages/Champions";
import { Players } from "./pages/Players";
import { Weekly } from "./pages/Weekly";
import { Reports } from "./pages/Reports";
import { Sessions } from "./pages/Sessions";
import { Settings } from "./pages/Settings";
import type {
  AccountView,
  ActivityEntry,
  AppSettings,
  ChampionEntry,
  EngineEvent,
  LivePhase,
  GoalInput,
  MatchRow,
  StatusView,
  UpdateInfo,
} from "./types";

type Page =
  | "dashboard"
  | "live"
  | "matches"
  | "match"
  | "champions"
  | "patterns"
  | "sessions"
  | "weekly"
  | "benchmarks"
  | "goals"
  | "players"
  | "reports"
  | "settings"
  | "guide";

const NAV: { id: Page; label: string; icon: string }[] = [
  { id: "dashboard", label: "Dashboard", icon: "◈" },
  { id: "live", label: "Live", icon: "◉" },
  { id: "matches", label: "Matches", icon: "☰" },
  { id: "champions", label: "Champions", icon: "♛" },
  { id: "patterns", label: "Patterns", icon: "⌖" },
  { id: "sessions", label: "Sessions", icon: "◷" },
  { id: "weekly", label: "Weekly", icon: "▦" },
  { id: "benchmarks", label: "Benchmarks", icon: "▲" },
  { id: "goals", label: "Goals", icon: "◎" },
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
  sessionScope?: string | null;
  week?: string | null;
  champion?: string | null;
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
  const [goalDraft, setGoalDraft] = useState<GoalInput | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [livePhase, setLivePhase] = useState<LivePhase>("idle");
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [updating, setUpdating] = useState(false);
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
    api.getSettings().then(
      (s) => {
        setSettings(s);
        // Quietly: no release yet, offline, etc. aren't worth a banner.
        if (s.desktop.check_updates) api.checkUpdate().then(setUpdate, () => {});
      },
      (e) => setError(String(e)),
    );
    api.getUiState().then(
      (s) =>
        setUi({
          page: (s?.page as Page) ?? "dashboard",
          matchId: (s?.matchId as string) ?? null,
          sessionScope: (s?.sessionScope as string) ?? null,
          week: (s?.week as string) ?? null,
          champion: (s?.champion as string) ?? null,
        }),
      () => setUi({ page: "dashboard", matchId: null }),
    );
    api.listMatches(1000).then(setMatches, (e) => setError(String(e)));
    api.recentActivity(100).then(setActivity, () => {});
    api.listAccounts().then(setAccounts, () => {});
    api.champions().then(
      (list) => setChampions(new Map(list.map((c) => [c.key, c]))),
      () => {},
    );
    // Only the phase: the board itself re-renders on its own page.
    api.liveState().then(
      (s) => setLivePhase(s.phase),
      () => {},
    );
    const unlistenLive = onLiveUpdate((s) => setLivePhase(s.phase));
    const unlisten = onEngineEvent((ev: EngineEvent) => {
      if (ev.type === "key_status_changed" || ev.type === "in_game_changed") void refreshStatus();
      // Collected benchmark games don't change anything on screen.
      if (ev.type === "metrics_computed" && ev.benchmark) return;
      if (
        [
          "match_stored",
          "timeline_stored",
          "metrics_computed",
          "review_ready",
          "vision_review_ready",
          "session_review_ready",
          "weekly_report_ready",
          "live_game_linked",
          "post_game_ready",
          "job_failed",
        ].includes(ev.type)
      )
        scheduleRefresh();
    });
    return () => {
      void unlisten.then((f) => f());
      void unlistenLive.then((f) => f());
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

  const go = (page: Page, matchId: string | null = ui?.matchId ?? null) =>
    setUi((u) => ({ ...u, page, matchId }));
  const openMatch = (id: string) => setUi((u) => ({ ...u, page: "match", matchId: id }));
  const openSession = (scope: string | null) =>
    setUi((u) => ({ ...u, page: "sessions", matchId: u?.matchId ?? null, sessionScope: scope }));
  const openWeek = (week: string | null) =>
    setUi((u) => ({ ...u, page: "weekly", matchId: u?.matchId ?? null, week }));
  const openChampion = (champion: string | null) =>
    setUi((u) => ({ ...u, page: "champions", matchId: u?.matchId ?? null, champion }));
  const draftGoal = (draft: GoalInput) => {
    setGoalDraft(draft);
    go("goals");
  };

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

  const mostPlayed = useMemo(() => {
    const counts = new Map<number, number>();
    for (const m of matches.slice(0, 50))
      if (m.is_standard_rift) counts.set(m.champion_id, (counts.get(m.champion_id) ?? 0) + 1);
    return [...counts.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? null;
  }, [matches]);

  // Background champion: explicit choice → open match → most played.
  const bgChampion = useMemo(() => {
    const chosen = settings?.appearance.background_champion;
    if (chosen) return chosen;
    if (ui?.page === "match" && ui.matchId) {
      const m = matches.find((x) => x.match_id === ui.matchId);
      if (m) return String(m.champion_id);
    }
    return mostPlayed != null ? String(mostPlayed) : null;
  }, [settings, ui, matches, mostPlayed]);
  const playing = livePhase === "loading" || livePhase === "in_game";

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
        still={playing}
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
              {n.id === "live" && playing && <span className="live-dot" title="Game running" />}
            </button>
          ))}
          <div className="spacer" />
          <div className="muted small-text sidebar-foot">
            {status?.engine.in_game
              ? "Game running: work paused"
              : busy
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
          {update && (
            <div className="banner update">
              Rift Coach {update.version} is available (you have {update.current_version}).{" "}
              <button
                className="small"
                disabled={updating}
                onClick={() => {
                  setUpdating(true);
                  api.installUpdate().catch((e) => (setError(String(e)), setUpdating(false)));
                }}
              >
                {updating ? "Installing…" : "Install and restart"}
              </button>{" "}
              <button className="link" onClick={() => setUpdate(null)}>
                Later
              </button>
            </div>
          )}
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
                onGoPatterns={() => go("patterns")}
                onGoGoals={() => go("goals")}
                onSync={sync}
              />
            )}
            {page === "live" && (
              <Live
                status={status}
                refreshKey={refreshKey}
                defaultChampion={mostPlayed != null ? (champions.get(mostPlayed)?.id ?? null) : null}
                onOpenMatch={openMatch}
                onOpenSettings={() => go("settings")}
              />
            )}
            {page === "patterns" && (
              <Patterns
                refreshKey={refreshKey}
                baselineGames={settings.baseline_games}
                onOpenMatch={openMatch}
                onMakeGoal={draftGoal}
              />
            )}
            {page === "sessions" && (
              <Sessions
                refreshKey={refreshKey}
                scope={ui.sessionScope ?? null}
                onScope={openSession}
                onOpenMatch={openMatch}
                onOpenSettings={() => go("settings")}
                onTrackFocus={draftGoal}
              />
            )}
            {page === "weekly" && (
              <Weekly
                refreshKey={refreshKey}
                week={ui.week ?? null}
                onWeek={openWeek}
                onOpenSettings={() => go("settings")}
                onTrackFocus={draftGoal}
              />
            )}
            {page === "champions" && (
              <Champions
                refreshKey={refreshKey}
                champion={ui.champion ?? null}
                onChampion={openChampion}
                onOpenMatch={openMatch}
                onMakeGoal={draftGoal}
              />
            )}
            {page === "benchmarks" && (
              <Benchmarks refreshKey={refreshKey} onOpenSettings={() => go("settings")} />
            )}
            {page === "goals" && (
              <Goals
                refreshKey={refreshKey}
                draft={goalDraft}
                onDraftUsed={() => setGoalDraft(null)}
                onOpenMatch={openMatch}
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
                onTrackFocus={draftGoal}
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
            {page === "reports" && (
              <Reports
                refreshKey={refreshKey}
                onOpenMatch={openMatch}
                onOpenSession={openSession}
                onOpenWeek={openWeek}
              />
            )}
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
      {!settings.desktop.onboarded && (
        <Setup
          settings={settings}
          status={status}
          onSave={saveSettings}
          onKeyChanged={() => void refreshStatus()}
          onAccountAdded={() => {
            void refreshStatus();
            refreshData();
          }}
          onDone={() => go("dashboard")}
        />
      )}
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

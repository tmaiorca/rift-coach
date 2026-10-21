import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AccountView,
  ActivityEntry,
  AiUsage,
  AppSettings,
  ChampionEntry,
  DashboardView,
  EngineEvent,
  Goal,
  GoalInput,
  KeyStatus,
  LiveGameView,
  LiveState,
  BenchmarkStatus,
  GapsView,
  Rank,
  WeekDetail,
  WeekSummary,
  BackupInfo,
  BuildPlanView,
  McpStatus,
  UpdateInfo,
  ChampionDetail,
  ChampionSummary,
  PlannerItem,
  MatchDetail,
  MatchRow,
  MetricDef,
  PatternsView,
  ProviderInfo,
  Report,
  SessionDetail,
  SessionSummary,
  StatusView,
} from "./types";

/** True inside the Rift Coach window; false in a plain browser tab. */
export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const api = {
  getStatus: () => invoke<StatusView>("get_status"),
  recentActivity: (limit = 100) => invoke<ActivityEntry[]>("recent_activity", { limit }),

  startIngest: (riotId: string, platform: string, count: number) =>
    invoke<number>("start_ingest", { riotId, platform, count }),
  listAccounts: () => invoke<AccountView[]>("list_accounts"),
  setActiveAccount: (accountId: number) => invoke<void>("set_active_account", { accountId }),
  setPrimaryAccount: (accountId: number) => invoke<void>("set_primary_account", { accountId }),
  removeAccount: (accountId: number) => invoke<void>("remove_account", { accountId }),

  listMatches: (limit: number, offset = 0) => invoke<MatchRow[]>("list_matches", { limit, offset }),
  matchDetail: (matchId: string) => invoke<MatchDetail>("match_detail", { matchId }),
  dashboard: () => invoke<DashboardView>("dashboard"),
  metricDefinitions: () => invoke<MetricDef[]>("metric_definitions"),

  patterns: (window: number | null = null, role: string | null = null) =>
    invoke<PatternsView>("patterns", { window, role }),
  listGoals: (includeEnded = false) => invoke<Goal[]>("list_goals", { includeEnded }),
  createGoal: (input: GoalInput) => invoke<Goal>("create_goal", { input }),
  setGoalStatus: (goalId: number, status: Goal["status"]) =>
    invoke<void>("set_goal_status", { goalId, status }),
  deleteGoal: (goalId: number) => invoke<void>("delete_goal", { goalId }),
  listSessions: (limit = 100) => invoke<SessionSummary[]>("list_sessions", { limit }),
  sessionDetail: (scope: string) => invoke<SessionDetail>("session_detail", { scope }),
  generateSessionReview: (scope: string, force: boolean) =>
    invoke<Report>("generate_session_review", { scope, force }),
  sessionFacts: (scope: string) => invoke<unknown>("session_facts", { scope }),
  aiUsage: () => invoke<AiUsage>("ai_usage"),

  generateReview: (matchId: string, force: boolean) => invoke<Report>("generate_review", { matchId, force }),
  reviewFacts: (matchId: string) => invoke<unknown>("review_facts", { matchId }),
  listReports: (limit = 200) => invoke<Report[]>("list_reports", { limit }),
  aiProviders: () => invoke<ProviderInfo[]>("ai_providers"),
  setAiKey: (provider: string, key: string) => invoke<void>("set_ai_key", { provider, key }),
  deleteAiKey: (provider: string) => invoke<void>("delete_ai_key", { provider }),
  listAiModels: (provider: string, baseUrl: string | null) =>
    invoke<string[]>("list_ai_models", { provider, baseUrl }),
  testAi: (provider: string, model: string, baseUrl: string | null) =>
    invoke<string>("test_ai", { provider, model, baseUrl }),

  getSettings: () => invoke<AppSettings>("get_settings"),
  saveSettings: (s: AppSettings) => invoke<AppSettings>("save_settings", { new: s }),
  getUiState: () => invoke<Record<string, unknown> | null>("get_ui_state"),
  saveUiState: (ui: Record<string, unknown>) => invoke<void>("save_ui_state", { ui }),

  reloadKey: () => invoke<KeyStatus>("reload_key"),
  setRiotKey: (key: string) => invoke<KeyStatus>("set_riot_key", { key }),
  deleteRiotKey: () => invoke<KeyStatus>("delete_riot_key"),

  champions: () => invoke<ChampionEntry[]>("champions"),
  championIcon: (championId: number, patch: string) => invoke<string>("champion_icon", { championId, patch }),
  itemIcon: (itemId: number, patch: string) => invoke<string>("item_icon", { itemId, patch }),
  spellIcon: (spellId: number, patch: string) => invoke<string>("spell_icon", { spellId, patch }),
  championSplash: (champion: string) => invoke<string>("champion_splash", { champion }),
  minimap: (patch: string) => invoke<string>("minimap", { patch }),

  liveState: () => invoke<LiveState>("live_state"),
  liveGames: (limit = 20) => invoke<LiveGameView[]>("live_games", { limit }),
  plannerItems: () => invoke<{ patch: string; items: PlannerItem[] }>("planner_items"),
  buildPlan: (champion: string) => invoke<BuildPlanView>("build_plan", { champion }),
  setBuildPlan: (champion: string, items: number[]) =>
    invoke<BuildPlanView>("set_build_plan", { champion, items }),
  openLiveWindow: () => invoke<void>("open_live_window"),
  generateGamePlan: () => invoke<void>("generate_game_plan"),

  benchmarkStatus: () => invoke<BenchmarkStatus>("benchmark_status"),
  benchmarkGaps: (role: string | null = null, champion: string | null = null) =>
    invoke<GapsView | null>("benchmark_gaps", { role, champion }),
  ranks: () => invoke<Rank[]>("ranks"),
  listWeeks: (limit = 52) => invoke<WeekSummary[]>("list_weeks", { limit }),
  weekDetail: (week: string) => invoke<WeekDetail>("week_detail", { week }),
  generateWeeklyReport: (week: string, force: boolean) =>
    invoke<Report>("generate_weekly_report", { week, force }),
  weeklyFacts: (week: string) => invoke<unknown>("weekly_facts", { week }),

  championList: () => invoke<ChampionSummary[]>("champion_list"),
  championDetail: (champion: string) => invoke<ChampionDetail>("champion_detail", { champion }),

  autostartEnabled: () => invoke<boolean>("autostart_enabled"),
  setAutostart: (enabled: boolean) => invoke<boolean>("set_autostart", { enabled }),
  mcpStatus: () => invoke<McpStatus>("mcp_status"),
  mcpToken: () => invoke<string>("mcp_token"),
  mcpNewToken: () => invoke<void>("mcp_new_token"),
  backupDatabase: () => invoke<string | null>("backup_database"),
  exportGamesCsv: () => invoke<string | null>("export_games_csv"),
  restoreDatabase: () => invoke<BackupInfo | null>("restore_database"),
  deleteAllData: (forgetKeys: boolean) => invoke<void>("delete_all_data", { forgetKeys }),
  openDataFolder: () => invoke<void>("open_data_folder"),
  checkUpdate: () => invoke<UpdateInfo | null>("check_update"),
  installUpdate: () => invoke<void>("install_update"),
};

export function onEngineEvent(handler: (e: EngineEvent) => void): Promise<UnlistenFn> {
  return listen<EngineEvent>("engine-event", (msg) => handler(msg.payload));
}

export function onLiveUpdate(handler: (s: LiveState) => void): Promise<UnlistenFn> {
  return listen<LiveState>("live-update", (msg) => handler(msg.payload));
}

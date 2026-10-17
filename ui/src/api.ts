import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AccountView,
  ActivityEntry,
  AppSettings,
  ChampionEntry,
  DashboardView,
  EngineEvent,
  KeyStatus,
  MatchDetail,
  MatchRow,
  MetricDef,
  ProviderInfo,
  Report,
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
};

export function onEngineEvent(handler: (e: EngineEvent) => void): Promise<UnlistenFn> {
  return listen<EngineEvent>("engine-event", (msg) => handler(msg.payload));
}

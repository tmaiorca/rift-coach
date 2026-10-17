import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { EngineEvent, KeyStatus, MatchRow, StatusView } from "./types";

export const api = {
  getStatus: () => invoke<StatusView>("get_status"),
  startIngest: (riotId: string, platform: string, count: number) =>
    invoke<number>("start_ingest", { riotId, platform, count }),
  listMatches: (limit: number, offset = 0) => invoke<MatchRow[]>("list_matches", { limit, offset }),
  championIcon: (championId: number, patch: string) => invoke<string>("champion_icon", { championId, patch }),
  reloadKey: () => invoke<KeyStatus>("reload_key"),
};

export function onEngineEvent(handler: (e: EngineEvent) => void): Promise<UnlistenFn> {
  return listen<EngineEvent>("engine-event", (msg) => handler(msg.payload));
}

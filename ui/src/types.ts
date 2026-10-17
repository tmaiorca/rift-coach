// Mirrors of the Rust types returned by Tauri commands (src-tauri/src/commands.rs).

export type KeyStatus = "unknown" | "ok" | "missing" | "rejected";
export type KeySource = "process_env" | "user_env";

export interface JobCounts {
  ready: number;
  scheduled: number;
  running: number;
  done: number;
  failed: number;
}

export interface WindowSnapshot {
  limit: number;
  period_s: number;
  used: number;
}

export interface BucketSnapshot {
  routing: string;
  scope: string;
  windows: WindowSnapshot[];
  blocked_for_ms: number;
}

export interface FailedJob {
  kind: string;
  dedupe_key: string;
  last_error: string | null;
  updated_at: number;
}

export interface EngineStatus {
  key_status: KeyStatus;
  paused: boolean;
  jobs: JobCounts;
  rate_limits: BucketSnapshot[];
  recent_failures: FailedJob[];
}

export interface Account {
  id: number;
  game_name: string;
  tag_line: string;
  platform: string;
  region: string;
  is_primary: boolean;
}

export interface StatusView {
  key_source: KeySource | null;
  engine: EngineStatus;
  account: Account | null;
  counts: { matches: number; with_timeline: number } | null;
  data_dir: string;
}

export interface MatchRow {
  match_id: string;
  queue_id: number;
  map_id: number;
  patch: string;
  game_start_ms: number;
  duration_s: number;
  is_remake: boolean;
  has_timeline: boolean;
  champion_id: number;
  champion_name: string;
  team_position: string | null;
  win: boolean;
  kills: number;
  deaths: number;
  assists: number;
  cs: number;
  gold_earned: number;
  vision_score: number;
  champ_level: number;
  queue_name: string;
  is_standard_rift: boolean;
}

export type EngineEvent =
  | { type: "job_started"; kind: string; label: string }
  | { type: "job_succeeded"; kind: string; label: string }
  | { type: "job_retrying"; kind: string; label: string; error: string; retry_in_s: number }
  | { type: "job_failed"; kind: string; label: string; error: string }
  | { type: "match_stored"; match_id: string }
  | { type: "timeline_stored"; match_id: string }
  | { type: "key_status_changed"; status: KeyStatus }
  | { type: "key_generation_changed"; key_gen_id: number };

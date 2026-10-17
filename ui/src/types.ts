// Mirrors of the Rust types returned by Tauri commands (src-tauri/src/commands.rs
// and the crates it re-exports). Keep in sync.

export type KeyStatus = "unknown" | "ok" | "missing" | "rejected";
export type KeySource = "credential_manager" | "process_env" | "user_env";

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

export interface MatchCounts {
  matches: number;
  with_timeline: number;
}

export interface AccountView extends Account {
  counts: MatchCounts;
  is_active: boolean;
}

export interface StatusView {
  key_source: KeySource | null;
  engine: EngineStatus;
  account: Account | null;
  counts: MatchCounts | null;
  data_dir: string;
  version: string;
}

export interface ActivityEntry {
  id: number;
  at: number;
  level: "info" | "warn" | "error";
  message: string;
}

export interface MatchSummary {
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
}

export interface MatchRow extends MatchSummary {
  queue_name: string;
  is_standard_rift: boolean;
  has_review: boolean;
}

export type MetricGroup = "laning" | "macro" | "fighting" | "vision" | "items" | "overall";
export type MetricUnit = "count" | "gold" | "xp" | "percent" | "minutes" | "seconds" | "per_minute" | "ratio";

export interface MetricDef {
  key: string;
  label: string;
  group: MetricGroup;
  unit: MetricUnit;
  higher_is_better: boolean | null;
  decimals: number;
  approximate: boolean;
  zero_sum: boolean;
  description: string;
}

export interface Comparison {
  better_than_pct: number;
  median: number;
  n: number;
}

export interface MetricView {
  key: string;
  label: string;
  group: MetricGroup;
  unit: MetricUnit;
  decimals: number;
  higher_is_better: boolean | null;
  approximate: boolean;
  zero_sum: boolean;
  description: string;
  value: number;
  vs_role: Comparison | null;
  vs_champion: Comparison | null;
  vs_personal: Comparison | null;
}

export interface Populations {
  role: string | null;
  champion_name: string;
  role_label: string;
  champion_label: string;
  personal_label: string;
  baseline_games: number;
}

export interface ScoreRow {
  participant_id: number;
  team_id: number;
  riot_id: string | null;
  champion_id: number;
  champion_name: string;
  team_position: string | null;
  win: boolean;
  kills: number;
  deaths: number;
  assists: number;
  level: number;
  cs: number;
  gold: number;
  damage: number;
  vision_score: number;
  items: number[];
  summoner_spells: number[];
  is_subject: boolean;
}

export interface ParticipantSeries {
  gold: number[];
  xp: number[];
  cs: number[];
  level: number[];
}

export interface Series {
  minutes: number[];
  by_participant: Record<string, ParticipantSeries>;
}

export interface MapEvent {
  kind: "kill" | "objective" | "building";
  t_s: number;
  x: number;
  y: number;
  killer_id: number | null;
  victim_id: number | null;
  assist_ids: number[];
  team_id: number | null;
  label: string | null;
  shutdown_bounty: number;
}

export interface DeathContext {
  t_s: number;
  x: number;
  y: number;
  killer_id: number | null;
  assist_ids: number[];
  enemies_involved: number;
  before_14: boolean;
  in_enemy_half: boolean;
  gold_vs_opponent: number | null;
  unspent_gold: number | null;
  shutdown_bounty: number;
}

export interface ReviewItem {
  title: string;
  detail: string;
  evidence: string[];
}

export interface ReviewFix {
  title: string;
  why: string;
  how: string;
  evidence: string[];
}

export interface ReviewOutput {
  summary: string;
  went_well: ReviewItem[];
  improve: ReviewItem[];
  priority_fixes: ReviewFix[];
  next_game_focus: { goal: string; metric?: string };
  confidence: "low" | "medium" | "high";
  confidence_reason?: string;
}

export interface Report {
  id: number;
  kind: string;
  account_id: number;
  match_id: string | null;
  provider: string;
  model: string;
  prompt_version: string;
  output: ReviewOutput;
  validation: "ok" | "unverified";
  notes: string | null;
  input_tokens: number | null;
  output_tokens: number | null;
  created_at: number;
}

export interface MatchDetail {
  summary: MatchSummary;
  queue_name: string;
  is_standard_rift: boolean;
  participant_id: number;
  opponent_id: number | null;
  scoreboard: ScoreRow[];
  comparison: { populations: Populations; metrics: MetricView[] };
  series: Series | null;
  events: MapEvent[];
  deaths: DeathContext[];
  report: Report | null;
}

export interface GameMetrics {
  match_id: string;
  game_start_ms: number;
  champion_name: string;
  champion_id: number;
  team_position: string | null;
  win: boolean;
  metrics: Record<string, number>;
}

export interface RoleChampionStat {
  team_position: string;
  champion_name: string;
  games: number;
  wins: number;
}

export interface DashboardView {
  account: Account | null;
  baseline_games: number;
  games: GameMetrics[];
  roles: RoleChampionStat[];
}

export type ProviderId = "anthropic" | "open_ai" | "gemini" | "open_ai_compatible";

export interface ProviderInfo {
  id: ProviderId;
  label: string;
  default_base_url: string;
  default_model: string;
  key_required: boolean;
  has_key: boolean;
}

export interface AppSettings {
  ingest_count: number;
  baseline_games: number;
  show_other_queues: boolean;
  active_account_id: number | null;
  ai: {
    provider: ProviderId;
    model: string;
    base_url: string | null;
    auto_post_game: boolean;
    max_tokens: number;
  };
  appearance: {
    animated_background: boolean;
    background_intensity: number;
    background_champion: string | null;
    particles: boolean;
  };
}

export interface ChampionEntry {
  key: number;
  id: string;
  name: string;
}

export type EngineEvent =
  | { type: "job_started"; kind: string; label: string }
  | { type: "job_succeeded"; kind: string; label: string }
  | { type: "job_retrying"; kind: string; label: string; error: string; retry_in_s: number }
  | { type: "job_failed"; kind: string; label: string; error: string }
  | { type: "match_stored"; match_id: string }
  | { type: "timeline_stored"; match_id: string }
  | { type: "key_status_changed"; status: KeyStatus }
  | { type: "key_generation_changed"; key_gen_id: number }
  | { type: "metrics_computed"; match_id: string }
  | { type: "review_ready"; match_id: string; report_id: number };

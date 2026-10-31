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
  /** Queued work is held because a game is running. */
  in_game: boolean;
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
  vs_benchmark: Comparison | null;
  vs_benchmark_champion: Comparison | null;
}

export interface Populations {
  role: string | null;
  champion_name: string;
  role_label: string;
  champion_label: string;
  personal_label: string;
  baseline_games: number;
  /** e.g. "Diamond mid laners, patch 16.20"; null until benchmarks are collected. */
  benchmark_label: string | null;
  benchmark_champion_label: string | null;
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
  zone: Zone;
  solo: boolean;
  enemy_objective: string | null;
}

export type Zone =
  | "own_base"
  | "enemy_base"
  | "top_lane"
  | "mid_lane"
  | "bot_lane"
  | "top_river"
  | "bot_river"
  | "own_jungle"
  | "enemy_jungle";

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

export type Comparator = "at_most" | "at_least";

export interface ReviewOutput {
  summary: string;
  went_well: ReviewItem[];
  improve: ReviewItem[];
  priority_fixes: ReviewFix[];
  next_game_focus: { goal: string; metric?: string; comparator?: Comparator; target?: number };
  goal_feedback?: string;
  confidence: "low" | "medium" | "high";
  confidence_reason?: string;
}

export interface Report {
  id: number;
  kind: string;
  account_id: number;
  match_id: string | null;
  scope: string | null;
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
  patterns: MatchPattern[];
  report: Report | null;
}

export interface GameMetrics {
  match_id: string;
  participant_id: number;
  game_start_ms: number;
  duration_s: number;
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
  session_gap_min: number;
  ai: {
    provider: ProviderId;
    model: string;
    base_url: string | null;
    auto_post_game: boolean;
    auto_session: boolean;
    auto_weekly: boolean;
    auto_pregame: boolean;
    max_tokens: number;
    monthly_token_limit: number | null;
  };
  appearance: {
    animated_background: boolean;
    background_intensity: number;
    background_champion: string | null;
    particles: boolean;
  };
  desktop: {
    close_to_tray: boolean;
    check_updates: boolean;
    onboarded: boolean;
  };
  mcp: {
    enabled: boolean;
    port: number;
    lan: boolean;
    require_token_local: boolean;
  };
  benchmarks: {
    enabled: boolean;
    tier: BenchmarkTier;
    champions: string[];
    step_seconds: number;
    max_games: number;
  };
  live: {
    enabled: boolean;
    pause_background: boolean;
    cs_target_pm: number | null;
    keep_snapshots_days: number;
    notify: boolean;
  };
  vision: VisionSettings;
}

/** Mirrors rc_store::settings::VisionSettings. */
export interface VisionSettings {
  enabled: boolean;
  interval_s: number;
  max_width: number;
  max_frames: number;
  auto_review: boolean;
  provider: ProviderId;
  model: string;
  base_url: string | null;
  max_tokens: number;
}

/** Mirrors rc_vision::CaptureStats. */
export interface CaptureStats {
  frames: number;
  bytes: number;
  width: number;
  height: number;
  avg_ms: number;
  max_ms: number;
  slowed: number;
  black: number;
  sessions: number;
  interval_s: number;
  error: string | null;
}

export interface DiskUsage {
  games: number;
  frames: number;
  bytes: number;
}

export type VisionState = "captured" | "reviewed" | "failed" | "deleted";

export interface VisionView {
  enabled: boolean;
  live_game_id: number | null;
  state: VisionState | null;
  frames_on_disk: number;
  stats: CaptureStats | null;
  report: Report | null;
  queued: boolean;
}

export type VisionCategory =
  | "positioning"
  | "map_awareness"
  | "vision"
  | "wave_management"
  | "trading"
  | "objective_setup"
  | "recall_timing"
  | "teamfight"
  | "macro"
  | "other";

/** A vision review's output (`Report.output` when `kind` is "vision"). */
export interface VisionReviewOutput {
  summary: string;
  moments: {
    id: string;
    /** Added by the app from the facts. */
    time?: string;
    kind?: "death" | "objective" | "checkpoint";
    title: string;
    seen: string;
    lesson: string;
    category: VisionCategory;
    severity: "low" | "medium" | "high";
    clear: boolean;
  }[];
  themes?: { title: string; detail?: string; moments?: string[] }[];
  next_game_focus?: { goal: string; how?: string };
  confidence: "low" | "medium" | "high";
  confidence_reason?: string;
}

export interface VisionLive {
  running: boolean;
  frames: number;
  interval_s: number;
  avg_ms: number;
  error: string | null;
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
  | { type: "metrics_computed"; match_id: string; benchmark: boolean }
  | { type: "review_ready"; match_id: string; report_id: number }
  | { type: "session_review_ready"; scope: string; report_id: number }
  | { type: "vision_review_ready"; match_id: string; report_id: number }
  | { type: "live_game_linked"; live_game_id: number; match_id: string }
  | { type: "post_game_ready"; live_game_id: number; match_id: string; review_queued: boolean }
  | { type: "in_game_changed"; in_game: boolean }
  | { type: "weekly_report_ready"; week: string; report_id: number }
  | { type: "pregame_ready"; live_game_id: number | null };

// ---------- patterns (rc-signals via rc-coach) ----------

export type Polarity = "weakness" | "strength";
export type Severity = "low" | "medium" | "high";

export interface Evidence {
  match_id: string;
  t_s: number | null;
  x: number | null;
  y: number | null;
  note: string;
}

export interface GoalSuggestion {
  metric: string;
  comparator: Comparator;
  target: number;
}

export interface Signal {
  id: string;
  kind: string;
  polarity: Polarity;
  severity: Severity;
  score: number;
  title: string;
  detail: string;
  tip: string;
  games_considered: number;
  games_affected: number;
  rate_pct: number;
  metric: string | null;
  win_rate_affected: number | null;
  win_rate_other: number | null;
  evidence: Evidence[];
  goal: GoalSuggestion | null;
}

export type LinkStrength = "none" | "weak" | "moderate" | "strong";

export interface WinLink {
  metric: string;
  label: string;
  early: boolean;
  games: number;
  wins: number;
  higher_in_wins_pct: number;
  win_median: number;
  loss_median: number;
  win_rate_above: number | null;
  win_rate_below: number | null;
  p_value: number;
  strength: LinkStrength;
  summary: string;
}

export interface MapDeath {
  match_id: string;
  t_s: number;
  x: number;
  y: number;
  zone: Zone;
  win: boolean;
  blue_side: boolean;
}

export interface PatternsView {
  window: number;
  role: string | null;
  roles: [string, number][];
  games: number;
  wins: number;
  signals: Signal[];
  win_links: WinLink[];
  deaths: MapDeath[];
}

export interface GameFlag {
  kind: string;
  title: string;
  polarity: Polarity;
  what: string;
  evidence: Evidence[];
}

export interface MatchPattern extends GameFlag {
  recurring: {
    games_affected: number;
    games_considered: number;
    rate_pct: number;
    win_rate_affected: number | null;
    win_rate_other: number | null;
  } | null;
}

// ---------- goals ----------

export interface GoalGame {
  match_id: string;
  game_start_ms: number;
  champion_name: string;
  champion_id: number;
  win: boolean;
  value: number;
  met: boolean;
}

export interface GoalProgress {
  games_played: number;
  games_total: number;
  average: number | null;
  games_met: number;
  on_track: boolean | null;
  done: boolean;
  improvement: number | null;
  games: GoalGame[];
}

export interface Goal {
  id: number;
  account_id: number;
  metric: string;
  comparator: Comparator;
  target: number;
  games: number;
  role: string | null;
  champion_name: string | null;
  baseline: number | null;
  source: "user" | "ai" | "pattern";
  note: string | null;
  status: "active" | "completed" | "abandoned";
  achieved: boolean | null;
  created_at: number;
  ended_at: number | null;
  label: string;
  unit: MetricUnit;
  decimals: number;
  higher_is_better: boolean | null;
  title: string;
  progress: GoalProgress;
}

export interface GoalInput {
  metric: string;
  comparator: Comparator;
  target: number;
  games: number;
  role: string | null;
  champion_name: string | null;
  source?: "user" | "ai" | "pattern";
  note?: string | null;
}

// ---------- sessions ----------

export interface SessionSummary {
  scope: string;
  first_match_id: string;
  start_ms: number;
  end_ms: number;
  games: number;
  rift_games: number;
  wins: number;
  losses: number;
  champion_ids: number[];
  has_report: boolean;
}

export interface SessionGame {
  summary: MatchSummary;
  queue_name: string;
  is_standard_rift: boolean;
  metrics: Record<string, number>;
  flags: GameFlag[];
}

export interface Aggregate {
  metric: string;
  label: string;
  unit: MetricUnit;
  decimals: number;
  higher_is_better: boolean | null;
  session: number;
  baseline: number | null;
  better: boolean | null;
  games: number;
}

export interface SessionPattern {
  kind: string;
  title: string;
  what: string;
  polarity: Polarity;
  session_games_affected: number;
  session_games: number;
  baseline_rate_pct: number | null;
}

export interface SessionDetail {
  summary: SessionSummary;
  games: SessionGame[];
  aggregates: Aggregate[];
  patterns: SessionPattern[];
  baseline_games: number;
  report: Report | null;
}

export interface AiUsage {
  reports_30d: number;
  tokens_30d: number;
  limit: number | null;
}

// ---------- live (rc-live via rc-jobs) ----------

export type LivePhase = "idle" | "loading" | "in_game" | "ended";

export interface LiveItem {
  itemID: number;
  count: number;
  slot: number;
}

export interface OwnSnapshot {
  riot_id: string;
  champion_name: string;
  champion_ddragon_id: string | null;
  position: string;
  game_mode: string;
  map_number: number;
  game_time_s: number;
  level: number;
  current_gold: number;
  scores: { kills: number; deaths: number; assists: number; creepScore: number; wardScore: number };
  items: LiveItem[];
  is_dead: boolean;
  game_end: string | null;
}

export interface CsTarget {
  per_min: number;
  source: "goal" | "setting" | "usual";
}

export interface Benchmark {
  per_min: number;
  label: string;
}

export interface PlannedItem {
  item_id: number;
  name: string;
  total_gold: number;
}

export interface GoalReminder {
  goal_id: number;
  title: string;
  metric: string;
  label: string;
  comparator: "at_most" | "at_least";
  target: number;
}

export interface LiveContext {
  account_id: number | null;
  champion_id: number | null;
  champion_ddragon_id: string | null;
  role: string | null;
  cs_target: CsTarget | null;
  benchmark: Benchmark | null;
  build: PlannedItem[];
  build_source: "plan" | "usual" | null;
  goals: GoalReminder[];
  patch: string | null;
}

export interface Pacing {
  cs: number;
  cs_per_min: number;
  target: CsTarget | null;
  expected_cs: number | null;
  diff: number | null;
  benchmark: Benchmark | null;
}

export interface BuildProgress {
  plan: { item_id: number; name: string; owned: boolean }[];
  next: {
    item_id: number;
    name: string;
    total_gold: number;
    components_gold: number;
    remaining_gold: number;
    progress_pct: number;
  } | null;
}

export interface GoalLive extends GoalReminder {
  value: number | null;
  pending: string | null;
  met: boolean | null;
}

export interface Footprint {
  samples: number;
  cpu_avg_pct: number;
  cpu_max_pct: number;
  backend_cpu_avg_pct: number;
  mem_max_mb: number;
  backend_mem_max_mb: number;
}

export interface LiveState {
  enabled: boolean;
  phase: LivePhase;
  live_game_id: number | null;
  tracked: boolean;
  snapshot: OwnSnapshot | null;
  context: LiveContext | null;
  pacing: Pacing | null;
  build: BuildProgress | null;
  goals: GoalLive[];
  footprint: Footprint | null;
  brief: BriefState | null;
  vision: VisionLive | null;
  updated_ms: number;
}

export interface BriefRecord {
  games: number;
  wins: number;
  averages: Record<string, number>;
}

export interface Brief {
  game_id: number;
  champion: string;
  champion_id: number;
  role: string | null;
  record: BriefRecord;
  likely_opponent: { champion: string; champion_id: number; role_share_pct: number } | null;
  matchup: BriefRecord | null;
  enemies: { champion: string; champion_id: number; games: number; wins: number }[];
  reminders: { id: string; title: string; detail: string; tip: string }[];
  goals: string[];
  benchmark_label: string | null;
  targets: {
    metric: string;
    label: string;
    benchmark_median: number;
    your_average: number | null;
    decimals: number;
  }[];
}

export interface GamePlanItem {
  title: string;
  detail: string;
  evidence: string[];
}

export interface GamePlan {
  summary: string;
  plan: GamePlanItem[];
  watch_out: GamePlanItem[];
  focus: string;
  confidence: "low" | "medium" | "high";
  confidence_reason?: string;
}

export interface BriefState {
  brief: Brief | null;
  loading: boolean;
  note: string | null;
  account_id: number | null;
  plan_status: "off" | "generating" | "ready" | "failed";
  plan: GamePlan | null;
  plan_report_id: number | null;
  plan_error: string | null;
}

// ---------- benchmarks, ranks, weeks (Phase 5) ----------

export type BenchmarkTier = "AUTO" | "GOLD" | "PLATINUM" | "EMERALD" | "DIAMOND" | "MASTER";

export interface SampleProgress {
  role: string;
  champion: string | null;
  label: string;
  samples: number;
  target: number;
}

export interface BenchmarkStatus {
  progress: {
    enabled: boolean;
    /** The tier in use ("AUTO" resolved). */
    tier: Exclude<BenchmarkTier, "AUTO">;
    tier_label: string;
    /** Picked from your solo rank. */
    auto: boolean;
    patch: string | null;
    focus: { roles: string[]; champions: string[] };
    targets: SampleProgress[];
    games_by_patch: [string, number][];
    collector: {
      players: number;
      players_checked: number;
      focus_players: number;
      players_scanned: number;
      queued: number;
    };
  };
  collector: {
    phase: "starting" | "disabled" | "waiting" | "collecting" | "done" | "nothing_to_do";
    detail: string;
    last_error: string | null;
    fetched_this_run: number;
    updated_ms: number;
  };
}

export interface Gap {
  metric: string;
  label: string;
  unit: MetricUnit;
  decimals: number;
  higher_is_better: boolean;
  yours: number;
  benchmark_median: number;
  better_than_pct: number;
  n: number;
  early: boolean;
}

export interface GapsView {
  reference: string | null;
  role: string;
  champion: string | null;
  games: number;
  gaps: Gap[];
}

export interface Rank {
  queue_type: "RANKED_SOLO_5x5" | "RANKED_FLEX_SR" | string;
  tier: string | null;
  division: string | null;
  lp: number;
  wins: number;
  losses: number;
  fetched_at: number;
}

export interface WeekSummary {
  week: string;
  start_ms: number;
  end_ms: number;
  games: number;
  rift_games: number;
  wins: number;
  losses: number;
  complete: boolean;
  has_report: boolean;
}

export interface WeekDetail {
  summary: WeekSummary;
  baseline_games: number;
  aggregates: Aggregate[];
  patterns: SessionPattern[];
  champions: { champion: string; champion_id: number; games: number; wins: number }[];
  trend: { week: string; games: number; win_rate_pct: number | null; metrics: Record<string, number> }[];
  benchmark: GapsView | null;
  report: Report | null;
}

export type LiveGameStatus =
  "in_progress" | "not_tracked" | "not_published" | "waiting" | "analyzing" | "analyzed" | "reviewed";

export interface LiveGameView {
  id: number;
  account_id: number | null;
  champion_name: string;
  champion_ddragon: string | null;
  position: string | null;
  game_mode: string;
  started_at: number;
  ended_at: number | null;
  end_reason: "game_end" | "disappeared" | null;
  result: "Win" | "Lose" | null;
  duration_s: number | null;
  match_id: string | null;
  analyzed_at: number | null;
  report_id: number | null;
  snapshots_pruned: boolean;
  vision_json: string | null;
  vision_state: VisionState | null;
  vision_report_id: number | null;
  status: LiveGameStatus;
  last: OwnSnapshot | null;
  footprint: Footprint | null;
  budget_cpu_avg_pct: number;
  budget_mem_mb: number;
  within_budget: boolean | null;
  vision: CaptureStats | null;
}

export interface PlannerItem {
  item_id: number;
  name: string;
  total_gold: number;
  completed: boolean;
}

export interface BuildPlanView {
  champion: string;
  source: "plan" | "usual" | null;
  items: PlannedItem[];
  patch: string;
}

// ---------- champions ----------

export interface ChampionSummary {
  champion: string;
  champion_id: number;
  games: number;
  wins: number;
  roles: [string, number][];
  last_played_ms: number;
  averages: Record<string, number>;
  focus: boolean;
}

export interface Matchup {
  champion: string;
  champion_id: number;
  games: number;
  wins: number;
  gold_diff_15: number | null;
  cs_diff_10: number | null;
  deaths_pre14: number | null;
}

export interface ChampionVsRole {
  metric: string;
  label: string;
  on_champion: number;
  role_average: number | null;
  better: boolean | null;
}

export interface ChampionRecentGame {
  match_id: string;
  start_ms: number;
  win: boolean;
  role: string | null;
  opponent: string | null;
  opponent_id: number | null;
  kills: number | null;
  deaths: number | null;
  assists: number | null;
}

export interface ChampionDetail {
  summary: ChampionSummary;
  role: string | null;
  role_games: number;
  vs_role: ChampionVsRole[];
  matchups: Matchup[];
  signals: Signal[];
  benchmark: GapsView | null;
  recent: ChampionRecentGame[];
}

// ---------- desktop, MCP, data, updates ----------

export interface McpStatus {
  running: boolean;
  error: string | null;
  local_url: string | null;
  lan_url: string | null;
  token_required: boolean;
  exe_path: string;
}

export interface BackupInfo {
  schema_version: number;
  accounts: number;
  matches: number;
  reports: number;
}

export interface UpdateInfo {
  version: string;
  current_version: string;
  notes: string | null;
}

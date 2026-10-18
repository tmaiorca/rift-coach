import type { MetricUnit, Zone } from "../types";

export const ROLE_LABEL: Record<string, string> = {
  TOP: "Top",
  JUNGLE: "Jungle",
  MIDDLE: "Mid",
  BOTTOM: "ADC",
  UTILITY: "Support",
};

export const roleLabel = (r: string | null | undefined) => (r ? (ROLE_LABEL[r] ?? r) : "-");

export const ZONE_LABEL: Record<Zone, string> = {
  own_base: "your base",
  enemy_base: "enemy base",
  top_lane: "top lane",
  mid_lane: "mid lane",
  bot_lane: "bot lane",
  top_river: "top river",
  bot_river: "bot river",
  own_jungle: "your jungle",
  enemy_jungle: "enemy jungle",
};

/** 754 → "12:34" */
export function clock(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

export function formatMetric(value: number, unit: MetricUnit, decimals: number): string {
  if (!Number.isFinite(value)) return "-";
  if (unit === "minutes") return clock(value * 60);
  const n = value.toLocaleString(undefined, {
    minimumFractionDigits: decimals,
    maximumFractionDigits: decimals,
  });
  return unit === "percent" ? `${n}%` : n;
}

export function signed(n: number): string {
  return n > 0 ? `+${n.toLocaleString()}` : n.toLocaleString();
}

export function dateTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

export function ago(ms: number): string {
  const d = Date.now() - ms;
  const m = Math.round(d / 60000);
  if (m < 1) return "just now";
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 48) return `${h} h ago`;
  return `${Math.round(h / 24)} days ago`;
}

/** Tone for a "better than X%" value. */
export function pctTone(p: number): "good" | "ok" | "bad" {
  return p >= 60 ? "good" : p >= 40 ? "ok" : "bad";
}

export const PLATFORMS = [
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

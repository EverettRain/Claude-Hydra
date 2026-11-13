import type { Quota, QuotaHistory, QuotaRun } from "./api";
import { dayFrom, dayKey } from "./usage";

const HOUR = 3_600_000;
/** Reset deadlines are rounded by the service; within this they are the same cycle. */
const RESET_TOLERANCE = 1_000;
const DAYS = 14;

/**
 * The last recorded reading of each window, as a quota to show before a fresh one arrives.
 * Windows not seen in the latest round of readings are left out (the service dropped them).
 */
export function latestQuota(history: QuotaHistory): Quota | null {
  const last = Object.entries(history.windows).flatMap(([id, runs]) => (runs.length ? [{ id, run: runs[runs.length - 1] }] : []));
  if (!last.length) return null;
  const newest = Math.max(...last.map((w) => w.run.to));
  const rank = (id: string) => (id === "five_hour" ? 0 : id === "seven_day" ? 1 : 2);
  const windows = last.filter((w) => newest - w.run.to <= HOUR)
    .sort((a, b) => rank(a.id) - rank(b.id) || a.id.localeCompare(b.id))
    .map((w) => ({ id: w.id, utilization: w.run.utilization, resetsAt: w.run.resetsAt === null ? null : new Date(w.run.resetsAt).toISOString() }));
  return { windows, fetchedAt: newest };
}

export function windowLabel(id: string): string {
  const known: Record<string, string> = { five_hour: "5 小时", seven_day: "本周", seven_day_opus: "本周 · Opus", seven_day_sonnet: "本周 · Sonnet", seven_day_oauth_apps: "本周 · OAuth 应用" };
  if (known[id]) return known[id];
  const model = id.match(/^seven_day_(.+)$/)?.[1];
  return model ? `本周 · ${model.replace(/_/g, " ")}` : id.replace(/_/g, " ");
}

/** Length of a window's cycle, when its kind tells it. */
export function windowDuration(id: string): number | null {
  if (id === "five_hour") return 5 * HOUR;
  if (id.startsWith("seven_day")) return 7 * 24 * HOUR;
  return null;
}

const sameCycle = (a: number | null, b: number | null) => (a === null || b === null ? a === b : Math.abs(a - b) <= RESET_TOLERANCE);

interface Point { at: number; used: number; resetsAt: number | null }

/** Observations in time order: each run's first and last reading. */
function points(runs: QuotaRun[]): Point[] {
  return runs.flatMap((r) => (r.to > r.from
    ? [{ at: r.from, used: r.utilization, resetsAt: r.resetsAt }, { at: r.to, used: r.utilization, resetsAt: r.resetsAt }]
    : [{ at: r.from, used: r.utilization, resetsAt: r.resetsAt }]));
}

export interface Trend {
  /** Axis range (ms). */
  start: number;
  end: number;
  /** Whether the range is the advertised cycle (then an even-use guide applies). */
  cycle: boolean;
  /** Remaining percentage over time. */
  line: { at: number; remaining: number }[];
  remaining: number | null;
  resetsAt: number | null;
}

/** The latest cycle's remaining quota. Without a known cycle, the last 14 days. */
export function trend(runs: QuotaRun[], id: string, now = Date.now()): Trend {
  const all = points(runs);
  const last = all[all.length - 1];
  const duration = windowDuration(id);
  if (last && last.resetsAt !== null && duration !== null) {
    const end = last.resetsAt;
    const start = end - duration;
    // Readings outside the cycle's span cannot belong to it; never squeeze them onto the edge.
    const line = all.filter((p) => sameCycle(p.resetsAt, end) && p.at >= start && p.at <= end)
      .map((p) => ({ at: p.at, remaining: 100 - p.used }));
    return { start, end, cycle: true, line, remaining: 100 - last.used, resetsAt: end };
  }
  const start = now - DAYS * 24 * HOUR;
  return {
    start, end: now, cycle: false,
    line: all.filter((p) => p.at >= start && (!last || sameCycle(p.resetsAt, last.resetsAt))).map((p) => ({ at: p.at, remaining: 100 - p.used })),
    remaining: last ? 100 - last.used : null,
    resetsAt: last?.resetsAt ?? null,
  };
}

export interface DailyUse {
  /** Local days, oldest first; `used` is null when no interval of that day was measured. */
  days: { key: string; used: number | null }[];
  /** Increases whose endpoints fall on different days: their timing within the gap is unknown. */
  unallocated: number;
}

/**
 * Observed consumption per local day (token-monitor's rule): an increase in used percentage
 * between two compatible readings counts for a day only when both readings fall on it.
 * A new cycle or a decrease is a reset or refill, never consumption. An unchanged interval is a
 * measured zero.
 */
export function dailyUse(runs: QuotaRun[], now = Date.now()): DailyUse {
  const keys = Array.from({ length: DAYS }, (_, i) => dayKey(dayFrom(now, i - (DAYS - 1)).getTime()));
  const used = new Map<string, number>();
  let unallocated = 0;
  const p = points(runs);
  for (let i = 1; i < p.length; i++) {
    const [a, b] = [p[i - 1], p[i]];
    if (!sameCycle(a.resetsAt, b.resetsAt) || b.used < a.used) continue;
    const [da, db] = [dayKey(a.at), dayKey(b.at)];
    if (!keys.includes(db)) continue;
    if (da === db) used.set(db, (used.get(db) ?? 0) + (b.used - a.used));
    else unallocated += b.used - a.used;
  }
  return { days: keys.map((key) => ({ key, used: used.has(key) ? used.get(key)! : null })), unallocated };
}

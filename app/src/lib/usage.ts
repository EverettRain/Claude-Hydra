import type { CodeAccount, CodeUsage, Tokens } from "./api";

export const ZERO: Tokens = { input: 0, output: 0, cacheCreation: 0, cacheRead: 0 };

export const totalOf = (t: Tokens) => t.input + t.output + t.cacheCreation + t.cacheRead;

export const addTokens = (a: Tokens, b: Tokens): Tokens => ({
  input: a.input + b.input, output: a.output + b.output, cacheCreation: a.cacheCreation + b.cacheCreation, cacheRead: a.cacheRead + b.cacheRead,
});

/** Local calendar date, e.g. "2026-11-13". */
export function dayKey(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** The local date `offset` days from `now` (negative = earlier), by calendar arithmetic so a
 * daylight-saving day is still one day. */
export function dayFrom(now: number, offset: number): Date {
  const d = new Date(now);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() + offset);
}

export interface Day { key: string; tokens: Tokens; cost: number }

/** Recorded days, by local date. */
export function byDay(usages: CodeUsage[]): Map<string, Day> {
  const days = new Map<string, Day>();
  for (const u of usages) for (const h of u.hours) {
    const key = dayKey(h.start);
    const d = days.get(key) ?? { key, tokens: ZERO, cost: 0 };
    days.set(key, { key, tokens: addTokens(d.tokens, h.tokens), cost: d.cost + h.cost });
  }
  return days;
}

/** The last `days` local days ending today, oldest first; days without records are zero. */
export function daily(usages: CodeUsage[], days: number, now = Date.now()): Day[] {
  const sums = byDay(usages);
  return Array.from({ length: days }, (_, i) => {
    const key = dayKey(dayFrom(now, i - (days - 1)).getTime());
    return sums.get(key) ?? { key, tokens: ZERO, cost: 0 };
  });
}

export const sumDays = (days: Day[]) => days.reduce((t, d) => addTokens(t, d.tokens), ZERO);

export interface Highlights {
  activeDays: number;
  longestStreak: number;
  peak: Day | null;
}

export function highlights(usages: CodeUsage[]): Highlights {
  const days = [...byDay(usages).values()].filter((d) => totalOf(d.tokens) > 0).sort((a, b) => a.key.localeCompare(b.key));
  let longestStreak = 0;
  let run = 0;
  let previous: string | null = null;
  for (const d of days) {
    const [y, m, day] = d.key.split("-").map(Number);
    const yesterday = dayKey(new Date(y, m - 1, day - 1).getTime());
    run = previous === yesterday ? run + 1 : 1;
    longestStreak = Math.max(longestStreak, run);
    previous = d.key;
  }
  const peak = days.reduce<Day | null>((best, d) => (!best || totalOf(d.tokens) > totalOf(best.tokens) ? d : best), null);
  return { activeDays: days.length, longestStreak, peak };
}

export function compact(n: number): string {
  // Three significant digits without trailing zeros (1.23K, 56.8K, 5M); the unit is chosen
  // after rounding so 999,999 becomes 1M rather than 1000K.
  const units = ["", "K", "M", "B"];
  let v = n;
  let i = 0;
  while (i < units.length - 1 && Number(v.toPrecision(3)) >= 1000) { v /= 1000; i++; }
  return i ? `${Number(v.toPrecision(3))}${units[i]}` : String(Math.round(n));
}

export function money(usd: number): string {
  if (usd >= 1000) return `$${compact(usd)}`;
  return `$${usd >= 100 ? usd.toFixed(0) : usd.toFixed(2)}`;
}

export function duration(ms: number): string {
  const minutes = Math.round(ms / 60_000);
  const h = Math.floor(minutes / 60);
  return h ? `${h}h ${minutes % 60}m` : `${minutes}m`;
}

/** "Max 20x", "Pro" … from the subscription type and rate-limit tier Claude Code stores. */
export function planLabel(a: CodeAccount): string | null {
  const tier = a.tier?.toLowerCase() ?? "";
  const multiple = tier.match(/max_(\d+)x/)?.[1];
  if (multiple) return `Max ${multiple}x`;
  if (!a.plan) return null;
  return a.plan.charAt(0).toUpperCase() + a.plan.slice(1);
}

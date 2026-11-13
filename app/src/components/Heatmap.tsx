import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { compact, dayFrom, dayKey, money, totalOf, type Day } from "@/lib/usage";

const WEEKS = 53;
const GAP = 3;
const LEFT = 22;
const BOTTOM = 18;
const WEEKDAYS = ["一", "", "三", "", "五", "", ""];
/** One hue, light to dark: the step's opacity over the card. */
const STEPS = [0.3, 0.5, 0.75, 1];

export type Metric = "tokens" | "cost";

const valueOf = (d: Day | undefined, metric: Metric) => (!d ? 0 : metric === "tokens" ? totalOf(d.tokens) : d.cost);

/** Quartile thresholds of the non-zero values, so a single huge day does not wash out the rest. */
function thresholds(values: number[]): number[] {
  const sorted = values.filter((v) => v > 0).sort((a, b) => a - b);
  if (!sorted.length) return [];
  return [0.25, 0.5, 0.75].map((q) => sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))]);
}

/** A year of daily activity, one column per week (Monday first). */
export function Heatmap({ days, metric, now = Date.now() }: { days: Map<string, Day>; metric: Metric; now?: number }) {
  const box = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(760);
  const [hover, setHover] = useState<string | null>(null);

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setWidth(e.contentRect.width));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const cell = Math.max(6, Math.min(14, Math.floor((width - LEFT) / WEEKS) - GAP));
  const step = cell + GAP;
  const today = dayKey(now);
  // Monday of the week 52 weeks before this one.
  const mondayOffset = (new Date(now).getDay() + 6) % 7;
  const start = -(mondayOffset + 7 * (WEEKS - 1));

  const grid = useMemo(() => {
    const out: { key: string; col: number; row: number; month: number; date: number }[] = [];
    for (let i = 0; i < WEEKS * 7; i++) {
      const d = dayFrom(now, start + i);
      const key = dayKey(d.getTime());
      if (key > today) break;
      out.push({ key, col: Math.floor(i / 7), row: i % 7, month: d.getMonth(), date: d.getDate() });
    }
    return out;
  }, [now, start, today]);

  const limits = useMemo(() => thresholds(grid.map((g) => valueOf(days.get(g.key), metric))), [grid, days, metric]);
  const level = (v: number) => (v <= 0 ? -1 : limits.filter((t) => v > t).length);
  const months = grid.filter((g) => g.row === 0 && g.date <= 7);
  const shown = hover ? days.get(hover) : undefined;

  return (
    <div ref={box}>
      <div className="mb-1.5 h-4 px-0.5 text-[11.5px] text-muted">
        {hover
          ? <><span className="font-mono tabular-nums text-fg">{metric === "tokens" ? compact(valueOf(shown, "tokens")) : money(valueOf(shown, "cost"))}</span> {metric === "tokens" ? "tokens" : "（API 等价）"} · {hover}</>
          : "悬停查看每天的数值"}
      </div>
      <svg width={LEFT + WEEKS * step} height={7 * step + BOTTOM} role="img" aria-label={`最近一年每日 ${metric === "tokens" ? "token" : "成本"} 热力图`} onPointerLeave={() => setHover(null)}>
        {WEEKDAYS.map((w, i) => w && (
          <text key={i} x={0} y={i * step + cell - 2} fontSize={10} style={{ fill: "rgb(var(--muted))" }}>{w}</text>
        ))}
        {grid.map((g) => {
          const lv = level(valueOf(days.get(g.key), metric));
          return (
            <rect
              key={g.key}
              x={LEFT + g.col * step}
              y={g.row * step}
              width={cell}
              height={cell}
              rx={2.5}
              style={{
                fill: lv < 0 ? "rgb(var(--pill))" : `rgb(var(--accent) / ${STEPS[lv]})`,
                stroke: hover === g.key ? "rgb(var(--fg))" : "none",
                strokeWidth: 1.5,
              }}
              onPointerEnter={() => setHover(g.key)}
            >
              <title>{`${g.key}：${metric === "tokens" ? `${compact(valueOf(days.get(g.key), "tokens"))} tokens` : money(valueOf(days.get(g.key), "cost"))}`}</title>
            </rect>
          );
        })}
        {months.map((g) => (
          <text key={g.key} x={LEFT + g.col * step} y={7 * step + 12} fontSize={10.5} style={{ fill: "rgb(var(--muted))" }}>{g.month + 1}月</text>
        ))}
      </svg>
      <div className="mt-1 flex items-center justify-end gap-1 text-[10.5px] text-muted">
        少
        {[-1, 0, 1, 2, 3].map((lv) => (
          <span key={lv} className="inline-block h-2.5 w-2.5 rounded-[2px]" style={{ background: lv < 0 ? "rgb(var(--pill))" : `rgb(var(--accent) / ${STEPS[lv]})` }} />
        ))}
        多
      </div>
    </div>
  );
}

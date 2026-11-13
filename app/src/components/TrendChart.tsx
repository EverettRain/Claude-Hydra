import { useLayoutEffect, useRef, useState } from "react";
import { compact, money, totalOf, type Day } from "@/lib/usage";
import type { Metric } from "./Heatmap";

const HEIGHT = 168;
const PAD = { top: 12, right: 12, bottom: 24, left: 44 };

const shortDate = (key: string) => `${Number(key.slice(5, 7))}/${Number(key.slice(8, 10))}`;

/** Rounds the axis top up to 1, 2 or 5 × a power of ten. */
function niceMax(v: number): number {
  if (v <= 0) return 1;
  const p = 10 ** Math.floor(Math.log10(v));
  return ([1, 2, 5, 10].find((m) => m * p >= v) ?? 10) * p;
}

/** Daily totals (tokens or API-equivalent cost) as one line, with a crosshair readout and a table view. */
export function TrendChart({ days, label, metric = "tokens" }: { days: Day[]; label: string; metric?: Metric }) {
  const box = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(600);
  const [active, setActive] = useState<number | null>(null);
  const [table, setTable] = useState(false);

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setWidth(Math.max(240, e.contentRect.width)));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const values = days.map((d) => (metric === "tokens" ? totalOf(d.tokens) : d.cost));
  const fmt = metric === "tokens" ? compact : money;
  const top = niceMax(Math.max(...values, 0));
  const plotW = width - PAD.left - PAD.right;
  const plotH = HEIGHT - PAD.top - PAD.bottom;
  const x = (i: number) => PAD.left + (days.length > 1 ? (i / (days.length - 1)) * plotW : plotW / 2);
  const y = (v: number) => PAD.top + plotH - (v / top) * plotH;
  const line = values.map((v, i) => `${i ? "L" : "M"}${x(i).toFixed(1)},${y(v).toFixed(1)}`).join("");
  const area = `${line}L${x(days.length - 1).toFixed(1)},${y(0)}L${x(0).toFixed(1)},${y(0)}Z`;
  const ticks = [0, Math.floor((days.length - 1) / 2), days.length - 1].filter((v, i, a) => a.indexOf(v) === i);

  const pick = (clientX: number) => {
    const rect = box.current?.getBoundingClientRect();
    if (!rect || !days.length) return;
    const i = Math.round(((clientX - rect.left - PAD.left) / plotW) * (days.length - 1));
    setActive(Math.min(days.length - 1, Math.max(0, i)));
  };
  const shown = active ?? days.length - 1;
  const day = days[shown];

  return (
    <div>
      <div className="mb-1.5 flex items-baseline justify-between gap-3 px-1">
        <div className="min-w-0 text-[12px] text-muted">
          {day && (
            <>
              <span className="font-mono text-[13px] font-medium tabular-nums text-fg">{fmt(values[shown])}</span>
              <span className="ml-1.5">{active === null ? "今天" : day.key}</span>
              <span className="ml-2">
                {metric === "cost" && `${compact(totalOf(day.tokens))} tokens · `}输入 {compact(day.tokens.input)} · 输出 {compact(day.tokens.output)} · 缓存写入 {compact(day.tokens.cacheCreation)} · 缓存读取 {compact(day.tokens.cacheRead)}
              </span>
            </>
          )}
        </div>
        <button className="shrink-0 text-[11.5px] text-muted hover:text-fg active:!scale-100" onClick={() => setTable((t) => !t)}>
          {table ? "图表" : "表格"}
        </button>
      </div>
      <div ref={box} className="relative">
      {table ? (
        <div className="max-h-[220px] overflow-y-auto rounded-lg border border-line-2">
          <table className="w-full text-[11.5px] tabular-nums">
            <thead className="sticky top-0 bg-card text-muted">
              <tr>{["日期", "合计", "输入", "输出", "缓存写入", "缓存读取", "API 等价成本"].map((h) => <th key={h} className="px-2 py-1.5 text-right font-normal first:text-left">{h}</th>)}</tr>
            </thead>
            <tbody className="font-mono text-fg-2">
              {[...days].reverse().map((d) => (
                <tr key={d.key} className="border-t border-line-2">
                  <td className="px-2 py-1">{d.key}</td>
                  {[totalOf(d.tokens), d.tokens.input, d.tokens.output, d.tokens.cacheCreation, d.tokens.cacheRead].map((v, i) => <td key={i} className="px-2 py-1 text-right">{v.toLocaleString()}</td>)}
                  <td className="px-2 py-1 text-right">{money(d.cost)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
          <svg
            width={width}
            height={HEIGHT}
            role="img"
            aria-label={`${label}：最近 ${days.length} 天每日${metric === "tokens" ? " token 消耗" : "成本（API 等价）"}`}
            tabIndex={0}
            className="block outline-none focus-visible:rounded-md focus-visible:ring-2 focus-visible:ring-line"
            onPointerMove={(e) => pick(e.clientX)}
            onPointerLeave={() => setActive(null)}
            onBlur={() => setActive(null)}
            onKeyDown={(e) => {
              if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
                e.preventDefault();
                setActive((a) => Math.min(days.length - 1, Math.max(0, (a ?? days.length - 1) + (e.key === "ArrowLeft" ? -1 : 1))));
              }
            }}
          >
            {[0, 0.5, 1].map((f) => (
              <line key={f} x1={PAD.left} x2={width - PAD.right} y1={y(top * f)} y2={y(top * f)} strokeWidth={1} style={{ stroke: "rgb(var(--line-2))" }} />
            ))}
            {[0, 1].map((f) => (
              <text key={f} x={PAD.left - 8} y={y(top * f) + 3.5} textAnchor="end" fontSize={10.5} className="tabular-nums" style={{ fill: "rgb(var(--muted))" }}>
                {fmt(top * f)}
              </text>
            ))}
            {ticks.map((i) => (
              <text key={i} x={x(i)} y={HEIGHT - 6} textAnchor={i === 0 ? "start" : i === days.length - 1 ? "end" : "middle"} fontSize={10.5} style={{ fill: "rgb(var(--muted))" }}>
                {shortDate(days[i].key)}
              </text>
            ))}
            <path d={area} style={{ fill: "rgb(var(--accent) / 0.1)" }} />
            <path d={line} fill="none" style={{ stroke: "rgb(var(--accent))" }} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" />
            {active !== null && (
              <>
                <line x1={x(active)} x2={x(active)} y1={PAD.top} y2={y(0)} strokeWidth={1} style={{ stroke: "rgb(var(--muted))" }} />
                <circle cx={x(active)} cy={y(values[active])} r={4.5} strokeWidth={2} style={{ fill: "rgb(var(--accent))", stroke: "rgb(var(--card))" }} />
              </>
            )}
          </svg>
      )}
      </div>
    </div>
  );
}

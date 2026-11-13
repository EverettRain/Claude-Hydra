import { useLayoutEffect, useMemo, useRef, useState } from "react";
import type { QuotaHistory } from "@/lib/api";
import { dailyUse, trend, windowLabel } from "@/lib/quotaHistory";
import { cn } from "@/lib/utils";

const LINE_H = 120;
const BAR_H = 72;
const PAD = { left: 36, right: 8, top: 8 };

const pct = (v: number) => `${Math.round(v)}%`;
const time = (ms: number, withDate: boolean) => {
  const d = new Date(ms);
  const hm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  return withDate ? `${d.getMonth() + 1}/${d.getDate()} ${hm}` : hm;
};
const shortDay = (key: string) => `${Number(key.slice(5, 7))}/${Number(key.slice(8, 10))}`;

function useWidth() {
  const ref = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(480);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setWidth(Math.max(220, e.contentRect.width)));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return [ref, width] as const;
}

/** Remaining quota of the current cycle and the daily use of the last 14 days, per window. */
export function QuotaTrend({ history, order }: { history: QuotaHistory; order: string[] }) {
  const ids = useMemo(() => {
    const known = Object.keys(history.windows).filter((id) => history.windows[id].length);
    return [...order.filter((id) => known.includes(id)), ...known.filter((id) => !order.includes(id)).sort()];
  }, [history, order]);
  const [picked, setPicked] = useState<string | null>(null);
  const id = picked && ids.includes(picked) ? picked : ids[0];
  const [boxRef, width] = useWidth();
  const [hoverDay, setHoverDay] = useState<number | null>(null);

  return <div ref={boxRef}>{id ? <Panel history={history} ids={ids} id={id} onPick={setPicked} width={width} hoverDay={hoverDay} setHoverDay={setHoverDay} /> : <p className="text-[12px] text-muted">还没有额度记录。刷新一次额度后开始记录。</p>}</div>;
}

interface PanelProps {
  history: QuotaHistory;
  ids: string[];
  id: string;
  onPick: (id: string) => void;
  width: number;
  hoverDay: number | null;
  setHoverDay: (i: number | null) => void;
}

function Panel({ history, ids, id, onPick, width, hoverDay, setHoverDay }: PanelProps) {
  const runs = history.windows[id];
  const t = trend(runs, id);
  const d = dailyUse(runs);
  const plotW = width - PAD.left - PAD.right;
  const x = (at: number) => PAD.left + ((Math.min(Math.max(at, t.start), t.end) - t.start) / Math.max(1, t.end - t.start)) * plotW;
  const y = (remaining: number) => PAD.top + (1 - remaining / 100) * (LINE_H - PAD.top - 18);
  const line = t.line.map((p, i) => `${i ? "L" : "M"}${x(p.at).toFixed(1)},${y(p.remaining).toFixed(1)}`).join("");
  const lastAt = runs[runs.length - 1].to;
  const withDate = t.end - t.start > 24 * 3_600_000;
  const maxUse = Math.max(1, ...d.days.map((day) => day.used ?? 0));
  const step = plotW / d.days.length;
  const barW = Math.max(4, Math.min(18, step - 6));
  const shown = hoverDay ?? d.days.length - 1;
  const shownDay = d.days[shown];

  return (
    <div className="space-y-3">
      {ids.length > 1 && (
        <div role="tablist" aria-label="额度窗口" className="flex gap-0.5 rounded-lg bg-pill p-0.5">
          {ids.map((w) => (
            <button
              key={w}
              role="tab"
              aria-selected={w === id}
              onClick={() => onPick(w)}
              className={cn("h-6 flex-1 truncate rounded-md px-2 text-[12px] active:!scale-100", w === id ? "bg-card font-medium text-fg shadow-edge dark:bg-white/[.12]" : "text-muted hover:text-fg")}
            >
              {windowLabel(w)}
            </button>
          ))}
        </div>
      )}

      <div className="text-[13px]">
        {windowLabel(id)} · <span className="font-mono font-semibold tabular-nums">{t.remaining === null ? "—" : pct(t.remaining)}</span> 剩余
      </div>

      <div>
        <div className="mb-1 flex items-baseline justify-between text-[12px]">
          <span className="font-medium text-fg-2">剩余走势</span>
          <span className="font-mono text-[11px] text-muted">{time(lastAt, true)}</span>
        </div>
        <svg width={width} height={LINE_H} role="img" aria-label={`${windowLabel(id)}剩余额度走势，目前剩余 ${t.remaining === null ? "未知" : pct(t.remaining)}`}>
          {[100, 50, 0].map((v) => (
            <line key={v} x1={PAD.left} x2={width - PAD.right} y1={y(v)} y2={y(v)} strokeWidth={1} style={{ stroke: "rgb(var(--line-2))" }} />
          ))}
          {[100, 0].map((v) => (
            <text key={v} x={PAD.left - 6} y={y(v) + 3.5} textAnchor="end" fontSize={10.5} style={{ fill: "rgb(var(--muted))" }}>{v}%</text>
          ))}
          {t.cycle && (
            <line x1={x(t.start)} y1={y(100)} x2={x(t.end)} y2={y(0)} strokeWidth={1.25} strokeDasharray="4 4" style={{ stroke: "rgb(var(--accent) / 0.55)" }}>
              <title>平均用量参考线：从周期开始匀速用到重置时为 0，仅供对照，不是预测</title>
            </line>
          )}
          <path d={line} fill="none" strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" style={{ stroke: "rgb(var(--accent))" }} />
          {t.line.length > 0 && (
            <circle cx={x(t.line[t.line.length - 1].at)} cy={y(t.line[t.line.length - 1].remaining)} r={3.5} strokeWidth={2} style={{ fill: "rgb(var(--accent))", stroke: "rgb(var(--card))" }} />
          )}
          <text x={PAD.left} y={LINE_H - 2} fontSize={10.5} style={{ fill: "rgb(var(--muted))" }}>{time(t.start, withDate)}</text>
          <text x={width - PAD.right} y={LINE_H - 2} textAnchor="end" fontSize={10.5} style={{ fill: "rgb(var(--muted))" }}>{t.cycle ? time(t.end, withDate) : "现在"}</text>
        </svg>
      </div>

      <div className="border-t border-line-2 pt-3">
        <div className="mb-1 flex items-baseline justify-between text-[12px]">
          <span className="font-medium text-fg-2">每日额度用量</span>
          <span className="text-[11px] text-muted">最近 14 天</span>
        </div>
        <svg width={width} height={BAR_H + 18} role="img" aria-label="最近 14 天每日额度用量" onPointerLeave={() => setHoverDay(null)}>
          <line x1={PAD.left} x2={width - PAD.right} y1={BAR_H} y2={BAR_H} strokeWidth={1} style={{ stroke: "rgb(var(--line))" }} />
          {d.days.map((day, i) => {
            const cx = PAD.left + step * (i + 0.5);
            const h = day.used === null ? 0 : Math.max(2, (day.used / maxUse) * (BAR_H - 6));
            return (
              <g key={day.key} onPointerEnter={() => setHoverDay(i)}>
                <rect x={cx - step / 2} y={0} width={step} height={BAR_H} fill="transparent" />
                {day.used !== null && (
                  <rect x={cx - barW / 2} y={BAR_H - h} width={barW} height={h} rx={Math.min(3, barW / 2)}
                    style={{ fill: day.used > 0 ? `rgb(var(--accent) / ${hoverDay === i ? 1 : 0.75})` : "rgb(var(--muted) / 0.6)" }}>
                    <title>{`${day.key}：${pct(day.used)}`}</title>
                  </rect>
                )}
              </g>
            );
          })}
          <text x={PAD.left} y={BAR_H + 14} fontSize={10.5} style={{ fill: "rgb(var(--muted))" }}>{shortDay(d.days[0].key)}</text>
          <text x={width - PAD.right} y={BAR_H + 14} textAnchor="end" fontSize={10.5} style={{ fill: "rgb(var(--muted))" }}>{shortDay(d.days[d.days.length - 1].key)}</text>
        </svg>
        <div className="mt-1 text-[13px]">
          {hoverDay === null ? "今日" : shortDay(shownDay.key)} ·{" "}
          <span className="font-mono font-semibold tabular-nums">{shownDay.used === null ? "未测得" : pct(shownDay.used)}</span>
        </div>
      </div>

      <div className="space-y-0.5 border-t border-line-2 pt-2 text-[11.5px] text-muted">
        {t.resetsAt !== null && <div>{t.resetsAt > Date.now() ? "重置于" : "已于"} {time(t.resetsAt, true)}{t.resetsAt > Date.now() ? "" : " 重置，等待新的读数"}</div>}
        {d.unallocated >= 0.5 && <div>另有 {pct(d.unallocated)} 未能分配至具体日期（两次读数跨了日期）</div>}
        <div>用量是两次读数之间观察到的已用百分比增长，不是 token 数；没有读数的日子留空。</div>
      </div>
    </div>
  );
}

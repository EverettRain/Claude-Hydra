import { Box, ChartLine, ChevronDown, Loader2, RefreshCw } from "lucide-react";
import { hasContainers } from "@/components/AppShell";
import { QuotaTrend } from "@/components/QuotaTrend";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Heatmap, type Metric } from "@/components/Heatmap";
import { Group, Page, PageHeader } from "@/components/PageHeader";
import { Tag } from "@/components/Tag";
import { TrendChart } from "@/components/TrendChart";
import { Button } from "@/components/ui/button";
import { api, errorText, type PlanLimit, type PlanLimits, type QuotaHistory, type Tokens, type UsageDashboard } from "@/lib/api";
import { byDay, compact, daily, duration, highlights, money, sumDays, totalOf } from "@/lib/usage";
import { cn, relativeTime } from "@/lib/utils";

const RANGES = [7, 30, 90] as const;
const MAX_MODELS = 6;
// The last dashboard of this session: shown at once when the page is opened again.
let lastDashboard: UsageDashboard | null = null;

function LimitMeter({ label, value }: { label: string; value: PlanLimit }) {
  // Past its reset the window has started over; the recorded share no longer applies.
  const reset = value.resetsAt !== null && value.resetsAt <= Date.now();
  const used = reset ? 0 : value.used;
  const tone = used >= 90 ? "bg-danger" : used >= 75 ? "bg-warn" : "bg-fg";
  const when = value.resetsAt === null ? "" : reset ? "已重置" : `重置 ${new Date(value.resetsAt).toLocaleString("zh-CN", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })}`;
  return (
    <div className="min-w-0 flex-1">
      <div className="flex items-baseline justify-between gap-2">
        <span className="text-[11.5px] text-muted">{label}已用</span>
        <span className="font-mono text-[12px] tabular-nums">{reset ? "—" : `${used.toFixed(0)}%`}</span>
      </div>
      <div className="mt-1.5 h-1 overflow-hidden rounded-full bg-pill">
        <div className={cn("h-full rounded-full", tone)} style={{ width: `${Math.max(used, 1.5)}%` }} />
      </div>
      <div className="mt-1 h-4 text-[10.5px] text-faint">{when}</div>
    </div>
  );
}

const TREND_WINDOWS = ["five_hour", "seven_day"];

/**
 * The plan's 5-hour and weekly limits as the container's own Claude Code last saw them: its status
 * line logs what Claude Code hands it, so they are as fresh as the account's latest session.
 */
function Limits({ id, value }: { id: string; value: PlanLimits | null }) {
  const [open, setOpen] = useState(false);
  const [history, setHistory] = useState<QuotaHistory | null>(null);
  const [historyError, setHistoryError] = useState("");
  // Reload whenever the dashboard brings a newer reading, so the trend ends at it.
  useEffect(() => {
    if (!open) return;
    api.quotaHistory(id).then((h) => { setHistory(h); setHistoryError(""); }, (e) => setHistoryError(errorText(e)));
  }, [open, id, value?.recordedAt]);
  if (!value || (!value.fiveHour && !value.sevenDay)) {
    return <p className="mt-2 pl-10 text-[11.5px] text-faint">在容器中使用 Claude Code 后，这里显示 5 小时与本周额度。</p>;
  }
  return (
    <div className="mt-3 pl-10">
      <div className="grid grid-cols-2 gap-x-6">
        {value.fiveHour ? <LimitMeter label="5 小时" value={value.fiveHour} /> : <span />}
        {value.sevenDay ? <LimitMeter label="本周" value={value.sevenDay} /> : <span />}
      </div>
      <div className="flex items-center gap-3 text-[11.5px] text-muted">
        <button onClick={() => setOpen((o) => !o)} className="flex items-center gap-1 hover:text-fg active:!scale-100" aria-expanded={open}>
          <ChevronDown className={cn("h-3.5 w-3.5 transition-transform", open && "rotate-180")} />
          剩余额度趋势
        </button>
        <span className="text-faint">记录于 {relativeTime(value.recordedAt)}</span>
      </div>
      {open && (
        <div className="mt-2 rounded-xl border border-line-2 p-3">
          {historyError ? <p className="text-[12px] text-danger">{historyError}</p>
            : history ? <QuotaTrend history={history} order={TREND_WINDOWS} />
            : <p className="text-[12px] text-muted">正在读取…</p>}
        </div>
      )}
    </div>
  );
}

function Segmented<T extends string | number>({ value, options, onChange, label }: { value: T; options: [T, string][]; onChange: (v: T) => void; label: string }) {
  return (
    <div role="radiogroup" aria-label={label} className="flex min-w-0 flex-wrap gap-0.5 rounded-lg bg-pill p-0.5">
      {options.map(([v, text]) => (
        <button
          key={String(v)}
          role="radio"
          aria-checked={value === v}
          onClick={() => onChange(v)}
          className={cn("h-6 max-w-[160px] truncate rounded-md px-2.5 text-[12px] active:!scale-100", value === v ? "bg-card font-medium text-fg shadow-edge dark:bg-white/[.12]" : "text-muted hover:text-fg")}
        >
          {text}
        </button>
      ))}
    </div>
  );
}

function Stat({ label, value, sub, title }: { label: string; value: string; sub?: string; title?: string }) {
  return (
    <div className="min-w-0 bg-card px-4 py-3" title={title}>
      <div className="truncate font-mono text-[19px] font-semibold tabular-nums tracking-[-0.01em]">{value}</div>
      <div className="mt-0.5 truncate text-[11.5px] text-muted">{label}</div>
      {sub && <div className="truncate text-[10.5px] text-muted">{sub}</div>}
    </div>
  );
}

interface BarRow { key: string; label: string; value: number | null }

/** Share of a total as thin bars; `null` values (e.g. unpriced models) are listed without a bar. */
function Bars({ title, rows, metric }: { title: string; rows: BarRow[]; metric: Metric }) {
  const total = rows.reduce((t, r) => t + (r.value ?? 0), 0);
  const fmt = metric === "tokens" ? compact : money;
  return (
    <div className="min-w-0 flex-1">
      <div className="mb-2 text-[12.5px] font-medium text-fg-2">{title}</div>
      <div className="space-y-2">
        {rows.map((r) => {
          const pct = r.value !== null && total > 0 ? (r.value / total) * 100 : null;
          return (
            <div key={r.key} className="grid grid-cols-[minmax(0,6.5rem)_1fr_3.75rem_3rem] items-center gap-2.5 text-[12px]">
              <span className="truncate" title={r.label}>{r.label}</span>
              <div className="h-1.5 overflow-hidden rounded-full bg-pill">
                {pct !== null && <div className="h-full rounded-full bg-accent" style={{ width: `${Math.max(pct, 1)}%` }} />}
              </div>
              <span className="text-right font-mono tabular-nums">{r.value === null ? "未定价" : fmt(r.value)}</span>
              <span className="text-right font-mono tabular-nums text-muted">{pct === null ? "—" : `${pct.toFixed(1)}%`}</span>
            </div>
          );
        })}
        {!rows.length && <p className="text-[12px] text-muted">暂无记录</p>}
      </div>
    </div>
  );
}

const valueOf = (t: Tokens, cost: number | null, metric: Metric) => (metric === "tokens" ? totalOf(t) : cost);
const shortModel = (m: string) => m.replace(/^claude-/, "");

function ContainerDashboard() {
  const [data, setData] = useState<UsageDashboard | null>(lastDashboard);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [tab, setTab] = useState<"overview" | "trend">("overview");
  const [metric, setMetric] = useState<Metric>("tokens");
  const [range, setRange] = useState<number>(30);
  const [series, setSeries] = useState<string>("all");
  const panel = useRef<HTMLElement>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const fresh = await api.usageDashboard();
      lastDashboard = fresh;
      setData(fresh);
      setError("");
    } catch (e) {
      setError(errorText(e));
    } finally {
      setLoading(false);
    }
  }, []);
  // Show what is known at once (this session, else the copy saved on disk), then rebuild.
  useEffect(() => {
    if (!lastDashboard) {
      api.usageDashboardCached().then((cached) => { if (cached) setData((d) => d ?? cached); }, () => { /* no cache */ });
    }
    void load();
  }, [load]);

  const usages = useMemo(() => data?.containers.map((c) => c.usage) ?? [], [data]);
  const days = useMemo(() => byDay(usages), [usages]);
  const hl = useMemo(() => highlights(usages), [usages]);
  const chartDays = useMemo(() => {
    const picked = series === "all" ? usages : data?.containers.filter((c) => c.id === series).map((c) => c.usage) ?? [];
    return daily(picked, range);
  }, [data, usages, series, range]);

  if (error && !data) return <div className="selectable rounded-xl bg-danger-soft px-4 py-3 text-[13px] text-danger">{error}</div>;
  if (!data) return hasContainers ? <p className="px-1 text-[13px] text-muted">正在读取会话记录…</p> : null;
  if (!data.containers.length) {
    return hasContainers ? (
      <div className="rounded-xl border border-dashed border-line px-4 py-8 text-center text-[12.5px] text-muted">
        新建容器账号并在其中使用 Claude Code 后，这里会显示 token 消耗、成本与活动热力图。
      </div>
    ) : null;
  }

  const total = data.total;
  const topModel = total.models[0];
  const models = total.models.slice(0, MAX_MODELS).map((m): BarRow => ({ key: m.model, label: shortModel(m.model), value: valueOf(m.tokens, m.cost, metric) }));
  const rest = total.models.slice(MAX_MODELS);
  if (rest.length) {
    const tokens = rest.reduce((t, m) => t + totalOf(m.tokens), 0);
    const cost = rest.reduce((t, m) => t + (m.cost ?? 0), 0);
    models.push({ key: "other", label: `其他 ${rest.length} 个`, value: metric === "tokens" ? tokens : cost });
  }
  const byContainer = data.containers.map((c): BarRow => ({ key: c.id, label: c.name, value: valueOf(c.usage.total, c.usage.cost, metric) }));
  const metricOptions: [Metric, string][] = [["tokens", "Tokens"], ["cost", "成本"]];

  return (
    <>
      <section ref={panel} className="list-card">
        <div className="flex items-center justify-between gap-3 px-4 pt-3">
          <Segmented label="视图" value={tab} onChange={setTab} options={[["overview", "总览"], ["trend", "趋势"]]} />
          <div className="flex items-center gap-1.5">
            <span className="hidden text-[11px] text-muted min-[640px]:inline" title={error || undefined}>
              {loading ? "更新中…" : error ? "更新失败，显示的是上次结果" : `更新于 ${relativeTime(data.updatedAt)}`}
            </span>
            <Segmented label="指标" value={metric} onChange={setMetric} options={metricOptions} />
            <Button variant="ghost" size="iconSm" aria-label="重新读取" title="重新读取会话记录" disabled={loading} onClick={() => load()}>
              {loading ? <Loader2 className="animate-spin" /> : <RefreshCw />}
            </Button>
          </div>
        </div>

        {tab === "overview" ? (
          <>
            <div className="m-4 grid grid-cols-2 gap-px overflow-hidden rounded-xl border border-line-2 bg-line-2 min-[620px]:grid-cols-4">
              <Stat label="总 Token" value={compact(totalOf(total.total))} sub={`输出 ${compact(total.total.output)}`} />
              <Stat label="API 等价成本" value={money(total.cost)} sub={total.unpriced ? `另有 ${compact(total.unpriced)} 未定价` : "按官方 API 单价估算"} />
              <Stat label="活跃天数" value={String(hl.activeDays)} />
              <Stat label="最长连续" value={`${hl.longestStreak} 天`} />
              <Stat label="活跃时间" value={duration(total.activeMs)} sub="间隔超过 5 分钟不计" />
              <Stat label="峰值单日" value={hl.peak ? compact(totalOf(hl.peak.tokens)) : "—"} sub={hl.peak?.key} />
              <Stat label="常用模型" value={topModel ? shortModel(topModel.model) : "—"} title={topModel?.model} />
              <Stat label="模型响应" value={compact(total.responses)} />
            </div>
            <div className="px-4">
              <div className="mb-2 text-[12.5px] font-medium text-fg-2">{metric === "tokens" ? "Token 活动" : "成本活动（API 等价）"}</div>
              <Heatmap days={days} metric={metric} />
            </div>
            <div className="flex flex-col gap-6 px-4 pb-4 pt-5 min-[720px]:flex-row">
              <Bars title="按模型" rows={models} metric={metric} />
              <Bars title="按容器" rows={byContainer} metric={metric} />
            </div>
          </>
        ) : (
          <div className="px-4 pb-4 pt-3">
            <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
              <Segmented label="查看的容器" value={series} onChange={setSeries} options={[["all", "全部容器"], ...data.containers.map((c): [string, string] => [c.id, c.name])]} />
              <Segmented label="时间范围" value={range} onChange={setRange} options={RANGES.map((r): [number, string] => [r, `${r} 天`])} />
            </div>
            <TrendChart days={chartDays} metric={metric} label={series === "all" ? "全部容器" : data.containers.find((c) => c.id === series)?.name ?? "容器"} />
          </div>
        )}
      </section>

      <Group label="容器账号">
        {data.containers.map((c) => {
          const month = daily([c.usage], 30);
          const plan = c.account.plan;
          return (
            <div key={c.id} className="px-4 py-3">
              <div className="flex items-center gap-3">
                <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[8px] bg-pill text-fg-2"><Box className="h-3.5 w-3.5" /></span>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <span className="truncate text-[13.5px] font-medium">{c.name}</span>
                    {plan && <Tag title="订阅">{plan}</Tag>}
                    {!c.account.signedIn && <Tag tone="warn">未登录</Tag>}
                    {c.blocked && (c.blockedSoft
                      ? <Tag tone="warn" title={c.blocked}>核验中断网</Tag>
                      : <Tag tone="danger" title={c.blocked}>已断网</Tag>)}
                  </div>
                  <div className="truncate text-[11px] text-muted">
                    {[c.account.email, `今天 ${compact(totalOf(month[month.length - 1].tokens))}`, `近 30 天 ${compact(totalOf(sumDays(month)))} · ${money(month.reduce((t, d) => t + d.cost, 0))}`].filter(Boolean).join(" · ")}
                  </div>
                </div>
                <Button variant="ghost" size="iconSm" aria-label="查看趋势" title="查看趋势" onClick={() => { setSeries(c.id); setTab("trend"); panel.current?.scrollIntoView({ behavior: "smooth", block: "start" }); }}>
                  <ChartLine />
                </Button>
              </div>
              <Limits id={c.id} value={c.limits} />
            </div>
          );
        })}
      </Group>
      <p className="px-1 text-[11.5px] leading-relaxed text-muted">
        Token 与成本读取自各容器 Claude 数据文件夹中的会话记录，不联网；每次读取后按小时汇总存入 Hydra 的用量历史，Claude Code 清理旧会话记录（默认 30 天）后，已记录的消耗仍然保留，但从未被 Hydra 读取过的旧记录无法补回。成本是按官方 API 单价估算的等价金额，不是订阅的实际扣费。额度来自容器中 Claude Code 的状态栏：Claude Code 把 5 小时与本周额度交给状态栏，Hydra 读取状态栏留下的记录，不使用账号的登录凭据；所以只在使用 Claude Code 时更新，按模型的额度请在 Claude Code 中用 /usage 查看。
      </p>
    </>
  );
}

export function DashboardPage() {
  const [, tick] = useState(0);
  useEffect(() => {
    const timer = window.setInterval(() => tick((value) => value + 1), 30_000);
    return () => window.clearInterval(timer);
  }, []);
  return (
    <Page>
      <PageHeader title="仪表板" subtitle="Claude Code 容器的 token 消耗与成本。" />
      <div className="space-y-5 px-5">
        <ContainerDashboard />
      </div>
    </Page>
  );
}

import { ProfileAvatar } from "@/components/ProfileAvatar";
import { listen } from "@tauri-apps/api/event";
import { Box, ChartLine, ChevronDown, Loader2, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Heatmap, type Metric } from "@/components/Heatmap";
import { QuotaTrend } from "@/components/QuotaTrend";
import { Group, Page, PageHeader } from "@/components/PageHeader";
import { Tag } from "@/components/Tag";
import { TrendChart } from "@/components/TrendChart";
import { Button } from "@/components/ui/button";
import { api, errorText, type Profile, type Quota, type QuotaHistory, type QuotaWindow, type Tokens, type UsageDashboard } from "@/lib/api";
import { latestQuota, windowLabel } from "@/lib/quotaHistory";
import { byDay, compact, daily, duration, highlights, money, planLabel, sumDays, totalOf } from "@/lib/usage";
import { cn, relativeTime } from "@/lib/utils";

// Cache contains only quota results, never credentials; it survives page navigation.
const cache = new Map<string, Quota>();
const RETRY_MS = 60_000;
const attempts = new Map<string, number>();
const RANGES = [7, 30, 90] as const;
const MAX_MODELS = 6;
const isMac = navigator.userAgent.includes("Mac");
// The last dashboard of this session: shown at once when the page is opened again.
let lastDashboard: UsageDashboard | null = null;

function Meter({ value }: { value: QuotaWindow }) {
  const title = `${windowLabel(value.id)}已用`;
  const date = value.resetsAt ? new Date(value.resetsAt) : null;
  const pct = value.utilization;
  const tone = pct >= 90 ? "bg-danger" : pct >= 75 ? "bg-warn" : "bg-fg";
  return (
    <div className="min-w-0 flex-1">
      <div className="flex items-baseline justify-between gap-2">
        <span className="text-[11.5px] text-muted">{title}</span>
        <span className="font-mono text-[12px] tabular-nums text-fg">{`${pct.toFixed(1)}%`}</span>
      </div>
      <div className="mt-1.5 h-1 overflow-hidden rounded-full bg-pill">
        <div className={cn("h-full rounded-full transition-[width] duration-500 ease-soft", tone)} style={{ width: `${Math.max(pct, 1.5)}%` }} />
      </div>
      <div className="mt-1 h-4 text-[10.5px] text-faint">{date && !Number.isNaN(date.getTime()) ? `重置 ${date.toLocaleString()}` : ""}</div>
    </div>
  );
}

/** Quota refresh shared by desktop and container accounts: cached, at most once a minute each. */
function useQuota() {
  const [values, setValues] = useState<Record<string, Quota>>(() => Object.fromEntries(cache));
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<string | null>(null);
  // Readings taken in the background (see quota_history.rs) update the page as they arrive.
  useEffect(() => {
    const un = listen<[string, Quota]>("quota-sampled", ({ payload: [id, q] }) => {
      cache.set(id, q);
      setValues((old) => ({ ...old, [id]: q }));
    });
    return () => { void un.then((f) => f()); };
  }, []);
  const refresh = async (id: string, fetch: () => Promise<Quota>) => {
    const since = Date.now() - (attempts.get(id) ?? 0);
    if (since < RETRY_MS) {
      setErrors((old) => ({ ...old, [id]: `请在 ${Math.ceil((RETRY_MS - since) / 1000)} 秒后再次刷新。` }));
      return;
    }
    attempts.set(id, Date.now());
    setBusy(id);
    setErrors((old) => ({ ...old, [id]: "" }));
    try {
      const result = await fetch();
      cache.set(id, result);
      setValues((old) => ({ ...old, [id]: result }));
    } catch (e) {
      setErrors((old) => ({ ...old, [id]: errorText(e) }));
    } finally {
      setBusy(null);
    }
  };
  // Accounts without a reading this session show their last recorded one until refreshed.
  const seed = useCallback((ids: string[]) => {
    for (const id of ids.filter((id) => !cache.has(id))) {
      api.quotaHistory(id).then((h) => {
        const q = latestQuota(h);
        if (q && !cache.has(id)) {
          cache.set(id, q);
          setValues((old) => (old[id] ? old : { ...old, [id]: q }));
        }
      }, () => { /* no history yet */ });
    }
  }, []);
  return { values, errors, busy, refresh, seed };
}

function QuotaBlock({ id, q, error }: { id: string; q?: Quota; error?: string }) {
  const [open, setOpen] = useState(false);
  const [history, setHistory] = useState<QuotaHistory | null>(null);
  const [historyError, setHistoryError] = useState("");
  // Reload whenever a new reading arrives, so the trend ends at the latest one.
  useEffect(() => {
    if (!open) return;
    api.quotaHistory(id).then((h) => { setHistory(h); setHistoryError(""); }, (e) => setHistoryError(errorText(e)));
  }, [open, id, q?.fetchedAt]);
  return (
    <>
      {q && (
        <div className="mt-3 grid grid-cols-2 gap-x-6 pl-10">
          {q.windows.map((w) => <Meter key={w.id} value={w} />)}
        </div>
      )}
      {q && (
        <div className="pl-10">
          <div className="flex items-center gap-3 text-[11.5px] text-muted">
            <button onClick={() => setOpen((o) => !o)} className="flex items-center gap-1 hover:text-fg active:!scale-100" aria-expanded={open}>
              <ChevronDown className={cn("h-3.5 w-3.5 transition-transform", open && "rotate-180")} />
              剩余额度趋势
            </button>
            <span className="text-faint">读于 {relativeTime(q.fetchedAt)}</span>
          </div>
          {open && (
            <div className="mt-2 rounded-xl border border-line-2 p-3">
              {historyError ? <p className="text-[12px] text-danger">{historyError}</p>
                : history ? <QuotaTrend history={history} order={q.windows.map((w) => w.id)} />
                : <p className="text-[12px] text-muted">正在读取…</p>}
            </div>
          )}
        </div>
      )}
      {error && (
        <p className="selectable mt-2 whitespace-pre-line break-all pl-10 text-[12px] text-danger">
          {error}
          {q && " 当前显示的是上次结果。"}
        </p>
      )}
    </>
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

function ContainerDashboard({ quota }: { quota: ReturnType<typeof useQuota> }) {
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
  const { seed } = quota;
  useEffect(() => { if (data) seed(data.containers.map((c) => c.id)); }, [data, seed]);

  const usages = useMemo(() => data?.containers.map((c) => c.usage) ?? [], [data]);
  const days = useMemo(() => byDay(usages), [usages]);
  const hl = useMemo(() => highlights(usages), [usages]);
  const chartDays = useMemo(() => {
    const picked = series === "all" ? usages : data?.containers.filter((c) => c.id === series).map((c) => c.usage) ?? [];
    return daily(picked, range);
  }, [data, usages, series, range]);

  if (error && !data) return <div className="selectable rounded-xl bg-danger-soft px-4 py-3 text-[13px] text-danger">{error}</div>;
  if (!data) return isMac ? <p className="px-1 text-[13px] text-muted">正在读取会话记录…</p> : null;
  if (!data.containers.length) {
    return isMac ? (
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
          const plan = planLabel(c.account);
          return (
            <div key={c.id} className="px-4 py-3">
              <div className="flex items-center gap-3">
                <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[8px] bg-pill text-fg-2"><Box className="h-3.5 w-3.5" /></span>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <span className="truncate text-[13.5px] font-medium">{c.name}</span>
                    {plan && <Tag title={`订阅：${c.account.tier ?? plan}`}>{plan}</Tag>}
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
                <Button variant="outline" size="sm" disabled={quota.busy !== null || !c.account.signedIn} onClick={() => quota.refresh(c.id, () => api.containerQuota(c.id))}>
                  {quota.busy === c.id ? <Loader2 className="animate-spin" /> : <RefreshCw />}
                  {quota.busy === c.id ? "读取中" : "额度"}
                </Button>
              </div>
              <QuotaBlock id={c.id} q={quota.values[c.id]} error={quota.errors[c.id]} />
            </div>
          );
        })}
      </Group>
      <p className="px-1 text-[11.5px] leading-relaxed text-muted">
        Token 与成本读取自各容器 Claude 数据文件夹中的会话记录，不联网；每次读取后按小时汇总存入 Hydra 的用量历史，Claude Code 清理旧会话记录（默认 30 天）后，已记录的消耗仍然保留，但从未被 Hydra 读取过的旧记录无法补回。成本是按官方 API 单价估算的等价金额，不是订阅的实际扣费。额度经该账号的代理节点查询；手动刷新过一次的账号，Hydra 运行期间每 10 分钟在后台再记录一次，用于剩余额度趋势（可在设置中关闭）。
      </p>
    </>
  );
}

export function DashboardPage({ profiles }: { profiles: Profile[] | null }) {
  const quota = useQuota();
  const { seed } = quota;
  useEffect(() => { if (profiles) seed(profiles.map((p) => p.id)); }, [profiles, seed]);
  const [, tick] = useState(0);
  useEffect(() => {
    const timer = window.setInterval(() => tick((value) => value + 1), 30_000);
    return () => window.clearInterval(timer);
  }, []);
  return (
    <Page>
      <PageHeader title="仪表板" subtitle="Claude Code 的 token 消耗、成本与订阅额度。" />
      <div className="space-y-5 px-5">
        <ContainerDashboard quota={quota} />

        {/* Container-only setups have no Claude desktop and therefore no desktop accounts. */}
        {profiles && profiles.length > 0 && (
          <Group label="桌面账号额度">
            {profiles.map((profile) => {
              const q = quota.values[profile.id];
              return (
                <div key={profile.id} className="px-4 py-3">
                  <div className="flex items-center gap-3">
                    <ProfileAvatar badge={profile.badge} isDefault={profile.isDefault} color={profile.color} className="h-7 w-7 shrink-0" />
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-[13.5px] font-medium">{profile.name}</div>
                      <div className="text-[11px] text-muted">{q ? `${relativeTime(new Date(q.fetchedAt).getTime())}更新` : "尚未读取"}</div>
                    </div>
                    <Button variant="outline" size="sm" disabled={quota.busy !== null} onClick={() => quota.refresh(profile.id, () => api.profileQuota(profile.id))}>
                      {quota.busy === profile.id ? <Loader2 className="animate-spin" /> : <RefreshCw />}
                      {quota.busy === profile.id ? "读取中" : "刷新"}
                    </Button>
                  </div>
                  <QuotaBlock id={profile.id} q={q} error={quota.errors[profile.id]} />
                </div>
              );
            })}
          </Group>
        )}
      </div>
    </Page>
  );
}

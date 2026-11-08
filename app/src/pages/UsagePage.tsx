import { Loader2, RefreshCw } from "lucide-react";
import { useState } from "react";
import { Page, PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { api, errorText, type Profile, type Quota, type QuotaWindow } from "@/lib/api";
import { PALETTE, badgeLetter, cn } from "@/lib/utils";

// Cache contains only quota results, never credentials; it survives page navigation.
const cache = new Map<string, Quota>();
const RETRY_MS = 60_000;
const attempts = new Map<string, number>();

function Meter({ title, value }: { title: string; value: QuotaWindow | null }) {
  const date = value?.resetsAt ? new Date(value.resetsAt) : null;
  const pct = value?.utilization ?? 0;
  const tone = pct >= 90 ? "bg-danger" : pct >= 75 ? "bg-warn" : "bg-fg";
  return (
    <div className="min-w-0 flex-1">
      <div className="flex items-baseline justify-between gap-2">
        <span className="text-[11.5px] text-muted">{title}</span>
        <span className="font-mono text-[12px] tabular-nums text-fg">{value ? `${pct.toFixed(1)}%` : "—"}</span>
      </div>
      <div className="mt-1.5 h-1 overflow-hidden rounded-full bg-pill">
        {value && <div className={cn("h-full rounded-full transition-[width] duration-500 ease-soft", tone)} style={{ width: `${Math.max(pct, 1.5)}%` }} />}
      </div>
      <div className="mt-1 h-4 text-[10.5px] text-faint">{date && !Number.isNaN(date.getTime()) ? `重置 ${date.toLocaleString()}` : ""}</div>
    </div>
  );
}

export function UsagePage({ profiles }: { profiles: Profile[] | null }) {
  const [values, setValues] = useState<Record<string, Quota>>(() => Object.fromEntries(cache));
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const refresh = async (id: string) => {
    const since = Date.now() - (attempts.get(id) ?? 0);
    if (since < RETRY_MS) {
      setErrors((old) => ({ ...old, [id]: `请在 ${Math.ceil((RETRY_MS - since) / 1000)} 秒后再次刷新。` }));
      return;
    }
    attempts.set(id, Date.now());
    setBusy(id);
    setErrors((old) => ({ ...old, [id]: "" }));
    try {
      const result = await api.profileQuota(id);
      cache.set(id, result);
      setValues((old) => ({ ...old, [id]: result }));
    } catch (e) {
      setErrors((old) => ({ ...old, [id]: errorText(e) }));
    } finally {
      setBusy(null);
    }
  };
  return (
    <Page>
      <PageHeader title="用量" subtitle="按账号读取订阅额度，含同账号其他设备的用量；点击刷新才会联网。" />
      <div className="space-y-3 px-5">
        {profiles === null && <p className="px-1 text-[13px] text-muted">正在读取账号…</p>}
        {profiles && profiles.length > 0 && (
          <div className="list-card">
            {profiles.map((profile) => {
              const q = values[profile.id];
              return (
                <div key={profile.id} className="px-4 py-3">
                  <div className="flex items-center gap-3">
                    <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-[12px] font-semibold text-white" style={{ background: PALETTE[profile.color % PALETTE.length] }}>
                      {badgeLetter(profile.name)}
                    </div>
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-[13.5px] font-medium">{profile.name}</div>
                      <div className="text-[11px] text-muted">{q ? `更新于 ${new Date(q.fetchedAt).toLocaleString()}` : "尚未读取"}</div>
                    </div>
                    <Button variant="outline" size="sm" disabled={busy !== null} onClick={() => refresh(profile.id)}>
                      {busy === profile.id ? <Loader2 className="animate-spin" /> : <RefreshCw />}
                      {busy === profile.id ? "读取中" : "刷新"}
                    </Button>
                  </div>
                  {q && (
                    <div className="mt-3 flex gap-6 pl-10">
                      <Meter title="当前时段" value={q.fiveHour} />
                      <Meter title="每周" value={q.sevenDay} />
                    </div>
                  )}
                  {errors[profile.id] && (
                    <p className="mt-2 pl-10 text-[12px] text-danger">
                      {errors[profile.id]}
                      {q && " 当前显示的是上次结果。"}
                    </p>
                  )}
                </div>
              );
            })}
          </div>
        )}
        <p className="px-1 text-[11.5px] leading-[18px] text-muted">
          此接口未公开，可能随 Claude 更新变化。macOS 首次读取时系统会询问是否允许访问钥匙串中的“Claude Safe Storage”，请选择“允许”；凭据只在本机解密并用于这次查询。
        </p>
      </div>
    </Page>
  );
}

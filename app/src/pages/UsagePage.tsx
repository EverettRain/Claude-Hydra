import { useState } from "react";
import { PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { api, errorText, type Profile, type Quota, type QuotaWindow } from "@/lib/api";

// Cache contains only quota results, never credentials; it survives page navigation.
const cache = new Map<string, Quota>();
const RETRY_MS = 60_000;
const attempts = new Map<string, number>();
function Meter({ title, value }: { title: string; value: QuotaWindow | null }) {
  const date = value?.resetsAt ? new Date(value.resetsAt) : null;
  return <div className="space-y-1">
    <div className="flex justify-between text-sm"><span>{title}</span><span>{value ? `${value.utilization.toFixed(1)}%` : "暂无数据"}</span></div>
    {value && <div className="h-2 overflow-hidden rounded-full bg-muted"><div className="h-full rounded-full bg-blue-500" style={{ width: `${value.utilization}%` }} /></div>}
    {date && !Number.isNaN(date.getTime()) && <p className="text-xs text-muted-foreground">重置：{date.toLocaleString()}</p>}
  </div>;
}
export function UsagePage({ profiles }: { profiles: Profile[] | null }) {
  const [values, setValues] = useState<Record<string, Quota>>(() => Object.fromEntries(cache));
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const refresh = async (id: string) => {
    const since = Date.now() - (attempts.get(id) ?? 0);
    if (since < RETRY_MS) { setErrors(old => ({ ...old, [id]: `请在 ${Math.ceil((RETRY_MS - since) / 1000)} 秒后再次刷新。` })); return; }
    attempts.set(id, Date.now()); setBusy(id); setErrors(old => ({ ...old, [id]: "" }));
    try { const result = await api.profileQuota(id); cache.set(id, result); setValues(old => ({ ...old, [id]: result })); }
    catch (e) { setErrors(old => ({ ...old, [id]: errorText(e) })); }
    finally { setBusy(null); }
  };
  return <div className="pb-8"><PageHeader title="用量统计" /><div className="space-y-4 px-6">
    <p className="text-sm text-muted-foreground">按账号读取 Claude 返回的订阅额度，包含共用额度的其他设备和产品。点击刷新才会联网；此接口未公开，可能随 Claude 更新变化。macOS 首次读取时系统会询问是否允许访问钥匙串中的「Claude Safe Storage」，请选择「允许」；凭据只在本机解密并用于这次查询。</p>
    {profiles === null && <p className="text-sm text-muted-foreground">正在读取账号…</p>}
    {profiles?.map(profile => { const q = values[profile.id]; return <section key={profile.id} className="space-y-3 rounded-lg border border-border bg-card p-4">
      <div className="flex items-center justify-between gap-3"><h2 className="font-semibold">{profile.name}</h2><Button variant="outline" size="sm" disabled={busy !== null} onClick={() => refresh(profile.id)}>{busy === profile.id ? "读取中…" : "刷新额度"}</Button></div>
      {q ? <><Meter title="当前时段" value={q.fiveHour} /><Meter title="每周" value={q.sevenDay} /><p className="text-xs text-muted-foreground">更新于 {new Date(q.fetchedAt).toLocaleString()}</p></> : <p className="text-sm text-muted-foreground">尚未读取。</p>}
      {errors[profile.id] && <p className="text-sm text-red-500">{errors[profile.id]}{q && " 当前显示的是上次结果。"}</p>}
    </section>; })}
  </div></div>;
}

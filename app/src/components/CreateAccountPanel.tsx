import { ProxyField } from "./ProxyField";
import { Globe2, Loader2, Network, ShieldCheck, Shuffle, X } from "lucide-react";
import { useEffect, useState } from "react";
import { api, errorText, type Profile } from "@/lib/api";
import { Button } from "./ui/button";
import { Input, Label } from "./ui/input";

const locales = [
  ["", "跟随系统"], ["en-US", "英语（美国）"], ["en-GB", "英语（英国）"],
  ["zh-CN", "简体中文"], ["zh-TW", "繁體中文"], ["ja-JP", "日语"], ["ko-KR", "韩语"],
];

export function CreateAccountPanel({ open, onClose, onCreated }: { open: boolean; onClose: () => void; onCreated: (profile: Profile) => Promise<void> }) {
  const [name, setName] = useState("");
  const [proxy, setProxy] = useState("");
  const [locale, setLocale] = useState("");
  const [loading, setLoading] = useState(false);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setName(""); setProxy(""); setLocale(""); setError(""); setReady(false);
    api.nextAccountName().then((n) => { if (!cancelled) { setName(n); setReady(true); } }).catch((e) => { if (!cancelled) setError(errorText(e)); });
    return () => { cancelled = true; };
  }, [open]);

  useEffect(() => {
    if (!open || loading) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, loading, onClose]);

  if (!open) return null;

  const create = async () => {
    setLoading(true); setError("");
    try { await onCreated(await api.createConfiguredAccount(name, proxy, locale)); }
    catch (e) { setError(errorText(e)); }
    finally { setLoading(false); }
  };
  const randomize = () => {
    const array = new Uint32Array(1);
    crypto.getRandomValues(array);
    setLocale(locales[1 + (array[0] % (locales.length - 1))][0]);
  };

  return (
    <div className="fixed inset-0 z-50 flex justify-end">
      <div className="absolute inset-0 bg-black/40 dark:bg-black/60" onClick={() => !loading && onClose()} />
      <div role="dialog" aria-modal="true" aria-labelledby="create-account-title" className="relative flex h-full w-full max-w-[590px] flex-col border-l border-border bg-card shadow-2xl">
        <div className="flex items-center justify-between border-b border-border px-6 py-5">
          <div><h2 id="create-account-title" className="text-lg font-semibold">创建账号窗口</h2><p className="mt-1 text-xs text-muted-foreground">为这个窗口设置名称和网络配置</p></div>
          <button aria-label="关闭" disabled={loading} onClick={onClose} className="rounded-md p-2 text-muted-foreground hover:bg-muted"><X className="h-4 w-4" /></button>
        </div>
        <div className="min-h-0 flex-1 space-y-7 overflow-y-auto px-6 py-6">
          <section className="space-y-4">
            <div className="flex items-center gap-2 text-sm font-semibold text-blue-600 dark:text-blue-400"><span className="flex h-7 w-7 items-center justify-center rounded-full bg-blue-500/10"><ShieldCheck className="h-4 w-4" /></span>基础设置</div>
            <div className="space-y-2"><Label htmlFor="new-account-name">窗口名称</Label><Input id="new-account-name" value={name} maxLength={40} onChange={(e) => setName(e.target.value)} placeholder="请输入窗口名称" autoFocus /></div>
            <p className="text-xs leading-5 text-muted-foreground">每个账号使用独立数据目录，登录状态分别保存。</p>
          </section>
          <section className="space-y-4 border-t border-border pt-6">
            <div className="flex items-center gap-2 text-sm font-semibold text-blue-600 dark:text-blue-400"><span className="flex h-7 w-7 items-center justify-center rounded-full bg-blue-500/10"><Network className="h-4 w-4" /></span>网络设置</div>
            <ProxyField value={proxy} onChange={setProxy} />
          </section>
          <section className="space-y-4 border-t border-border pt-6">
            <div className="flex items-center gap-2 text-sm font-semibold text-blue-600 dark:text-blue-400"><span className="flex h-7 w-7 items-center justify-center rounded-full bg-blue-500/10"><Globe2 className="h-4 w-4" /></span>环境设置</div>
            <div className="space-y-2"><Label htmlFor="new-account-locale">窗口语言</Label><select id="new-account-locale" value={locale} onChange={(e) => setLocale(e.target.value)} className="h-10 w-full rounded-md border border-input bg-background px-3 text-sm">{locales.map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select></div>
            <p className="text-xs leading-5 text-muted-foreground">随机生成仅选择窗口语言，不会修改设备、浏览器或网络指纹，也不能保证账号不被关联。代理地址需自行填写。</p>
          </section>
          {error && <p role="alert" className="selectable rounded-md bg-red-500/10 p-3 text-sm text-red-600 dark:text-red-400">{error}</p>}
        </div>
        <div className="flex flex-wrap items-center justify-between gap-3 border-t border-border px-6 py-4">
          <Button type="button" variant="outline" onClick={randomize} disabled={loading}><Shuffle className="h-4 w-4" />一键随机生成指纹配置</Button>
          <div className="flex gap-2"><Button type="button" variant="outline" onClick={onClose} disabled={loading}>取消</Button><Button type="button" onClick={create} disabled={loading || !ready || !name.trim()}>{loading && <Loader2 className="h-4 w-4 animate-spin" />}确定</Button></div>
        </div>
      </div>
    </div>
  );
}

import { Loader2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { api, errorText, type Profile } from "@/lib/api";
import { EnvironmentPicker } from "./EnvironmentPicker";
import { Button } from "./ui/button";
import { Input } from "./ui/input";

export function CreateAccountPanel({ open, onClose, onCreated }: { open: boolean; onClose: () => void; onCreated: (profile: Profile) => Promise<void> }) {
  const [name, setName] = useState("");
  const [environment, setEnvironment] = useState("");
  const [loading, setLoading] = useState(false);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setName(""); setEnvironment(""); setError(""); setReady(false);
    api.nextAccountName().then((n) => { if (!cancelled) { setName(n); setReady(true); } }).catch((e) => { if (!cancelled) setError(errorText(e)); });
    return () => { cancelled = true; };
  }, [open]);

  useEffect(() => {
    if (!open || loading) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape" && !document.querySelector("[data-dialog-layer]")) onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, loading, onClose]);

  if (!open) return null;

  const create = async () => {
    setLoading(true); setError("");
    try { await onCreated(await api.createConfiguredAccount(name, environment)); }
    catch (e) { setError(errorText(e)); }
    finally { setLoading(false); }
  };

  return (
    <div className="fixed inset-0 z-50 flex justify-end">
      <div className="absolute inset-0 animate-fade-in bg-black/20 backdrop-blur-[2px] dark:bg-black/45" onClick={() => !loading && onClose()} />
      <div role="dialog" aria-modal="true" aria-labelledby="create-account-title" className="relative flex h-full w-full max-w-[480px] animate-slide-in flex-col bg-bg shadow-pop">
        <div data-tauri-drag-region className="flex h-12 shrink-0 items-center justify-between px-5">
          <h2 id="create-account-title" className="text-[15px] font-semibold tracking-[-0.01em]">添加账号</h2>
          <button aria-label="关闭" disabled={loading} onClick={onClose} className="flex h-[26px] w-[26px] items-center justify-center rounded-[7px] text-muted hover:bg-pill hover:text-fg">
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-5 pb-5">
          <section>
            <div className="group-label">名称</div>
            <div className="list-card">
              <div className="px-4 py-2.5">
                <Input id="new-account-name" aria-label="窗口名称" value={name} maxLength={40} onChange={(e) => setName(e.target.value)} placeholder="请输入窗口名称" autoFocus />
              </div>
            </div>
            <p className="mt-1.5 px-1 text-[11.5px] leading-[18px] text-muted">每个账号使用独立的数据目录与登录态。</p>
          </section>

          <section>
            <div className="group-label">环境配置</div>
            <EnvironmentPicker value={environment} onChange={setEnvironment} disabled={loading} />
            <p className="mt-1.5 px-1 text-[11.5px] leading-[18px] text-muted">代理节点、时区、窗口语言与启动参数都在环境配置里；也可以在左侧“环境配置”中统一管理。</p>
          </section>

          {error && <p role="alert" className="selectable rounded-xl bg-danger-soft px-4 py-3 text-[12.5px] text-danger">{error}</p>}
        </div>

        <div className="flex shrink-0 items-center justify-end gap-2 border-t border-line-2 px-5 py-3">
          <Button variant="outline" onClick={onClose} disabled={loading}>取消</Button>
          <Button onClick={create} disabled={loading || !ready || !name.trim()}>
            {loading && <Loader2 className="animate-spin" />}
            创建并打开
          </Button>
        </div>
      </div>
    </div>
  );
}

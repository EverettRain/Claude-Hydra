import { getIdentifier } from "@tauri-apps/api/app";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { ArrowDownToLine } from "lucide-react";
import { useEffect, useState } from "react";
import { Dialog } from "./ui/dialog";
import { Button } from "./ui/button";

export function UpdateNotice() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    let active = true, pending = false;
    let found: Update | null = null;
    async function poll() {
      if (pending || found) return;
      pending = true;
      try {
        if (await getIdentifier() !== "local.claude-hydra") return;
        const result = await check({ timeout: 15000 });
        if (!active) { await result?.close(); return; }
        found = result;
        setUpdate(result);
      } catch { /* Offline or no published release; retry on the next interval. */ }
      finally { pending = false; }
    }
    void poll();
    const timer = window.setInterval(poll, 60 * 60 * 1000);
    return () => { active = false; window.clearInterval(timer); void found?.close(); };
  }, []);
  if (!update) return null;
  const install = async () => {
    setBusy(true); setError("");
    try { await update.downloadAndInstall(); }
    catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  };
  return <>
    <Button size="sm" variant="ghost" onClick={() => setOpen(true)}><ArrowDownToLine />新版 v{update.version}</Button>
    <Dialog open={open} title={`更新到 v${update.version}`} description={`当前版本 v${update.currentVersion}。安装时将退出 Hydra。`}
      onClose={() => { if (!busy) setOpen(false); }}
      footer={<><Button variant="ghost" disabled={busy} onClick={() => setOpen(false)}>稍后</Button>
        <Button disabled={busy} onClick={install}>{busy ? "正在更新…" : "下载并安装"}</Button></>}>
      <p className="selectable whitespace-pre-wrap text-sm">{update.body}</p>
      {error && <p role="alert" className="text-sm text-danger">{error}</p>}
    </Dialog>
  </>;
}

import { getIdentifier, getVersion } from "@tauri-apps/api/app";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { ArrowDownToLine, Check, Loader2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { api, errorText, type ReleaseNote } from "@/lib/api";
import { isNewer } from "@/lib/releaseNotes";
import { ReleaseNotes } from "./ReleaseNotes";
import { Button } from "./ui/button";
import { Dialog } from "./ui/dialog";

const DISMISSED = "hydra.update.dismissed";
const LAST_SEEN = "hydra.lastSeenVersion";
const store = {
  get: (k: string) => { try { return localStorage.getItem(k); } catch { return null; } },
  set: (k: string, v: string) => { try { localStorage.setItem(k, v); } catch { /* storage unavailable */ } },
};

type Notes = { loading: boolean; list: ReleaseNote[] };
type Phase = { kind: "idle" } | { kind: "downloading"; done: number; total: number } | { kind: "ready" } | { kind: "error"; message: string };

/** Notes for releases in (after, upto]; falls back to what the update feed carried. */
async function loadNotes(after: string, upto: string, fallback?: ReleaseNote): Promise<ReleaseNote[]> {
  try {
    const list = await api.releaseNotes(after, upto);
    if (list.length) return list;
  } catch { /* offline or rate-limited: use the feed's notes */ }
  return fallback ? [fallback] : [];
}

const mb = (n: number) => (n / 1048576).toFixed(1);

/**
 * Update indicator in the title bar plus its dialogs:
 * - a quiet brand pill when a newer release exists (hover × hides it until a newer one ships);
 * - "更新到 vX": every release's notes since the running version, download progress, restart;
 * - after an update, "vX 更新了什么" once on first launch.
 */
export function UpdateNotice() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [current, setCurrent] = useState("");
  const [dismissed, setDismissed] = useState(() => store.get(DISMISSED));
  const [open, setOpen] = useState<"ask" | "whatsnew" | null>(null);
  const [notes, setNotes] = useState<Notes>({ loading: false, list: [] });
  const [since, setSince] = useState("");
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });

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
      } catch { /* Offline or no published release; retry on the next interval or focus. */ }
      finally { pending = false; }
    }
    // First launch after an update: show what changed, once.
    void (async () => {
      const version = await getVersion();
      if (!active) return;
      setCurrent(version);
      const last = store.get(LAST_SEEN);
      store.set(LAST_SEEN, version);
      if (last && isNewer(version, last)) {
        setSince(last);
        setOpen("whatsnew");
        setNotes({ loading: true, list: [] });
        const list = await loadNotes(last, version);
        if (active) setNotes({ loading: false, list });
      }
    })();
    void poll();
    const timer = window.setInterval(poll, 60 * 60 * 1000);
    const onFocus = () => void poll();
    window.addEventListener("focus", onFocus);
    return () => { active = false; window.clearInterval(timer); window.removeEventListener("focus", onFocus); void found?.close(); };
  }, []);

  const ask = async () => {
    if (!update) return;
    setSince(update.currentVersion);
    setOpen("ask");
    if (phase.kind === "error") setPhase({ kind: "idle" });
    setNotes({ loading: true, list: [] });
    const fallback = { version: update.version, date: update.date ?? "", body: update.body ?? "" };
    setNotes({ loading: false, list: await loadNotes(update.currentVersion, update.version, fallback) });
  };
  const install = async () => {
    if (!update) return;
    setPhase({ kind: "downloading", done: 0, total: 0 });
    try {
      let done = 0, total = 0;
      await update.downloadAndInstall((e) => {
        if (e.event === "Started") total = e.data.contentLength ?? 0;
        else if (e.event === "Progress") done += e.data.chunkLength;
        setPhase({ kind: "downloading", done, total });
      });
      setPhase({ kind: "ready" });
    } catch (e) { setPhase({ kind: "error", message: errorText(e) }); }
  };
  const dismiss = () => { if (update) { store.set(DISMISSED, update.version); setDismissed(update.version); } };
  const busy = phase.kind === "downloading";

  const pill = update && dismissed !== update.version && (
    <span className="group relative inline-flex animate-update-in overflow-visible">
      <button onClick={() => void ask()} title={`Claude-Hydra v${update.version} 已发布，点击查看更新内容`}
        className="inline-flex h-[22px] items-center gap-1 whitespace-nowrap rounded-full bg-brand-soft pl-[7px] pr-[9px] text-[11.5px] font-semibold text-brand shadow-[inset_0_0_0_0.5px_rgb(var(--brand)/0.35)] transition-colors hover:bg-brand hover:text-brand-fg active:scale-[.96]">
        <ArrowDownToLine className="h-3 w-3" strokeWidth={2.4} />v{update.version} 可更新
      </button>
      <button onClick={dismiss} aria-label="隐藏此版本的更新提示" title="隐藏，直到有更新的版本"
        className="absolute -right-1 -top-1 hidden h-3.5 w-3.5 place-items-center rounded-full bg-fg-2 text-card shadow-edge group-hover:grid">
        <X className="h-2.5 w-2.5" strokeWidth={3} />
      </button>
    </span>
  );

  const progress = phase.kind === "downloading" ? (phase.total ? phase.done / phase.total : 0) : phase.kind === "ready" ? 1 : 0;
  const status = phase.kind !== "idle" && open === "ask" && (
    <div className="mt-4 space-y-1.5 rounded-xl bg-card-2 px-3.5 py-3 shadow-edge">
      {phase.kind === "error" ? <p role="alert" className="text-[12px] text-danger">更新失败：{phase.message}</p> : <>
        <div className="h-1.5 overflow-hidden rounded-full bg-pill">
          <div className="h-full rounded-full bg-brand transition-[width] duration-200" style={{ width: `${Math.round(progress * 100)}%` }} />
        </div>
        <p className="flex items-center gap-1.5 text-[11.5px] text-muted">
          {phase.kind === "ready"
            ? <><Check className="h-3.5 w-3.5 text-ok" />已下载并安装，重启 Hydra 后生效。</>
            : phase.total
              ? <>正在下载 {mb(phase.done)} / {mb(phase.total)} MB · {Math.round(progress * 100)}%</>
              : <><Loader2 className="h-3 w-3 animate-spin" />正在准备下载…</>}
        </p>
      </>}
    </div>
  );

  const body = notes.loading
    ? <div className="flex items-center gap-2 py-6 text-[12.5px] text-muted"><Loader2 className="h-4 w-4 animate-spin" />正在读取更新说明…</div>
    : <ReleaseNotes releases={notes.list.length ? notes.list : [{ version: open === "ask" ? update?.version ?? current : current, date: "", body: "" }]} />;

  return <>
    {pill}
    <Dialog open={open === "ask" && !!update} onClose={() => { if (!busy) setOpen(null); }} className="max-w-[540px]"
      title={`更新到 v${update?.version ?? ""}`}
      description={<>自 v{since} 以来的变化。{phase.kind !== "ready" && "安装时会退出 Hydra，账号的代理中继会短暂中断。"}</>}
      footer={phase.kind === "ready"
        ? <><Button variant="ghost" onClick={() => setOpen(null)}>稍后</Button><Button onClick={() => void api.restartApp()}>重启 Hydra</Button></>
        : <><Button variant="ghost" disabled={busy} onClick={() => setOpen(null)}>稍后</Button>
            <Button disabled={busy} onClick={() => void install()}>
              {busy ? <><Loader2 className="animate-spin" />正在更新…</> : phase.kind === "error" ? "重试" : "下载并更新"}
            </Button></>}>
      {body}
      {status}
    </Dialog>
    <Dialog open={open === "whatsnew"} onClose={() => setOpen(null)} className="max-w-[540px]"
      title={`v${current} 更新了什么`} description={`自 v${since} 以来的变化。`}
      footer={<Button onClick={() => setOpen(null)}>知道了</Button>}>
      {body}
    </Dialog>
  </>;
}

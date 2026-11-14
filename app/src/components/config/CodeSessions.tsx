import { open as pickFile, save as pickSave } from "@tauri-apps/plugin-dialog";
import { ChevronDown, Copy, Download, Folder, MoveRight, Search, Trash2, Upload } from "lucide-react";
import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type ClaudeCodeSession, type ClaudeFolder } from "@/lib/api";
import { formatSize, formatTime, leaf } from "./shared";
import { TransferDialog, type Transfer } from "./TransferDialog";

// Loaded on first use, with the Markdown parser it brings.
const ChatView = lazy(() => import("./ChatView").then((m) => ({ default: m.ChatView })));

const NO_CWD = "未记录工作目录";

function scrollParent(el: HTMLElement | null): HTMLElement | null {
  for (let p = el?.parentElement; p; p = p.parentElement) {
    if (/(auto|scroll)/.test(getComputedStyle(p).overflowY)) return p;
  }
  return null;
}

/**
 * The Claude Code sessions of one data folder, grouped by working folder. An opened session is
 * drawn into `chatHost`, which the page shows in place of itself; the list stays mounted behind.
 */
export function CodeSessions({ folder, folders, chatHost, onReading }: {
  folder: ClaudeFolder; folders: ClaudeFolder[]; chatHost: HTMLElement | null; onReading: (open: boolean) => void;
}) {
  const toast = useToast();
  const [rows, setRows] = useState<ClaudeCodeSession[] | null>(null);
  const [error, setError] = useState("");
  const [revision, refresh] = useState(0);
  const [query, setQuery] = useState("");
  const [collapsed, setCollapsed] = useState(new Set<string>());
  const [picked, setPicked] = useState(new Set<string>());
  const [reading, setReading] = useState<ClaudeCodeSession | null>(null);
  const [transfer, setTransfer] = useState<Transfer | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [busy, setBusy] = useState(false);
  // Only a slow read (an index still being built) is worth explaining.
  const [slow, setSlow] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  // Where the list was scrolled when a session was opened, to return there.
  const listScroll = useRef(0);
  useEffect(() => { onReading(!!reading); }, [reading, onReading]);
  useEffect(() => () => onReading(false), [onReading]);

  useEffect(() => {
    let current = true;
    setError(""); setSlow(false);
    const timer = setTimeout(() => { if (current) setSlow(true); }, 1000);
    api.codeSessions(folder.id).then((r) => { if (current) { setRows(r); setPicked((old) => new Set([...old].filter((id) => r.some((s) => s.id === id)))); } })
      .catch((e) => { if (current) setError(errorText(e)); }).finally(() => clearTimeout(timer));
    return () => { current = false; clearTimeout(timer); };
  }, [folder.id, revision]);

  const needle = query.trim().toLocaleLowerCase();
  const groups = useMemo(() => {
    const map = new Map<string, ClaudeCodeSession[]>();
    for (const s of rows ?? []) {
      if (needle && !`${s.title} ${s.cwd ?? ""}`.toLocaleLowerCase().includes(needle)) continue;
      const key = s.cwd ?? NO_CWD;
      map.set(key, [...(map.get(key) ?? []), s]);
    }
    return [...map];
  }, [rows, needle]);

  const toggle = (ids: string[]) => setPicked((old) => {
    const next = new Set(old), off = ids.every((id) => next.has(id));
    for (const id of ids) { if (off) next.delete(id); else next.add(id); }
    return next;
  });
  const run = async (work: () => Promise<void>) => {
    setBusy(true);
    try { await work(); } catch (e) { toast(errorText(e), "error"); } finally { setBusy(false); refresh((n) => n + 1); }
  };
  const ids = [...picked];
  const exportSessions = (ids: string[]) => run(async () => {
    const path = await pickSave({ title: "导出会话", defaultPath: `claude-sessions-${new Date().toISOString().slice(0, 10)}.zip`, filters: [{ name: "Zip", extensions: ["zip"] }] });
    if (!path) return;
    const n = await api.exportCodeSessions(folder.id, ids, path);
    toast(`已导出 ${n} 个会话`);
  });
  const openSession = (s: ClaudeCodeSession) => {
    const scroller = scrollParent(root.current);
    listScroll.current = scroller?.scrollTop ?? 0;
    setReading(s);
    scroller?.scrollTo({ top: 0 });
  };
  const resume = (s: ClaudeCodeSession) => {
    const container = folder.id.slice("container:".length);
    api.openContainer(container, { cwd: s.cwd!, resume: s.id.split("/")[1] })
      .then(() => toast("已在终端中打开，出口检测通过后会继续这个会话"), (e) => toast(errorText(e), "error"));
  };
  const closeSession = () => {
    setReading(null);
    requestAnimationFrame(() => scrollParent(root.current)?.scrollTo({ top: listScroll.current }));
  };
  const importFile = async () => {
    try {
      const path = await pickFile({ title: "导入会话", filters: [{ name: "Hydra 会话导出", extensions: ["zip"] }] });
      if (typeof path === "string") setTransfer({ type: "import", path });
    } catch (e) { toast(errorText(e), "error"); }
  };

  return (
    <div ref={root}>
      {reading && chatHost && createPortal(
        <Suspense fallback={<p className="py-6 text-center text-sm text-muted">正在打开…</p>}>
          <ChatView folder={folder.id} session={reading} onBack={closeSession}
            onCopy={() => setTransfer({ type: "copy", ids: [reading.id] })} onMove={() => setTransfer({ type: "move", ids: [reading.id] })}
            onExport={() => exportSessions([reading.id])}
            onResume={folder.kind === "container" && reading.cwd ? () => resume(reading) : undefined} />
        </Suspense>,
        chatHost,
      )}
      {/* Kept mounted while a session is open, so going back finds the list as it was. */}
      <div className={reading ? "hidden" : "space-y-3"}>
      <div className="flex flex-wrap items-center gap-2">
        <label className="relative min-w-40 flex-1">
          <Search className="absolute left-2.5 top-2 h-4 w-4 text-muted" />
          <input className="field pl-8" aria-label="搜索会话" placeholder="搜索标题或工作目录" value={query} onChange={(e) => setQuery(e.target.value)} />
        </label>
        <Button size="sm" variant="outline" disabled={busy} onClick={importFile}><Upload />导入</Button>
      </div>
      {picked.size > 0 && (
        <div className="flex flex-wrap items-center gap-1.5 rounded-xl bg-card-2 px-3 py-2 text-[13px]">
          <span className="mr-auto">已选 {picked.size} 个</span>
          <Button size="sm" variant="ghost" disabled={busy} onClick={() => setPicked(new Set())}>取消选择</Button>
          <Button size="sm" variant="outline" disabled={busy} onClick={() => setTransfer({ type: "copy", ids })}><Copy />复制到…</Button>
          <Button size="sm" variant="outline" disabled={busy} onClick={() => setTransfer({ type: "move", ids })}><MoveRight />移动到…</Button>
          <Button size="sm" variant="outline" disabled={busy} onClick={() => exportSessions(ids)}><Download />导出</Button>
          <Button size="sm" variant="outline" disabled={busy} onClick={() => setConfirmDelete(true)}><Trash2 />删除</Button>
        </div>
      )}
      {error && <p role="alert" className="text-sm text-danger">{error} <button className="underline" onClick={() => refresh((n) => n + 1)}>重试</button></p>}
      {!error && (!rows || !groups.length) && (
        <p className="py-6 text-center text-sm text-muted">{!rows ? (slow ? "正在建立会话索引…" : "正在读取会话…") : needle ? "没有匹配的会话。" : "这里还没有 Claude Code 会话。"}</p>
      )}
      {groups.map(([path, list]) => {
        const folded = collapsed.has(path) && !needle;
        const all = list.map((s) => s.id);
        const every = all.every((id) => picked.has(id));
        return (
          <section key={path} className="list-card">
            <div className="flex items-center gap-2 bg-card-2 px-3 py-2">
              <input type="checkbox" aria-label={`选择 ${path} 下的全部会话`} checked={every} disabled={busy} onChange={() => toggle(all)} />
              <button className="flex min-w-0 flex-1 items-center gap-2 text-left" aria-expanded={!folded} onClick={() => setCollapsed((old) => {
                const next = new Set(old); if (!next.delete(path)) next.add(path); return next;
              })}>
                <ChevronDown className={`h-3 w-3 shrink-0 ${folded ? "-rotate-90" : ""}`} /><Folder className="h-4 w-4 shrink-0 text-muted" />
                <span className="min-w-0"><strong className="block truncate text-sm">{leaf(path)}</strong>
                  <span className="block truncate text-xs text-muted" title={path}>{path !== leaf(path) ? `${path} · ` : ""}{list.length} 个会话</span></span>
              </button>
            </div>
            {!folded && list.map((s) => (
              <div key={s.id} className="flex items-center gap-2.5 px-3 py-2">
                <input type="checkbox" aria-label={`选择 ${s.title}`} checked={picked.has(s.id)} disabled={busy} onChange={() => toggle([s.id])} />
                <button className="min-w-0 flex-1 text-left" onClick={() => openSession(s)}>
                  <span className="block truncate text-[13px] hover:underline" title={s.title}>{s.title}</span>
                  <small className="block truncate text-muted">{formatTime(s.lastActivityAt)} · {s.model ?? "模型未知"} · {s.turns} 轮 · {formatSize(s.size)}</small>
                </button>
              </div>
            ))}
          </section>
        );
      })}
      </div>
      <TransferDialog folder={folder} folders={folders} transfer={transfer} onClose={() => setTransfer(null)}
        onDone={(message) => {
          toast(message); setPicked(new Set()); refresh((n) => n + 1);
          // A moved session is no longer here to read.
          if (transfer?.type === "move" && reading && transfer.ids.includes(reading.id)) closeSession();
        }} />
      <Dialog open={confirmDelete} onClose={() => { if (!busy) setConfirmDelete(false); }} title={`删除 ${picked.size} 个会话？`}
        description="会话会移到这个位置的回收站，可以在「回收站」里恢复。"
        footer={<><Button variant="ghost" disabled={busy} onClick={() => setConfirmDelete(false)}>取消</Button>
          <Button variant="destructive" disabled={busy} onClick={() => run(async () => {
            const n = await api.trashCodeSessions(folder.id, ids);
            setConfirmDelete(false); setPicked(new Set()); toast(`已将 ${n} 个会话移到回收站`);
          })}>{busy ? "处理中…" : "删除"}</Button></>} />
    </div>
  );
}

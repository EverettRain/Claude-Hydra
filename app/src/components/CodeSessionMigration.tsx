import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { api, errorText, type Profile, type SessionPreview } from "@/lib/api";

const field = "w-full rounded-md border border-border bg-background p-2";

/** Copies Desktop Claude Code session indexes from one account to another (never moves or overwrites). */
export function CodeSessionMigration({ profiles }: { profiles: Profile[] | null }) {
  const [source, setSource] = useState("");
  const [target, setTarget] = useState("");
  const [preview, setPreview] = useState<SessionPreview | null>(null);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState("");

  const load = async () => {
    setPreview(null); setPicked(new Set()); setStatus("");
    if (!source || !target || source === target) return;
    setBusy(true);
    try { setPreview(await api.sessionPreview(source, target)); }
    catch (e) { setStatus(errorText(e)); }
    finally { setBusy(false); }
  };
  useEffect(() => { void load(); }, [source, target]);

  const copyable = preview?.sessions.filter(s => !s.inTarget) ?? [];
  const toggle = (id: string) => setPicked(old => { const next = new Set(old); next.has(id) ? next.delete(id) : next.add(id); return next; });
  const copy = async () => {
    setBusy(true); setStatus("");
    try {
      const r = await api.copySessions(source, target, [...picked]);
      setPreview(await api.sessionPreview(source, target)); setPicked(new Set());
      setStatus(`已复制 ${r.copied} 个，跳过已存在 ${r.skipped} 个，失败 ${r.failed} 个。请打开目标账号确认会话列表。`);
    } catch (e) { setStatus(errorText(e)); }
    finally { setBusy(false); }
  };

  const options = (exclude: string) => profiles?.filter(p => p.id !== exclude).map(p => <option key={p.id} value={p.id}>{p.name}{p.pid ? "（运行中）" : ""}</option>);
  const blocked = !preview || !!preview.targetProblem || preview.running;

  return <section className="space-y-3 rounded-lg border border-border bg-card p-4">
    <h2 className="font-semibold">Claude Code 会话迁移</h2>
    <p className="text-sm text-muted-foreground">复制 Claude Desktop 内本地 Claude Code 会话的索引到另一个账号。原会话保留不动，同名会话不会被覆盖；对话记录仍留在本机原位置。普通聊天和云端会话不在此列。</p>
    <div className="grid gap-3 sm:grid-cols-2">
      <label className="text-sm">来源账号<select className={field} value={source} disabled={busy} onChange={e => setSource(e.target.value)}>
        <option value="">选择账号</option>{options(target)}</select></label>
      <label className="text-sm">目标账号<select className={field} value={target} disabled={busy} onChange={e => setTarget(e.target.value)}>
        <option value="">选择账号</option>{options(source)}</select></label>
    </div>
    {preview && <>
      {preview.running && <p className="text-sm text-amber-600 dark:text-amber-400">来源或目标账号正在运行。请先自行退出这两个账号，再点「重新读取」。Hydra 不会自动关闭它们。</p>}
      {preview.targetProblem && <p className="text-sm text-amber-600 dark:text-amber-400">{preview.targetProblem}</p>}
      {preview.sessions.length === 0 ? <p className="text-sm text-muted-foreground">来源账号没有本地 Claude Code 会话。</p> :
        <div className="max-h-72 space-y-1 overflow-y-auto rounded-md border border-border p-2">
          {preview.sessions.map(s => <label key={s.id} className="flex items-start gap-2 rounded p-1 text-sm hover:bg-muted/60">
            <input type="checkbox" className="mt-1" disabled={s.inTarget || busy} checked={picked.has(s.id)} onChange={() => toggle(s.id)} />
            <span className="min-w-0">
              <span className="block truncate">{s.title}{s.inTarget && <span className="text-muted-foreground"> · 目标已存在</span>}</span>
              <span className="block truncate text-xs text-muted-foreground">
                {s.cwd ?? "未记录工作目录"}{s.cwd && !s.cwdExists && " · 目录不存在"}
                {s.lastActivityAt ? ` · ${new Date(s.lastActivityAt).toLocaleString()}` : ""}
              </span>
            </span>
          </label>)}
        </div>}
      <div className="flex flex-wrap gap-2">
        <Button variant="outline" size="sm" disabled={busy || !copyable.length} onClick={() => setPicked(picked.size === copyable.length ? new Set() : new Set(copyable.map(s => s.id)))}>{picked.size === copyable.length && copyable.length ? "取消全选" : "全选可复制"}</Button>
        <Button variant="outline" size="sm" disabled={busy} onClick={() => void load()}>重新读取</Button>
        <Button size="sm" disabled={busy || blocked || !picked.size} onClick={copy}>{busy ? "处理中…" : `复制 ${picked.size} 个会话`}</Button>
      </div>
    </>}
    {status && <p role="status" className="text-sm">{status}</p>}
  </section>;
}

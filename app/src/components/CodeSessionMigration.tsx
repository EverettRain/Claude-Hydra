import { useEffect, useState } from "react";
import { Group } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { AccountPicker } from "@/components/AccountPicker";
import { api, errorText, type Profile, type SessionPreview } from "@/lib/api";
import { cn } from "@/lib/utils";

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

  const copyable = preview?.sessions.filter((s) => !s.inTarget) ?? [];
  const toggle = (id: string) => setPicked((old) => { const next = new Set(old); next.has(id) ? next.delete(id) : next.add(id); return next; });
  const copy = async () => {
    setBusy(true); setStatus("");
    try {
      const r = await api.copySessions(source, target, [...picked]);
      setPreview(await api.sessionPreview(source, target)); setPicked(new Set());
      setStatus(`已复制 ${r.copied} 个，跳过已存在 ${r.skipped} 个，失败 ${r.failed} 个。请打开目标账号确认会话列表。`);
    } catch (e) { setStatus(errorText(e)); }
    finally { setBusy(false); }
  };

  const blocked = !preview || !!preview.targetProblem || preview.running;
  const allPicked = copyable.length > 0 && picked.size === copyable.length;

  return (
    <Group label="Claude Code 会话">
      <div className="space-y-3 px-4 py-3">
        <p className="text-[12px] leading-[18px] text-muted">复制 Claude Desktop 内本地 Claude Code 会话的索引。原会话保留不动，同名会话不会被覆盖，对话记录仍留在本机原位置；普通聊天与云端会话不在此列。</p>
      </div>
      <div className="grid grid-cols-[56px_1fr] items-start gap-3 px-4 py-3">
        <span className="pt-1.5 text-[12.5px] text-fg-2">从</span>
        <AccountPicker label="来源账号" profiles={profiles} value={source} exclude={target} disabled={busy} onChange={setSource} />
        <span className="pt-1.5 text-[12.5px] text-fg-2">复制到</span>
        <AccountPicker label="目标账号" profiles={profiles} value={target} exclude={source} disabled={busy} onChange={setTarget} />
      </div>
      {!preview && !status && <p className="px-4 py-2.5 text-[12px] text-muted">{source && target ? "正在读取会话…" : "选择来源和目标账号后，这里会列出可复制的会话。"}</p>}

      {preview && (
        <>
          {(preview.running || preview.targetProblem) && (
            <div className="space-y-1 bg-warn-soft/60 px-4 py-2.5 text-[12px] text-warn">
              {preview.running && <p>来源或目标账号正在运行。请先自行退出这两个账号，再点“重新读取”。Hydra 不会自动关闭它们。</p>}
              {preview.targetProblem && <p>{preview.targetProblem}</p>}
            </div>
          )}
          {preview.sessions.length === 0 ? (
            <p className="px-4 py-3 text-[12.5px] text-muted">来源账号没有本地 Claude Code 会话。</p>
          ) : (
            <div className="max-h-72 overflow-y-auto py-1">
              {preview.sessions.map((s) => (
                <label key={s.id} className={cn("flex cursor-pointer items-start gap-2.5 px-4 py-1.5 hover:bg-card-2", s.inTarget && "cursor-default opacity-55")}>
                  <input type="checkbox" className="mt-[3px]" disabled={s.inTarget || busy} checked={picked.has(s.id)} onChange={() => toggle(s.id)} />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[13px]">
                      {s.title}
                      {s.inTarget && <span className="text-muted"> · 目标已存在</span>}
                    </span>
                    <span className="block truncate text-[11px] text-muted">
                      {s.cwd ?? "未记录工作目录"}
                      {s.cwd && !s.cwdExists && <span className="text-warn"> · 目录不存在</span>}
                      {s.lastActivityAt ? ` · ${new Date(s.lastActivityAt).toLocaleString()}` : ""}
                    </span>
                  </span>
                </label>
              ))}
            </div>
          )}
          <div className="flex flex-wrap items-center gap-2 px-4 py-2.5">
            <Button variant="ghost" size="sm" disabled={busy || !copyable.length} onClick={() => setPicked(allPicked ? new Set() : new Set(copyable.map((s) => s.id)))}>
              {allPicked ? "取消全选" : "全选可复制"}
            </Button>
            <Button variant="ghost" size="sm" disabled={busy} onClick={() => void load()}>重新读取</Button>
            <span className="flex-1" />
            <Button size="sm" disabled={busy || blocked || !picked.size} onClick={copy}>{busy ? "处理中…" : `复制 ${picked.size} 个会话`}</Button>
          </div>
        </>
      )}
      {status && <p role="status" className="px-4 py-2.5 text-[12.5px] text-fg-2">{status}</p>}
    </Group>
  );
}

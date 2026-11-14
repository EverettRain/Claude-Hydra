import { useEffect, useState } from "react";
import { Copy, FileText, Trash2 } from "lucide-react";
import { Group } from "@/components/PageHeader";
import { Tag } from "@/components/Tag";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import { Switch } from "@/components/ui/switch";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type ClaudeExtension, type ClaudeFolder, type ClaudePlugin, type McpServer, type TrashedItem } from "@/lib/api";
import { CopyDialog, formatTime, otherFolders, TextEditor } from "./shared";

function useLoad<T>(load: () => Promise<T>, deps: unknown[]) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState("");
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    let current = true;
    setError("");
    load().then((d) => { if (current) setData(d); }).catch((e) => { if (current) setError(errorText(e)); });
    return () => { current = false; };
  }, [...deps, revision]);
  return { data, error, reload: () => setRevision((n) => n + 1) };
}

function Empty({ children }: { children: React.ReactNode }) {
  return <p className="px-4 py-3 text-[12.5px] text-muted">{children}</p>;
}

function settingsError(text: string) {
  try {
    const v = JSON.parse(text);
    return v && typeof v === "object" && !Array.isArray(v) ? null : "settings.json 必须是一个 JSON 对象。";
  } catch (e) {
    return `JSON 格式错误：${(e as Error).message}`;
  }
}

/** `CLAUDE.md` (memory loaded into every session) and `settings.json`. */
export function MemoryAndSettings({ folder }: { folder: ClaudeFolder }) {
  return (
    <div className="space-y-5">
      <Group label="CLAUDE.md · 全局记忆">
        <div className="space-y-2 px-4 py-3">
          <p className="text-[12.5px] text-muted">每个会话开始时都会读取的说明，适合写你的偏好和通用约定。</p>
          <TextEditor folder={folder.id} path="CLAUDE.md" />
        </div>
      </Group>
      <Group label="settings.json · 设置">
        <div className="space-y-2 px-4 py-3">
          <p className="text-[12.5px] text-muted">权限、模型、环境变量、钩子等。保存前会检查 JSON 格式。</p>
          <TextEditor folder={folder.id} path="settings.json" validate={settingsError} />
        </div>
      </Group>
    </div>
  );
}

const extensionGroups = [
  { kind: "skill", label: "技能 · skills", empty: "没有技能。" },
  { kind: "agent", label: "子代理 · agents", empty: "没有子代理。" },
  { kind: "command", label: "斜杠命令 · commands", empty: "没有自定义命令。" },
] as const;

/** Skills, sub-agents and slash commands: read, edit, delete, copy to other locations. */
export function Extensions({ folder, folders }: { folder: ClaudeFolder; folders: ClaudeFolder[] }) {
  const toast = useToast();
  const { data, error, reload } = useLoad(() => api.claudeExtensions(folder.id), [folder.id]);
  const [editing, setEditing] = useState<ClaudeExtension | null>(null);
  const [copying, setCopying] = useState<ClaudeExtension | null>(null);
  const [deleting, setDeleting] = useState<ClaudeExtension | null>(null);
  const [busy, setBusy] = useState(false);
  const remove = async (e: ClaudeExtension) => {
    setBusy(true);
    try { await api.deleteClaudeExtension(folder.id, e.path); toast(`已将「${e.name}」移到回收站`); setDeleting(null); reload(); }
    catch (err) { toast(errorText(err), "error"); } finally { setBusy(false); }
  };
  if (error) return <p role="alert" className="text-sm text-danger">{error}</p>;
  if (!data) return <p className="text-sm text-muted">正在读取…</p>;
  return (
    <div className="space-y-5">
      {extensionGroups.map((g) => {
        const list = data.filter((e) => e.kind === g.kind);
        return (
          <Group key={g.kind} label={`${g.label}（${list.length}）`}>
            {list.length ? list.map((e) => (
              <div key={e.path} className="flex items-center gap-2 px-4 py-2.5">
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[13px] font-medium">{g.kind === "command" ? `/${e.name}` : e.name}</div>
                  <div className="truncate text-[12px] text-muted" title={e.description ?? e.path}>{e.description ?? e.path}</div>
                </div>
                <Button size="iconSm" variant="ghost" aria-label={`查看 ${e.name}`} title="查看与编辑" onClick={() => setEditing(e)}><FileText /></Button>
                <Button size="iconSm" variant="ghost" aria-label={`复制 ${e.name}`} title="复制到其他位置" onClick={() => setCopying(e)}><Copy /></Button>
                <Button size="iconSm" variant="ghost" aria-label={`删除 ${e.name}`} title="移到回收站" onClick={() => setDeleting(e)}><Trash2 /></Button>
              </div>
            )) : <Empty>{g.empty}</Empty>}
          </Group>
        );
      })}
      <Dialog open={!!editing} onClose={() => setEditing(null)} title={editing?.name ?? ""} className="max-w-2xl"
        description={<span className="selectable font-mono">{editing?.file}</span>}>
        {editing && <TextEditor folder={folder.id} path={editing.file} rows={18} onSaved={reload} />}
      </Dialog>
      <CopyDialog open={!!copying} title={`复制「${copying?.name ?? ""}」`} folders={otherFolders(folders, folder)} onClose={() => setCopying(null)}
        copy={(targets, overwrite) => api.copyClaudeExtension(folder.id, copying!.path, targets, overwrite)} />
      <Dialog open={!!deleting} onClose={() => { if (!busy) setDeleting(null); }} title={`删除「${deleting?.name ?? ""}」？`}
        description="会移到这个位置的回收站，可以恢复。"
        footer={<><Button variant="ghost" disabled={busy} onClick={() => setDeleting(null)}>取消</Button>
          <Button variant="destructive" disabled={busy} onClick={() => deleting && remove(deleting)}>删除</Button></>} />
    </div>
  );
}

/** MCP servers from `.claude.json` and plugin switches from `settings.json`. */
export function McpAndPlugins({ folder, folders }: { folder: ClaudeFolder; folders: ClaudeFolder[] }) {
  const toast = useToast();
  const servers = useLoad(() => api.mcpServers(folder.id), [folder.id]);
  const plugins = useLoad(() => api.claudePlugins(folder.id), [folder.id]);
  const [viewing, setViewing] = useState<McpServer | null>(null);
  const [copying, setCopying] = useState<McpServer | null>(null);
  const [deleting, setDeleting] = useState<McpServer | null>(null);
  const [busy, setBusy] = useState(false);
  const act = async (work: () => Promise<void>) => {
    setBusy(true);
    try { await work(); } catch (e) { toast(errorText(e), "error"); } finally { setBusy(false); }
  };
  const setPlugin = (p: ClaudePlugin, enabled: boolean) => act(async () => { await api.setClaudePlugin(folder.id, p.id, enabled); plugins.reload(); });
  return (
    <div className="space-y-5">
      <p className="text-[12.5px] text-muted">修改前最好先退出这个位置正在运行的 Claude Code：它会整体重写 <code>.claude.json</code>，可能覆盖这里的改动。</p>
      <Group label="MCP 服务器">
        {servers.error ? <Empty>{servers.error}</Empty> : !servers.data ? <Empty>正在读取…</Empty> : servers.data.length ? servers.data.map((s) => (
          <div key={`${s.project ?? ""}/${s.name}`} className="flex items-center gap-2 px-4 py-2.5">
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-1.5 text-[13px] font-medium"><span className="truncate">{s.name}</span><Tag>{s.transport}</Tag>
                {s.project && <Tag tone="env" title={s.project}>仅项目</Tag>}</div>
              <div className="truncate font-mono text-[11.5px] text-muted" title={s.project ? `${s.project}\n${s.summary}` : s.summary}>{s.summary || "（无命令或地址）"}</div>
            </div>
            <Button size="iconSm" variant="ghost" aria-label={`查看 ${s.name}`} title="查看配置" onClick={() => setViewing(s)}><FileText /></Button>
            {!s.project && <Button size="iconSm" variant="ghost" aria-label={`复制 ${s.name}`} title="复制到其他位置" onClick={() => setCopying(s)}><Copy /></Button>}
            <Button size="iconSm" variant="ghost" aria-label={`删除 ${s.name}`} title="删除" disabled={busy} onClick={() => setDeleting(s)}><Trash2 /></Button>
          </div>
        )) : <Empty>没有配置 MCP 服务器。</Empty>}
      </Group>
      <Group label="插件">
        {plugins.error ? <Empty>{plugins.error}</Empty> : !plugins.data ? <Empty>正在读取…</Empty> : plugins.data.length ? plugins.data.map((p) => (
          <div key={p.id} className="flex items-center gap-3 px-4 py-2.5">
            <div className="min-w-0 flex-1">
              <div className="truncate text-[13px] font-medium">{p.name}</div>
              <div className="truncate text-[12px] text-muted">{[p.marketplace, p.version && `v${p.version}`, !p.installed && "未安装"].filter(Boolean).join(" · ")}</div>
            </div>
            <Switch checked={p.enabled} disabled={busy} label={`启用 ${p.name}`} onCheckedChange={(v) => setPlugin(p, v)} />
          </div>
        )) : <Empty>没有安装插件。在 Claude Code 里用 /plugin 安装。</Empty>}
      </Group>
      <Dialog open={!!viewing} onClose={() => setViewing(null)} title={viewing?.name ?? ""} className="max-w-xl"
        description={viewing?.project ? `仅在 ${viewing.project} 中可用。环境变量和请求头的值已隐藏。` : "所有项目可用。环境变量和请求头的值已隐藏。"}>
        <pre className="selectable whitespace-pre-wrap break-all rounded-lg bg-card-2 p-3 font-mono text-[12px]">{JSON.stringify(viewing?.config, null, 2)}</pre>
      </Dialog>
      <CopyDialog open={!!copying} title={`复制 MCP 服务器「${copying?.name ?? ""}」`} folders={otherFolders(folders, folder)} onClose={() => setCopying(null)}
        copy={(targets, overwrite) => api.copyMcpServer(folder.id, copying!.name, targets, overwrite)} />
      <Dialog open={!!deleting} onClose={() => { if (!busy) setDeleting(null); }} title={`删除 MCP 服务器「${deleting?.name ?? ""}」？`}
        description="会从 .claude.json 中移除这项配置，不进回收站。"
        footer={<><Button variant="ghost" disabled={busy} onClick={() => setDeleting(null)}>取消</Button>
          <Button variant="destructive" disabled={busy} onClick={() => deleting && act(async () => {
            await api.deleteMcpServer(folder.id, deleting.project, deleting.name); setDeleting(null); servers.reload(); toast("已删除");
          })}>删除</Button></>} />
    </div>
  );
}

const trashKind: Record<TrashedItem["kind"], string> = { session: "会话", skill: "技能", agent: "子代理", command: "命令" };

/** Sessions and extensions deleted from this location, kept by Hydra until restored or emptied. */
export function Trash({ folder }: { folder: ClaudeFolder }) {
  const toast = useToast();
  const { data, error, reload } = useLoad(() => api.claudeTrash(folder.id), [folder.id]);
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const act = async (work: () => Promise<void>) => {
    setBusy(true);
    try { await work(); } catch (e) { toast(errorText(e), "error"); } finally { setBusy(false); reload(); }
  };
  return (
    <div className="space-y-3">
      <Group label={`回收站（${data?.length ?? 0}）`} action={<Button size="sm" variant="ghost" disabled={busy || !data?.length} onClick={() => setConfirm(true)}>彻底清空</Button>}>
        {error ? <Empty>{error}</Empty> : !data ? <Empty>正在读取…</Empty> : data.length ? data.map((t) => (
          <div key={t.id} className="flex items-center gap-2 px-4 py-2.5">
            <Tag>{trashKind[t.kind] ?? t.kind}</Tag>
            <div className="min-w-0 flex-1">
              <div className="truncate text-[13px]" title={t.name}>{t.name}</div>
              <div className="truncate text-[12px] text-muted" title={t.detail ?? undefined}>删除于 {formatTime(t.deletedAt)}{t.detail ? ` · ${t.detail}` : ""}</div>
            </div>
            <Button size="sm" variant="outline" disabled={busy} onClick={() => act(async () => { await api.restoreClaudeTrash(folder.id, t.id); toast(`已恢复「${t.name}」`); })}>恢复</Button>
          </div>
        )) : <Empty>回收站是空的。</Empty>}
      </Group>
      <Dialog open={confirm} onClose={() => { if (!busy) setConfirm(false); }} title="彻底清空回收站？"
        description={`将永久删除「${folder.name}」回收站里的全部内容，无法恢复。`}
        footer={<><Button variant="ghost" disabled={busy} onClick={() => setConfirm(false)}>取消</Button>
          <Button variant="destructive" disabled={busy} onClick={() => act(async () => { await api.emptyClaudeTrash(folder.id); setConfirm(false); toast("回收站已清空"); })}>永久删除</Button></>} />
    </div>
  );
}

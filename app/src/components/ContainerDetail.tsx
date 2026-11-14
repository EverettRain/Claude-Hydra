import { ArrowLeft, Box, ChevronRight, Folder, FolderPlus, Loader2, Pencil, Play, Plus, SquareTerminal } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Group } from "@/components/PageHeader";
import { Tag } from "@/components/Tag";
import { Button } from "@/components/ui/button";
import {
  api, errorText, type ClaudeCodeSession, type Container, type ContainerLaunch, type ContainerProject, type ContainerProjects, type ContainerSubfolder,
} from "@/lib/api";
import { cn } from "@/lib/utils";

const leaf = (path: string) => path.split("/").filter(Boolean).pop() || path;
const day = (ms: number | null) => (ms ? new Date(ms).toLocaleDateString("zh-CN", { month: "2-digit", day: "2-digit" }) : "");
const time = (ms: number | null) => (ms ? new Date(ms).toLocaleString("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }) : "时间未知");
/** Sessions shown under an expanded project; the rest are in 配置管理. */
const SESSIONS_SHOWN = 8;

/** A project Claude Code has worked in: start a new session, or open it to continue a recent one. */
function ProjectRow({ project, sessions, busy, onOpen }: {
  project: ContainerProject; sessions: ClaudeCodeSession[]; busy: boolean; onOpen: (launch: ContainerLaunch) => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <div className="flex items-center gap-1 px-2 py-1.5">
        <button className="flex min-w-0 flex-1 items-center gap-2.5 rounded-lg px-2 py-1.5 text-left hover:bg-card-2" aria-expanded={open} onClick={() => setOpen(!open)}>
          <ChevronRight className={cn("h-3.5 w-3.5 shrink-0 text-muted transition-transform", open && "rotate-90")} />
          <Folder className="h-4 w-4 shrink-0 text-muted" />
          <span className="min-w-0 flex-1">
            <span className="block truncate text-[13.5px] font-medium">{leaf(project.cwd)}</span>
            <span className="block truncate font-mono text-[11px] text-muted" title={`${project.cwd}\n${project.host}`}>{project.host}</span>
          </span>
          <span className="shrink-0 text-[11.5px] text-muted">{project.sessions} 个会话 · {day(project.lastActiveAt)}</span>
        </button>
        <Button size="sm" variant="outline" disabled={busy} onClick={() => onOpen({ cwd: project.cwd })}><Plus />新会话</Button>
      </div>
      {open && (
        <div className="space-y-0.5 pb-2 pl-12 pr-3">
          {sessions.slice(0, SESSIONS_SHOWN).map((s) => (
            <div key={s.id} className="flex items-center gap-2 rounded-lg px-2 py-1 hover:bg-card-2">
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[13px]" title={s.title}>{s.title}</span>
                <span className="block text-[11px] text-muted">{time(s.lastActivityAt)} · {s.turns} 轮</span>
              </span>
              <Button size="sm" variant="ghost" disabled={busy} title="出口检测通过后，在终端里继续这个会话"
                onClick={() => onOpen({ cwd: project.cwd, resume: s.id.split("/")[1] })}><Play />继续</Button>
            </div>
          ))}
          {sessions.length > SESSIONS_SHOWN && <p className="px-2 pt-1 text-[11.5px] text-muted">另有 {sessions.length - SESSIONS_SHOWN} 个较早的会话，可在「配置管理」中查看。</p>}
        </div>
      )}
    </div>
  );
}

/**
 * Picks a folder inside the shared folders for a new project, browsed in place: nothing outside
 * them is visible in the container, so the system's folder picker is not offered.
 */
function NewProject({ container, shared, busy, onOpen, onCancel }: {
  container: string; shared: ContainerProject[]; busy: boolean; onOpen: (launch: ContainerLaunch) => void; onCancel?: () => void;
}) {
  const [at, setAt] = useState<string | null>(null);
  const [children, setChildren] = useState<ContainerSubfolder[] | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    if (!at) return;
    let current = true;
    setChildren(null); setError("");
    api.containerSubfolders(container, at).then((c) => { if (current) setChildren(c); }).catch((e) => { if (current) setError(errorText(e)); });
    return () => { current = false; };
  }, [container, at]);
  const mount = at ? shared.find((m) => at === m.cwd || at.startsWith(`${m.cwd}/`)) : undefined;
  const crumbs = at && mount ? [mount.cwd, ...at.slice(mount.cwd.length).split("/").filter(Boolean).map((_, i, all) => `${mount.cwd}/${all.slice(0, i + 1).join("/")}`)] : [];
  const row = "flex w-full items-center gap-2.5 px-4 py-2 text-left text-[13px] hover:bg-card-2";
  return (
    <div className="space-y-2 rounded-xl bg-card p-3 shadow-card">
      <div className="flex min-h-7 items-center gap-1 text-[12.5px]">
        {at ? <>
          <Button size="iconSm" variant="ghost" aria-label="回到共享文件夹" title="回到共享文件夹" onClick={() => setAt(null)}><ArrowLeft /></Button>
          {crumbs.map((c, i) => (
            <span key={c} className="flex items-center">
              {i > 0 && <ChevronRight className="h-3 w-3 text-muted" />}
              <button className={i === crumbs.length - 1 ? "px-1 font-medium" : "px-1 text-muted hover:text-fg"} onClick={() => setAt(c)}>{leaf(c)}</button>
            </span>
          ))}
        </> : <span className="px-1 font-medium">选择新项目的文件夹</span>}
      </div>
      <div className="list-card max-h-72 overflow-y-auto">
        {!at ? shared.map((m) => (
          <button key={m.cwd} className={row} onClick={() => setAt(m.cwd)}>
            <Folder className="h-4 w-4 shrink-0 text-muted" />
            <span className="min-w-0 flex-1"><span className="block truncate">{leaf(m.cwd)}</span><span className="block truncate font-mono text-[11px] text-muted">{m.host}</span></span>
            <ChevronRight className="h-3.5 w-3.5 text-muted" />
          </button>
        )) : error ? <p className="px-4 py-3 text-[12.5px] text-danger">{error}</p>
          : !children ? <p className="px-4 py-3 text-[12.5px] text-muted">正在读取…</p>
          : children.length ? children.map((c) => (
            <button key={c.cwd} className={row} onClick={() => setAt(c.cwd)}>
              <Folder className="h-4 w-4 shrink-0 text-muted" /><span className="min-w-0 flex-1 truncate">{c.name}</span><ChevronRight className="h-3.5 w-3.5 text-muted" />
            </button>
          )) : <p className="px-4 py-3 text-[12.5px] text-muted">没有子文件夹。</p>}
      </div>
      <div className="flex items-center gap-2">
        <p className="min-w-0 flex-1 text-[11.5px] text-muted">只能选共享文件夹里的目录；要用其他文件夹，请先在「编辑」里添加共享文件夹。</p>
        {onCancel && <Button size="sm" variant="ghost" onClick={onCancel}>取消</Button>}
        <Button size="sm" disabled={!at || busy} onClick={() => at && onOpen({ cwd: at })}><FolderPlus />在这里开始</Button>
      </div>
    </div>
  );
}

/**
 * A container account's own page: its projects, each started or continued with one click. Every
 * start checks the exit first and launches nothing if it fails.
 */
export function ContainerDetail({ container, busy, onBack, onOpen, onEdit }: {
  container: Container; busy?: string; onBack: () => void; onOpen: (launch: ContainerLaunch | null) => void; onEdit: () => void;
}) {
  const [projects, setProjects] = useState<ContainerProjects | null>(null);
  const [sessions, setSessions] = useState<ClaudeCodeSession[]>([]);
  const [error, setError] = useState("");
  const [adding, setAdding] = useState(false);

  useEffect(() => {
    let current = true;
    setError("");
    Promise.all([api.containerProjects(container.id), api.codeSessions(`container:${container.id}`)])
      .then(([p, s]) => { if (current) { setProjects(p); setSessions(s); } })
      .catch((e) => { if (current) setError(errorText(e)); });
    return () => { current = false; };
  }, [container.id]);
  const byFolder = useMemo(() => {
    const map = new Map<string, ClaudeCodeSession[]>();
    for (const s of sessions) if (s.cwd) map.set(s.cwd, [...(map.get(s.cwd) ?? []), s]);
    return map;
  }, [sessions]);
  const blocked = !!container.rebuild;
  const working = !!busy || blocked;
  const empty = projects && !projects.recent.length;

  return (
    <div className="view-in space-y-4">
      <div className="flex items-center gap-2">
        <Button size="sm" variant="ghost" onClick={onBack}><ArrowLeft />容器</Button>
        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[9px] bg-pill text-fg-2"><Box className="h-4 w-4" /></span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5"><span className="truncate text-[14px] font-semibold">{container.name}</span><Tag tone="env">{container.environmentName ?? "环境已删除"}</Tag></div>
          <div className="truncate text-[11.5px] text-muted">{container.mounts.length ? container.mounts.map((m) => `/workspace/${m.name}`).join("  ") : "未共享文件夹"}</div>
        </div>
        {busy ? <span className="flex h-7 items-center gap-1.5 px-2 text-[12.5px] text-muted"><Loader2 className="h-3.5 w-3.5 animate-spin" />{busy}</span>
          : <Button size="sm" variant="outline" disabled={blocked} onClick={() => onOpen(null)}><SquareTerminal />只打开终端</Button>}
        <Button size="sm" variant="ghost" onClick={onEdit}><Pencil />编辑</Button>
      </div>
      {container.rebuild && <div className="rounded-xl bg-warn-soft px-4 py-3 text-[12.5px] text-warn">{container.rebuild}</div>}
      {container.blocked && <div className={cn("rounded-xl px-4 py-3 text-[12.5px]", container.blockedSoft ? "bg-warn-soft text-warn" : "bg-danger-soft text-danger")}>出口保护：{container.blocked}</div>}
      {error && <div className="selectable rounded-xl bg-danger-soft px-4 py-3 text-[13px] text-danger">{error}</div>}
      {!projects && !error && <p className="text-sm text-muted">正在读取项目…</p>}

      {projects && (adding || empty) && (projects.shared.length
        ? <NewProject container={container.id} shared={projects.shared} busy={working} onOpen={(l) => { setAdding(false); onOpen(l); }} onCancel={empty ? undefined : () => setAdding(false)} />
        : <p className="rounded-xl bg-card px-4 py-3 text-[13px] text-muted shadow-card">这个容器还没有共享文件夹。点「编辑」添加项目所在的文件夹后，就能在这里开始。</p>)}

      {projects && !empty && (
        <Group label="项目" action={!adding && <Button size="sm" variant="ghost" onClick={() => setAdding(true)}><FolderPlus />新项目…</Button>}>
          {projects.recent.map((p) => <ProjectRow key={p.cwd} project={p} sessions={byFolder.get(p.cwd) ?? []} busy={working} onOpen={onOpen} />)}
        </Group>
      )}
      <p className="px-1 text-[11.5px] leading-relaxed text-muted">
        开始或继续会话时，Hydra 先经本机中继检测出口，再启动网关和容器，全部通过后才在终端里启动 Claude Code；检测不通过则不启动并显示原因。
      </p>
    </div>
  );
}

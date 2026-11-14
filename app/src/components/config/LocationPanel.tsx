import { AppWindow, Box, ChevronDown, FolderOpen, Laptop, Search, X } from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { Tag } from "@/components/Tag";
import { Button } from "@/components/ui/button";
import type { ClaudeFolder, ClaudeFolderSummary } from "@/lib/api";
import { cn } from "@/lib/utils";
import { kindLabel } from "./shared";

const icons = { local: Laptop, desktop: AppWindow, container: Box } as const;
const groupLabel = { local: "本机", desktop: "桌面版账号", container: "容器账号" } as const;
/** Past this many locations the picker gets a search field. */
const SEARCH_FROM = 7;
const revealLabel = navigator.userAgent.includes("Mac") ? "在 Finder 中显示" : "在资源管理器中显示";

/** Height-animated region: rows collapse from 1fr to 0fr; hidden content is not focusable. */
export function Collapse({ open, children }: { open: boolean; children: ReactNode }) {
  return (
    <div aria-hidden={!open} className={cn(
      "grid transition-[grid-template-rows,opacity,visibility] duration-300 ease-soft motion-reduce:transition-none",
      open ? "visible grid-rows-[1fr] opacity-100" : "invisible grid-rows-[0fr] opacity-0",
    )}>
      <div className="min-h-0 overflow-hidden">{children}</div>
    </div>
  );
}

function KindIcon({ kind, className }: { kind: ClaudeFolder["kind"]; className?: string }) {
  const Icon = icons[kind];
  return <span className={cn("flex shrink-0 items-center justify-center rounded-lg bg-pill text-fg-2", className)}><Icon className="h-4 w-4" /></span>;
}

function activity(f: ClaudeFolderSummary) {
  if (!f.exists) return "目录还不存在";
  if (!f.sessions) return "还没有会话";
  const when = f.lastActiveAt ? ` · 最近 ${new Date(f.lastActiveAt).toLocaleDateString("zh-CN", { month: "2-digit", day: "2-digit" })}` : "";
  return `${f.sessions} 个会话${when}`;
}

/** Every location as a card, grouped by kind and searchable once there are many. */
export function LocationPicker({ folders, current, onPick, onCancel }: {
  folders: ClaudeFolderSummary[]; current: string; onPick: (id: string) => void; onCancel?: () => void;
}) {
  const [query, setQuery] = useState("");
  const needle = query.trim().toLocaleLowerCase();
  const groups = useMemo(() => (["local", "desktop", "container"] as const).map((kind) => [kind, folders.filter((f) =>
    f.kind === kind && (!needle || `${f.name} ${f.path}`.toLocaleLowerCase().includes(needle)))] as const).filter(([, l]) => l.length), [folders, needle]);
  useEffect(() => {
    if (!onCancel) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape" && !document.querySelector("[data-dialog-layer]")) onCancel(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);
  return (
    <div className="space-y-4 p-4">
      <div className="flex flex-wrap items-center gap-3">
        <div className="min-w-0 flex-1">
          <h2 className="text-[14px] font-semibold">选择位置</h2>
          <p className="text-[12.5px] text-muted">每个位置是一个 Claude Code 数据目录：会话、记忆、技能和 MCP 都在里面。</p>
        </div>
        {folders.length >= SEARCH_FROM && (
          <label className="relative w-52">
            <Search className="absolute left-2.5 top-2 h-4 w-4 text-muted" />
            <input className="field pl-8" aria-label="搜索位置" placeholder="搜索名称或路径" value={query} onChange={(e) => setQuery(e.target.value)} />
          </label>
        )}
        {onCancel && <Button size="iconSm" variant="ghost" aria-label="收起" title="收起（Esc）" onClick={onCancel}><X /></Button>}
      </div>
      {!groups.length && <p className="py-4 text-center text-sm text-muted">没有匹配的位置。</p>}
      {groups.map(([kind, list]) => (
        <section key={kind}>
          <div className="group-label">{groupLabel[kind]}（{list.length}）</div>
          <div className="grid grid-cols-[repeat(auto-fill,minmax(220px,1fr))] gap-2">
            {list.map((f) => (
              <button key={f.id} onClick={() => onPick(f.id)} aria-current={f.id === current || undefined}
                className={cn("flex min-w-0 items-center gap-2.5 rounded-xl bg-card-2 px-3 py-2.5 text-left hover:bg-pill",
                  f.id === current && "shadow-[inset_0_0_0_1.5px_rgb(var(--accent))]")}>
                <KindIcon kind={f.kind} className="h-8 w-8" />
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-1.5"><span className="truncate text-[13px] font-medium">{f.name}</span>
                    {f.sharedWithLocal && <Tag title="没有独立的 Claude Code 目录，与本机共用同一个目录">共用本机</Tag>}</span>
                  <span className="block truncate text-[12px] text-muted">{activity(f)}</span>
                </span>
              </button>
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}

/** The chosen location once the picker has folded away: name, path, open in Finder, switch. */
export function LocationHeader({ folder, onSwitch, onReveal }: { folder: ClaudeFolder; onSwitch: () => void; onReveal: () => void }) {
  return (
    <div className="flex items-center gap-2.5 px-3 pb-2 pt-3">
      <KindIcon kind={folder.kind} className="h-8 w-8" />
      <button className="min-w-0 flex-1 text-left" onClick={onSwitch} title="切换位置">
        <span className="flex items-center gap-1.5"><span className="truncate text-[13.5px] font-semibold">{folder.name}</span><Tag>{kindLabel[folder.kind]}</Tag>
          {folder.sharedWithLocal && <Tag title="没有独立的 Claude Code 目录，与本机共用同一个目录">共用本机</Tag>}</span>
        <span className="block truncate font-mono text-[11.5px] text-muted" title={folder.path}>{folder.path}</span>
      </button>
      <Button size="iconSm" variant="ghost" aria-label={revealLabel} title={revealLabel} onClick={onReveal}><FolderOpen /></Button>
      <Button size="sm" variant="outline" onClick={onSwitch}>切换位置<ChevronDown /></Button>
    </div>
  );
}

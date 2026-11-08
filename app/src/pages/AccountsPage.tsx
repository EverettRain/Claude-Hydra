import { ProfileAvatar } from "@/components/ProfileAvatar";
import {
  FolderOpen,
  Link2,
  Loader2,
  MoreHorizontal,
  Pencil,
  Play,
  Plus,
  Power,
  RefreshCw,
  RotateCw,
  Trash2,
} from "lucide-react";
import { useState } from "react";
import { EditAccountDialog } from "@/components/EditAccountDialog";
import { CreateAccountPanel } from "@/components/CreateAccountPanel";
import { Menu } from "@/components/Menu";
import { Page, PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type Profile } from "@/lib/api";
import { cn, relativeTime } from "@/lib/utils";

// The browser sign-in callback router is Windows-only (see login.rs).
const IS_WINDOWS = navigator.userAgent.includes("Windows");
const IS_MAC = navigator.userAgent.includes("Mac");

interface Props {
  profiles: Profile[] | null;
  loadError: string | null;
  refresh: () => Promise<void>;
}

export function AccountsPage({ profiles, loadError, refresh }: Props) {
  const toast = useToast();
  const [busy, setBusy] = useState<Record<string, string>>({});
  const [editing, setEditing] = useState<Profile | null>(null);
  const [deleting, setDeleting] = useState<Profile | null>(null);
  const [adding, setAdding] = useState(false);

  const run = async (p: Profile, label: string, action: () => Promise<unknown>, done?: string) => {
    setBusy((b) => ({ ...b, [p.id]: label }));
    try {
      await action();
      if (done) toast(done);
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy((b) => {
        const { [p.id]: _, ...rest } = b;
        return rest;
      });
      setTimeout(refresh, 600);
      setTimeout(refresh, 2500);
    }
  };

  const launch = (p: Profile) =>
    run(p, "启动中", async () => {
      const r = await api.launch(p.id);
      if (r === "started") toast(`正在启动 ${p.name}`);
    });

  const addAccount = () => setAdding(true);

  return (
    <Page>
      <PageHeader
        title="账号"
        subtitle="每个账号使用独立的数据目录、登录态与环境配置，可同时运行。"
        actions={
          <>
            <Button variant="ghost" size="iconSm" aria-label="刷新" title="刷新" onClick={() => refresh()}>
              <RefreshCw />
            </Button>
            <Button size="sm" onClick={addAccount} disabled={adding}>
              <Plus />
              添加账号
            </Button>
          </>
        }
      />

      <div className="space-y-4 px-5">
        {loadError && <div className="selectable rounded-xl bg-danger-soft px-4 py-3 text-[13px] text-danger">{loadError}</div>}

        {profiles && profiles.length > 0 && (
          <div className="list-card">
            {profiles.map((p) => (
              <AccountRow
                key={p.id}
                p={p}
                busy={busy[p.id]}
                onLaunch={() => launch(p)}
                onClose={() => run(p, "关闭中", () => api.close(p.id), `已关闭 ${p.name}`)}
                onRestart={() => run(p, "重启中", () => api.restart(p.id), `已重启 ${p.name}`)}
                onEdit={() => setEditing(p)}
                onLogin={() => run(p, "准备登录", () => api.prepareLogin(p.id), `已选择 ${p.name}，请在 10 分钟内完成浏览器登录。`)}
                onFolder={() => api.openFolder(p.id).catch((e) => toast(errorText(e), "error"))}
                onShortcut={() =>
                  run(p, "创建中", async () => {
                    const out = await api.createShortcut(p.id);
                    toast(out.trim() || "已创建桌面快捷方式。");
                  })
                }
                onDelete={() => setDeleting(p)}
              />
            ))}
            {profiles.length === 1 && (
              <button onClick={addAccount} className="flex w-full items-center justify-center gap-1.5 py-3 text-[12.5px] text-muted hover:bg-card-2 hover:text-fg active:!scale-100">
                <Plus className="h-3.5 w-3.5" />
                添加第二个账号，新建独立数据目录并打开登录界面
              </button>
            )}
          </div>
        )}

        {profiles === null && !loadError && (
          <div className="list-card">
            {[0, 1].map((i) => (
              <div key={i} className="flex items-center gap-3 px-4 py-3.5">
                <div className="h-8 w-8 animate-pulse rounded-[9px] bg-pill" />
                <div className="space-y-1.5"><div className="h-3 w-28 animate-pulse rounded bg-pill" /><div className="h-2.5 w-44 animate-pulse rounded bg-pill" /></div>
              </div>
            ))}
          </div>
        )}
      </div>

      <EditAccountDialog profile={editing} onClose={() => setEditing(null)} onSaved={refresh} />
      <CreateAccountPanel open={adding} onClose={() => setAdding(false)} onCreated={async (p) => { setAdding(false); toast(`已创建 ${p.name}，请在打开的 Claude 窗口中登录。`); await refresh(); }} />

      <Dialog
        open={!!deleting}
        onClose={() => setDeleting(null)}
        title={`删除 ${deleting?.name ?? ""}？`}
        description="这会删除该账号的整个数据目录，并在本机退出该账号的登录。此操作无法撤销。"
        footer={
          <>
            <Button variant="outline" onClick={() => setDeleting(null)}>
              取消
            </Button>
            <Button
              variant="destructive"
              onClick={() => {
                const p = deleting!;
                setDeleting(null);
                run(p, "删除中", () => api.remove(p.id), `已删除 ${p.name}`);
              }}
            >
              删除
            </Button>
          </>
        }
      >
        <p className="selectable break-all rounded-lg bg-pill px-3 py-2 font-mono text-[11.5px] text-muted">{deleting?.path}</p>
      </Dialog>
    </Page>
  );
}

interface RowProps {
  p: Profile;
  busy?: string;
  onLaunch: () => void;
  onClose: () => void;
  onRestart: () => void;
  onEdit: () => void;
  onLogin: () => void;
  onFolder: () => void;
  onShortcut: () => void;
  onDelete: () => void;
}

function AccountRow({ p, busy, onLaunch, onClose, onRestart, onEdit, onLogin, onFolder, onShortcut, onDelete }: RowProps) {
  const isRunning = !!p.pid;
  return (
    <div onDoubleClick={onLaunch} className="group flex items-center gap-3 px-4 py-3 hover:bg-card-2">
      <ProfileAvatar badge={p.badge} isDefault={p.isDefault} color={p.color} className="h-8 w-8 shrink-0" />

      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="truncate text-[13.5px] font-medium">{p.name}</span>
          {p.isDefault && <Tag>默认</Tag>}
          {p.environment && <Tag tone="env">{p.environment}</Tag>}
          {p.keepRunning && <Tag>保持运行</Tag>}
          {p.signedIn === false && <Tag tone="warn">未登录</Tag>}
        </div>
        <div className="mt-0.5 flex items-center gap-1.5 text-[11.5px] text-muted">
          <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", isRunning ? "bg-ok shadow-[0_0_0_3px_rgb(var(--ok-soft))]" : "bg-faint")} />
          <span className="shrink-0">{isRunning ? `运行中 · PID ${p.pid}` : `上次使用 ${relativeTime(p.lastUsed)}`}</span>
          <span className="shrink-0 text-faint">·</span>
          <span className="min-w-0 truncate" dir="rtl" title={p.path}>
            <bdi dir="ltr">{p.path}</bdi>
          </span>
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-1" onDoubleClick={(event) => event.stopPropagation()}>
        {busy ? (
          <span className="flex h-7 items-center gap-1.5 px-2 text-[12.5px] text-muted">
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
            {busy}
          </span>
        ) : (
          <>
            <Button variant="outline" size="sm" className="min-w-[72px]" onClick={isRunning ? onClose : onLaunch}>
              {isRunning ? <><Power />关闭</> : <><Play className="!h-3 !w-3" />启动</>}
            </Button>
          </>
        )}
        <Menu
          trigger={(toggle) => (
            <Button variant="ghost" size="iconSm" aria-label="更多" onClick={toggle} disabled={!!busy}>
              <MoreHorizontal />
            </Button>
          )}
          items={[
            { label: "编辑…", icon: Pencil, onSelect: onEdit },
            ...(IS_WINDOWS || (IS_MAC && !p.isDefault) ? [{ label: "准备浏览器登录", icon: Link2, onSelect: onLogin }] : []),
            { label: "重启", icon: RotateCw, onSelect: onRestart, disabled: !isRunning },
            { label: "打开数据目录", icon: FolderOpen, onSelect: onFolder },
            { label: "创建桌面快捷方式", icon: Link2, onSelect: onShortcut },
            ...(p.isDefault ? [] : (["sep", { label: "删除…", icon: Trash2, onSelect: onDelete, danger: true, disabled: isRunning }] as const)),
          ]}
        />
      </div>
    </div>
  );
}

function Tag({ children, tone }: { children: React.ReactNode; tone?: "warn" | "env" }) {
  return (
    <span
      title={tone === "env" ? "环境配置" : undefined}
      className={cn(
        "inline-flex h-[18px] max-w-[160px] shrink-0 items-center truncate rounded-full px-1.5 text-[10.5px] font-medium",
        tone === "warn" ? "bg-warn-soft text-warn" : tone === "env" ? "bg-brand-soft text-brand" : "bg-pill text-fg-2",
      )}
    >
      {children}
    </span>
  );
}

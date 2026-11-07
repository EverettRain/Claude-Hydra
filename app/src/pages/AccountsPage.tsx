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
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { useState } from "react";
import { EditAccountDialog } from "@/components/EditAccountDialog";
import { Menu } from "@/components/Menu";
import { PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type Profile } from "@/lib/api";
import { PALETTE, badgeLetter, cn, relativeTime } from "@/lib/utils";

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

  const addAccount = async () => {
    setAdding(true);
    try {
      const p = await api.addAccount();
      toast(`已创建 ${p.name}，请在打开的 Claude 窗口中登录。`);
      await refresh();
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setAdding(false);
    }
  };

  const running = profiles?.filter((p) => p.pid).length ?? 0;

  return (
    <div className="pb-8">
      <PageHeader
        title="账号"
        subtitle={profiles ? `${profiles.length} 个账号 · ${running} 个运行中` : "读取中…"}
        actions={
          <>
            <Button variant="outline" size="icon" aria-label="刷新" title="刷新" onClick={() => refresh()}>
              <RefreshCw className="h-4 w-4" />
            </Button>
            <Button onClick={addAccount} disabled={adding}>
              {adding ? <Loader2 className="h-4 w-4 animate-spin" /> : <Plus className="h-4 w-4" />}
              添加账号
            </Button>
          </>
        }
      />

      <div className="space-y-3 px-6">
        {loadError && (
          <div className="selectable rounded-lg border border-red-500/40 bg-red-500/5 p-4 text-sm text-red-600 dark:text-red-400">
            {loadError}
          </div>
        )}

        {profiles?.map((p) => (
          <AccountCard
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

        {profiles && profiles.length === 1 && (
          <button
            onClick={addAccount}
            className="flex w-full flex-col items-center gap-1 rounded-lg border border-dashed border-border px-4 py-8 text-muted-foreground hover:border-blue-500/60 hover:text-foreground"
          >
            <Plus className="h-5 w-5" />
            <span className="text-sm font-medium">添加第二个账号</span>
            <span className="text-xs">新建一个独立的数据目录，并打开 Claude 的登录界面</span>
          </button>
        )}
      </div>

      <EditAccountDialog profile={editing} onClose={() => setEditing(null)} onSaved={refresh} />

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
        <p className="selectable break-all rounded-md bg-muted px-3 py-2 font-mono text-xs text-muted-foreground">
          {deleting?.path}
        </p>
      </Dialog>
    </div>
  );
}

interface CardProps {
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

function AccountCard({ p, busy, onLaunch, onClose, onRestart, onEdit, onLogin, onFolder, onShortcut, onDelete }: CardProps) {
  const isRunning = !!p.pid;
  return (
    <div
      onDoubleClick={onLaunch}
      className={cn(
        "group flex items-center gap-4 rounded-lg border bg-card p-4 transition-colors",
        isRunning ? "border-emerald-500/30" : "border-border hover:border-gray-300 dark:hover:border-gray-700",
      )}
    >
      <div className="relative shrink-0">
        <div
          className="flex h-11 w-11 items-center justify-center rounded-xl text-lg font-semibold text-white"
          style={{ background: PALETTE[p.color % PALETTE.length] }}
        >
          {badgeLetter(p.name)}
        </div>
        {isRunning && (
          <span className="absolute -bottom-0.5 -right-0.5 h-3.5 w-3.5 rounded-full border-2 border-card bg-emerald-500" />
        )}
      </div>

      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-[15px] font-semibold">{p.name}</span>
          {p.isDefault && <Tag>默认</Tag>}
          {p.keepRunning && (
            <Tag className="text-blue-600 dark:text-blue-400">
              <ShieldCheck className="h-3 w-3" />
              保持运行
            </Tag>
          )}
          {p.signedIn === false && <Tag className="text-amber-600 dark:text-amber-400">未登录</Tag>}
        </div>
        <div className="mt-0.5 flex items-center gap-1.5 text-xs text-muted-foreground">
          <span className="shrink-0">{isRunning ? `运行中 · PID ${p.pid}` : `上次使用 ${relativeTime(p.lastUsed)}`}</span>
          <span className="shrink-0">·</span>
          <span className="min-w-0 truncate" dir="rtl" title={p.path}>
            <bdi dir="ltr">{p.path}</bdi>
          </span>
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-1.5">
        {busy ? (
          <span className="flex h-9 items-center gap-2 px-3 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            {busy}
          </span>
        ) : (
          <>
            {isRunning && (
              <Button variant="ghost" size="icon" title="关闭" aria-label="关闭" onClick={onClose}>
                <Power className="h-4 w-4" />
              </Button>
            )}
            <Button variant={isRunning ? "outline" : "default"} size="sm" className="h-9 min-w-[84px]" onClick={onLaunch}>
              <Play className="h-3.5 w-3.5" />
              {isRunning ? "切换到" : "启动"}
            </Button>
          </>
        )}
        <Menu
          trigger={(toggle) => (
            <Button variant="ghost" size="icon" aria-label="更多" onClick={toggle} disabled={!!busy}>
              <MoreHorizontal className="h-4 w-4" />
            </Button>
          )}
          items={[
            { label: "编辑…", icon: Pencil, onSelect: onEdit },
            { label: "准备浏览器登录", icon: Link2, onSelect: onLogin },
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

function Tag({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-1 rounded-md bg-muted px-1.5 py-0.5 text-[11px] font-medium leading-none text-muted-foreground",
        className,
      )}
    >
      {children}
    </span>
  );
}

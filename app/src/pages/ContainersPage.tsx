import { Box, Hammer, Link2, Loader2, MoreHorizontal, Pencil, Plus, Power, RefreshCw, ShieldAlert, SquareTerminal, Trash2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { ContainerDialog } from "@/components/ContainerDialog";
import { ContainerDetail } from "@/components/ContainerDetail";
import { Menu } from "@/components/Menu";
import { Page, PageHeader } from "@/components/PageHeader";
import { Tag } from "@/components/Tag";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import { Switch } from "@/components/ui/switch";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type Container, type ContainerInput, type ContainerLaunch, type ContainerRuntime } from "@/lib/api";
import { cn } from "@/lib/utils";

type Editing = { id: string | null; initial?: ContainerInput } | null;

const STATE: Record<Container["state"], string> = { running: "运行中", stopped: "已停止", absent: "首次打开时创建", unknown: "OrbStack 未连接" };

const toInput = (c: Container): ContainerInput => ({ name: c.name, environment: c.environment, image: c.image, mounts: c.mounts, sshAgent: c.sshAgent, claudeDir: c.claudeDir });

export function ContainersPage({ onOpenSettings }: { onOpenSettings: () => void }) {
  const toast = useToast();
  const [items, setItems] = useState<Container[] | null>(null);
  const [runtime, setRuntime] = useState<ContainerRuntime | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState<Record<string, string>>({});
  const [editing, setEditing] = useState<Editing>(null);
  const [rebuilding, setRebuilding] = useState<Container | null>(null);
  const [keepChanges, setKeepChanges] = useState(true);
  const [deleting, setDeleting] = useState<Container | null>(null);
  // The container whose page is open, by id so it follows reloads of the list.
  const [viewing, setViewing] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [list, rt] = await Promise.all([api.listContainers(), api.containerRuntime()]);
      setItems(list);
      setRuntime(rt);
      setError("");
    } catch (e) {
      setError(errorText(e));
    }
  }, []);
  useEffect(() => { void load(); }, [load]);

  const run = async (c: Container, label: string, fn: () => Promise<unknown>, done?: string) => {
    setBusy((b) => ({ ...b, [c.id]: label }));
    try {
      await fn();
      if (done) toast(done);
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy((b) => { const { [c.id]: _, ...rest } = b; return rest; });
      void load();
    }
  };

  const openTerminal = (c: Container, launch: ContainerLaunch | null) => run(c, "打开中", () => api.openContainer(c.id, launch ?? undefined),
    launch ? `已在终端中打开 ${c.name}，出口检测通过后启动 Claude Code` : `已在终端中打开 ${c.name}`);

  const editDialog = (
    <ContainerDialog
      open={!!editing}
      id={editing?.id ?? null}
      initial={editing?.initial}
      onClose={() => setEditing(null)}
      onSaved={async () => {
        const created = !editing?.id;
        setEditing(null);
        toast(created ? "已创建容器账号，点“打开”进入。" : "已保存。环境的修改在新开的终端中生效。");
        await load();
      }}
    />
  );
  const shown = viewing ? items?.find((c) => c.id === viewing) : undefined;
  if (shown) {
    return (
      <Page>
        <div className="px-5 pt-1">
          <ContainerDetail container={shown} busy={busy[shown.id]} onBack={() => setViewing(null)}
            onOpen={(launch) => void openTerminal(shown, launch)} onEdit={() => setEditing({ id: shown.id, initial: toInput(shown) })} />
        </div>
        {editDialog}
      </Page>
    );
  }

  return (
    <Page>
      <PageHeader
        title="容器"
        subtitle="在 OrbStack 容器中运行 Claude Code：每个容器账号有独立的登录数据、网关与代理节点，网络只经节点，无直连回退。"
        actions={
          <>
            <Button variant="ghost" size="iconSm" aria-label="刷新" title="刷新" onClick={() => load()}>
              <RefreshCw />
            </Button>
            <Button size="sm" disabled={!runtime?.supported} onClick={() => setEditing({ id: null })}>
              <Plus />
              新建容器账号
            </Button>
          </>
        }
      />
      <div className="space-y-4 px-5">
        {error && <div className="selectable rounded-xl bg-danger-soft px-4 py-3 text-[13px] text-danger">{error}</div>}

        {runtime && !runtime.version && (
          <div className="flex items-start gap-3 rounded-xl bg-warn-soft px-4 py-3 text-[12.5px] text-warn">
            <span className="selectable min-w-0 flex-1 whitespace-pre-wrap">{runtime.error}</span>
            {runtime.supported && <Button variant="outline" size="sm" onClick={onOpenSettings}>OrbStack 设置</Button>}
          </div>
        )}

        {items && items.length === 0 && runtime?.supported && (
          <button onClick={() => setEditing({ id: null })} className="flex w-full flex-col items-center gap-1.5 rounded-xl border border-dashed border-line px-4 py-10 text-muted hover:border-faint hover:text-fg active:!scale-100">
            <Box className="h-5 w-5" />
            <span className="text-[13px] font-medium">还没有容器账号</span>
            <span className="text-[12px]">新建一个：选择带代理节点的环境和要共享的项目文件夹，打开后选择项目，直接进入 Claude Code</span>
          </button>
        )}

        {items && items.length > 0 && (
          <div className="list-card">
            {items.map((c) => {
              const isRunning = c.state === "running";
              return (
                <div key={c.id} onClick={() => setViewing(c.id)} className="group flex cursor-pointer items-center gap-3 px-4 py-3 hover:bg-card-2">
                  <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[9px] bg-pill text-fg-2">
                    <Box className="h-4 w-4" />
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5">
                      <span className="truncate text-[13.5px] font-medium">{c.name}</span>
                      <Tag tone="env">{c.environmentName ?? "环境已删除"}</Tag>
                      {c.rebuild && <Tag tone="warn" title={c.rebuild}>需要重建</Tag>}
                      {!c.rebuild && c.imageUpdate && <Tag title="镜像已更改，重建容器后使用新镜像">镜像有更新</Tag>}
                      {c.blocked && (c.blockedSoft
                        ? <Tag tone="warn" title={c.blocked}>核验中断网</Tag>
                        : <Tag tone="danger" title={c.blocked}>已断网</Tag>)}
                    </div>
                    {c.blocked && <div className={cn("mt-0.5 truncate text-[11.5px]", c.blockedSoft ? "text-warn" : "text-danger")} title={c.blocked}>{c.blockedSoft ? "出口保护（自动重试中）：" : "出口保护："}{c.blocked}</div>}
                    {c.rebuild && <div className="mt-0.5 truncate text-[11.5px] text-warn">{c.rebuild}</div>}
                    <div className="mt-0.5 flex items-center gap-1.5 text-[11.5px] text-muted">
                      <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", isRunning ? "bg-ok shadow-[0_0_0_3px_rgb(var(--ok-soft))]" : "bg-faint")} />
                      <span className="shrink-0">{STATE[c.state]}</span>
                      <span className="shrink-0 text-faint">·</span>
                      <span className="min-w-0 truncate" title={c.mounts.map((m) => `${m.source} → /workspace/${m.name}`).join("\n")}>
                        {c.mounts.length ? c.mounts.map((m) => `/workspace/${m.name}`).join("  ") : "未共享文件夹"}
                      </span>
                    </div>
                  </div>
                  <div className="flex shrink-0 items-center gap-1" onClick={(e) => e.stopPropagation()}>
                    {busy[c.id] ? (
                      <span className="flex h-7 items-center gap-1.5 px-2 text-[12.5px] text-muted">
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                        {busy[c.id]}
                      </span>
                    ) : (
                      <Button variant="outline" size="sm" className="min-w-[72px]" onClick={() => setViewing(c.id)}>
                        <SquareTerminal />
                        打开
                      </Button>
                    )}
                    <Menu
                      trigger={(toggle) => (
                        <Button variant="ghost" size="iconSm" aria-label="更多" onClick={toggle} disabled={!!busy[c.id]}>
                          <MoreHorizontal />
                        </Button>
                      )}
                      items={[
                        { label: "编辑…", icon: Pencil, onSelect: () => setEditing({ id: c.id, initial: toInput(c) }) },
                        ...(c.blocked ? [{ label: "恢复网络（重新检测出口）", icon: ShieldAlert, onSelect: () => run(c, "检测出口", async () => toast(`已恢复 ${c.name} 的网络，当前出口 ${await api.resumeAccount(c.id)}`)) }] : []),
                        { label: "停止", icon: Power, disabled: !isRunning, onSelect: () => run(c, "停止中", () => api.stopContainer(c.id), `已停止 ${c.name}`) },
                        { label: "重建…", icon: Hammer, disabled: c.state === "absent" || c.state === "unknown", onSelect: () => { setKeepChanges(true); setRebuilding(c); } },
                        { label: "创建桌面快捷方式", icon: Link2, onSelect: () => run(c, "创建中", async () => toast(await api.createContainerShortcut(c.id))) },
                        "sep",
                        { label: "删除…", icon: Trash2, danger: true, onSelect: () => setDeleting(c) },
                      ]}
                    />
                  </div>
                </div>
              );
            })}
          </div>
        )}

        {runtime?.supported && (
          <p className="px-1 text-[11.5px] leading-relaxed text-muted">
            打开时选择项目文件夹，Hydra 会先经本机中继检测出口，再启动网关和容器并核对两者处于同一网络空间，全部通过后在该文件夹中启动 Claude Code。
            Claude Code 登录时显示的链接会在 Mac 的浏览器中打开，请让浏览器使用与该账号相同的出口。账号运行期间请保持 Claude-Hydra 在托盘中运行，退出后容器会断网。
          </p>
        )}
      </div>

      {editDialog}

      <Dialog
        open={!!rebuilding}
        onClose={() => setRebuilding(null)}
        title={`重建 ${rebuilding?.name ?? ""}？`}
        description="按当前设置重新创建容器和网关：容器中打开的终端会结束。Claude Code 的数据文件夹、/root 数据卷与共享文件夹不受影响。"
        footer={
          <>
            <Button variant="outline" onClick={() => setRebuilding(null)}>取消</Button>
            <Button
              onClick={() => {
                const c = rebuilding;
                setRebuilding(null);
                if (c) void run(c, "重建中", () => api.rebuildContainer(c.id, keepChanges), `已重建 ${c.name}，下次打开时按新设置创建。`);
              }}
            >
              重建
            </Button>
          </>
        }
      >
        <div className="flex items-start gap-3">
          <Switch checked={keepChanges} onCheckedChange={setKeepChanges} label="保留容器内的修改" />
          <div className="text-[12.5px] leading-5">
            <div className="font-medium">保留容器内的修改</div>
            <div className="text-muted">
              {keepChanges
                ? "先把容器保存为快照（包括安装的软件），新容器从快照创建。镜像的更新不会应用。"
                : "从镜像重新创建，容器内安装的软件会丢失（/root 与 Claude 数据保留）；会使用新的镜像。"}
            </div>
          </div>
        </div>
      </Dialog>

      <Dialog
        open={!!deleting}
        onClose={() => setDeleting(null)}
        title={`删除 ${deleting?.name ?? ""}？`}
        description={deleting?.claudeDir
          ? "会删除它的容器、网关、快照与 /root 数据卷。你选择的 Claude 数据文件夹和共享文件夹不受影响。"
          : "会删除它的容器、网关、快照、/root 数据卷，以及 Hydra 数据目录中保存 Claude Code 登录与设置的文件夹。共享文件夹不受影响。"}
        footer={
          <>
            <Button variant="outline" onClick={() => setDeleting(null)}>取消</Button>
            <Button
              variant="destructive"
              onClick={() => {
                const c = deleting;
                setDeleting(null);
                if (c) void run(c, "删除中", async () => {
                  const note = await api.deleteContainer(c.id);
                  toast(note ?? `已删除 ${c.name}`, note ? "error" : undefined);
                });
              }}
            >
              删除
            </Button>
          </>
        }
      />
    </Page>
  );
}

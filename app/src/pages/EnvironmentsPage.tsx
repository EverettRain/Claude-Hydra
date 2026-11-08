import { Copy, Globe2, MoreHorizontal, Pencil, Plus, Trash2, WifiOff } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { EnvironmentDialog } from "@/components/EnvironmentDialog";
import { Menu } from "@/components/Menu";
import { Page, PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type Environment, type EnvironmentInput } from "@/lib/api";
import { environmentSummary, toInput } from "@/lib/environment";

type Editing = { id: string | null; initial?: EnvironmentInput; detectedTimezone?: string | null } | null;

export function EnvironmentsPage({ onChanged }: { onChanged: () => void }) {
  const toast = useToast();
  const [envs, setEnvs] = useState<Environment[] | null>(null);
  const [error, setError] = useState("");
  const [editing, setEditing] = useState<Editing>(null);
  const [deleting, setDeleting] = useState<Environment | null>(null);

  const load = useCallback(async () => {
    try { setEnvs(await api.listEnvironments()); setError(""); }
    catch (e) { setError(errorText(e)); }
  }, []);
  useEffect(() => { void load(); }, [load]);

  const remove = async (e: Environment) => {
    setDeleting(null);
    try {
      await api.deleteEnvironment(e.id);
      toast(`已删除 ${e.name}`);
      await load();
      onChanged();
    } catch (err) {
      toast(errorText(err), "error");
    }
  };

  return (
    <Page>
      <PageHeader
        title="环境配置"
        subtitle="把代理节点、时区、窗口语言与启动参数保存为环境，编辑账号时选择即可；修改一处，所有使用它的账号同步生效。"
        actions={
          <Button size="sm" onClick={() => setEditing({ id: null })}>
            <Plus />
            新建环境
          </Button>
        }
      />
      <div className="space-y-4 px-5">
        {error && <div className="selectable rounded-xl bg-danger-soft px-4 py-3 text-[13px] text-danger">{error}</div>}

        {envs && envs.length === 0 && (
          <button onClick={() => setEditing({ id: null })} className="flex w-full flex-col items-center gap-1.5 rounded-xl border border-dashed border-line px-4 py-10 text-muted hover:border-faint hover:text-fg active:!scale-100">
            <Globe2 className="h-5 w-5" />
            <span className="text-[13px] font-medium">还没有环境配置</span>
            <span className="text-[12px]">新建一个，例如“东京住宅 IP”：填入代理节点，时区自动跟随出口</span>
          </button>
        )}

        {envs && envs.length > 0 && (
          <div className="list-card">
            {envs.map((e) => (
              <div key={e.id} onDoubleClick={() => setEditing({ id: e.id, initial: toInput(e), detectedTimezone: e.detectedTimezone })} className="group flex items-center gap-3 px-4 py-3 hover:bg-card-2">
                <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[9px] bg-pill text-fg-2">
                  {e.node ? <Globe2 className="h-4 w-4" /> : <WifiOff className="h-4 w-4" />}
                </span>
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[13.5px] font-medium">{e.name}</div>
                  <div className="truncate text-[11.5px] text-muted">{environmentSummary(e)}</div>
                </div>
                <span className="hidden shrink-0 text-[11.5px] text-muted min-[820px]:inline" title={e.usedBy.join("、")}>
                  {e.usedBy.length ? `${e.usedBy.length} 个账号使用` : "未使用"}
                </span>
                <Button variant="outline" size="sm" onClick={() => setEditing({ id: e.id, initial: toInput(e), detectedTimezone: e.detectedTimezone })}>编辑</Button>
                <Menu
                  trigger={(toggle) => (
                    <Button variant="ghost" size="iconSm" aria-label="更多" onClick={toggle}>
                      <MoreHorizontal />
                    </Button>
                  )}
                  items={[
                    { label: "编辑…", icon: Pencil, onSelect: () => setEditing({ id: e.id, initial: toInput(e), detectedTimezone: e.detectedTimezone }) },
                    { label: "复制为新环境", icon: Copy, onSelect: () => setEditing({ id: null, initial: { ...toInput(e), name: `${e.name} 副本` } }) },
                    "sep",
                    { label: "删除…", icon: Trash2, danger: true, onSelect: () => setDeleting(e) },
                  ]}
                />
              </div>
            ))}
          </div>
        )}
      </div>

      <EnvironmentDialog
        open={!!editing}
        id={editing?.id ?? null}
        initial={editing?.initial}
        detectedTimezone={editing?.detectedTimezone}
        onClose={() => setEditing(null)}
        onSaved={async () => {
          const created = !editing?.id;
          setEditing(null);
          toast(created ? "已创建环境配置。" : "已保存。正在运行的账号：节点的新连接立即生效，时区与启动参数需重启账号。");
          await load();
          onChanged();
        }}
      />

      <Dialog
        open={!!deleting}
        onClose={() => setDeleting(null)}
        title={`删除 ${deleting?.name ?? ""}？`}
        description={deleting?.usedBy.length ? `正在使用它的账号（${deleting.usedBy.join("、")}）将改为不使用环境配置，即直连网络、系统时区。` : "此环境配置未被任何账号使用。"}
        footer={
          <>
            <Button variant="outline" onClick={() => setDeleting(null)}>取消</Button>
            <Button variant="destructive" onClick={() => deleting && remove(deleting)}>删除</Button>
          </>
        }
      />
    </Page>
  );
}
